//! Per-keystroke latency measurement exposed via `:perf`.
//!
//! Records a four-phase timing breakdown (context build, engine process,
//! effects dispatch, UI update) per keystroke and computes percentile
//! statistics over a rolling window.

/// Newtype preventing confusion between microsecond values and other `u64`s.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
pub(crate) struct Microseconds(pub(crate) u64);

impl std::fmt::Display for Microseconds {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}us", self.0)
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct FrameMetrics {
    pub(crate) context_build_us: Microseconds,
    pub(crate) engine_process_us: Microseconds,
    pub(crate) effects_dispatch_us: Microseconds,
    pub(crate) ui_update_us: Microseconds,
    pub(crate) total_us: Microseconds,
}

#[derive(Debug)]
pub(crate) struct PerfTracker {
    frames: Vec<FrameMetrics>,
    cursor: usize,
    count: usize,
    budget_us: Microseconds,
}

impl PerfTracker {
    #[must_use]
    pub(crate) fn new(capacity: usize, budget_us: Microseconds) -> Self {
        assert!(capacity > 0, "PerfTracker capacity must be > 0");
        Self {
            frames: vec![FrameMetrics::default(); capacity],
            cursor: 0,
            count: 0,
            budget_us,
        }
    }

    pub(crate) fn record(&mut self, metrics: FrameMetrics) {
        if metrics.total_us > self.budget_us {
            log::warn!(
                "Frame budget exceeded: {} > {} | ctx={} eng={} fx={} ui={}",
                metrics.total_us, self.budget_us,
                metrics.context_build_us, metrics.engine_process_us,
                metrics.effects_dispatch_us, metrics.ui_update_us,
            );
        }
        self.frames[self.cursor] = metrics;
        self.cursor = (self.cursor + 1) % self.frames.len();
        if self.count < self.frames.len() {
            self.count += 1;
        }
    }

    #[must_use]
    pub(crate) fn percentiles(&self) -> (Microseconds, Microseconds, Microseconds, Microseconds) {
        if self.count == 0 {
            return (Microseconds(0), Microseconds(0), Microseconds(0), Microseconds(0));
        }
        let mut totals: Vec<Microseconds> = self.frames[..self.count]
            .iter()
            .map(|f| f.total_us)
            .collect();
        totals.sort_unstable();
        let p = |pct: f64| -> Microseconds {
            let idx = ((pct / 100.0) * (totals.len() as f64 - 1.0)).ceil().max(0.0) as usize;
            totals[idx.min(totals.len() - 1)]
        };
        let &last = totals.last().unwrap_or(&Microseconds(0));
        (p(50.0), p(95.0), p(99.0), last)
    }

    pub(crate) fn reset(&mut self) {
        self.count = 0;
        self.cursor = 0;
    }

    #[must_use]
    pub(crate) fn format_report(&self) -> String {
        let (p50, p95, p99, max) = self.percentiles();
        format!(
            ":perf ({} frames)\n  p50: {}\n  p95: {}\n  p99: {}\n  max: {}\n  budget: {}",
            self.count, p50, p95, p99, max, self.budget_us
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn us(val: u64) -> Microseconds {
        Microseconds(val)
    }

    #[test]
    fn empty_tracker_returns_zeros() {
        let tracker = PerfTracker::new(100, us(2000));
        assert_eq!(tracker.percentiles(), (us(0), us(0), us(0), us(0)));
    }

    #[test]
    fn single_frame() {
        let mut tracker = PerfTracker::new(100, us(2000));
        tracker.record(FrameMetrics { total_us: us(500), ..Default::default() });
        let (p50, _, _, max) = tracker.percentiles();
        assert_eq!(p50, us(500));
        assert_eq!(max, us(500));
    }

    #[test]
    fn ring_buffer_wraps() {
        let mut tracker = PerfTracker::new(3, us(2000));
        tracker.record(FrameMetrics { total_us: us(100), ..Default::default() });
        tracker.record(FrameMetrics { total_us: us(200), ..Default::default() });
        tracker.record(FrameMetrics { total_us: us(300), ..Default::default() });
        tracker.record(FrameMetrics { total_us: us(400), ..Default::default() });
        // Capacity 3, so oldest (100) was overwritten
        assert_eq!(tracker.count, 3);
        let (_, _, _, max) = tracker.percentiles();
        assert_eq!(max, us(400));
    }

    #[test]
    fn reset_clears() {
        let mut tracker = PerfTracker::new(100, us(2000));
        tracker.record(FrameMetrics { total_us: us(500), ..Default::default() });
        tracker.reset();
        assert_eq!(tracker.percentiles(), (us(0), us(0), us(0), us(0)));
    }

    #[test]
    fn format_report_includes_stats() {
        let mut tracker = PerfTracker::new(100, us(2000));
        tracker.record(FrameMetrics { total_us: us(500), ..Default::default() });
        let report = tracker.format_report();
        assert!(report.contains(":perf"));
        assert!(report.contains("500us"));
    }

    #[test]
    #[should_panic(expected = "PerfTracker capacity must be > 0")]
    fn zero_capacity_panics() {
        let _ = PerfTracker::new(0, us(2000));
    }
}
