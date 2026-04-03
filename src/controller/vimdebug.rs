//! State management for `:vimdebug` — a developer tool for inspecting
//! the engine's effect pipeline in real time.
//!
//! Two modes:
//! - **Watch**: status bar annotations (provenance, effect summary, affected range).
//! - **Step**: pauses after engine processing for effect-by-effect inspection
//!   with n(ext) / p(rev) / c(ontinue) / q(uit) navigation.

use compact_str::CompactString;

use crate::types::MatchRange;

const STEP_DESC_MAX_LEN: usize = 60;

#[derive(Debug, Default)]
pub(crate) struct VimdebugState {
    mode: VimdebugMode,
    provenance: Option<CompactString>,
    effects: Option<CompactString>,
    range: Option<MatchRange>,
    step_pending: Vec<StepEffect>,
    step_index: usize,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) enum VimdebugMode {
    #[default]
    Off,
    Watch,
    Step,
}

#[derive(Debug, Clone)]
pub(crate) struct StepEffect {
    pub(crate) description: CompactString,
    pub(crate) applied: bool,
}

impl VimdebugState {
    #[must_use]
    pub(crate) const fn is_enabled(&self) -> bool {
        !matches!(self.mode, VimdebugMode::Off)
    }

    #[must_use]
    pub(crate) const fn is_step_mode(&self) -> bool {
        matches!(self.mode, VimdebugMode::Step)
    }

    #[must_use]
    pub(crate) const fn mode(&self) -> VimdebugMode {
        self.mode
    }

    pub(crate) fn set_mode(&mut self, mode: VimdebugMode) {
        self.mode = mode;
        if mode == VimdebugMode::Off {
            // Exhaustive destructure: adding a new field to VimdebugState
            // causes a compile error here until it is handled.
            let Self {
                mode: _,          // just set above
                provenance,
                effects,
                range,
                step_pending,
                step_index,
            } = self;
            *provenance = None;
            *effects = None;
            *range = None;
            step_pending.clear();
            *step_index = 0;
        }
    }

    pub(crate) fn provenance(&self) -> Option<&CompactString> {
        self.provenance.as_ref()
    }

    pub(crate) fn effects_summary(&self) -> Option<&CompactString> {
        self.effects.as_ref()
    }

    pub(crate) fn range(&self) -> Option<MatchRange> {
        self.range.clone()
    }

    pub(crate) fn capture_provenance(&mut self, provenance: Option<&str>) {
        if self.is_enabled() {
            self.provenance = provenance.map(CompactString::from);
        }
    }

    pub(crate) fn capture_effects_summary(&mut self, summary: CompactString) {
        if self.is_enabled() {
            self.effects = Some(summary);
        }
    }

    pub(crate) fn capture_range(&mut self, range: Option<MatchRange>) {
        if self.is_enabled() {
            self.range = range;
        }
    }

    pub(crate) fn clear_captures(&mut self) {
        self.provenance = None;
        self.effects = None;
        self.range = None;
    }

    pub(crate) fn load_step_effects(&mut self, descriptions: Vec<CompactString>) {
        self.step_pending = descriptions
            .into_iter()
            .map(|d| StepEffect { description: d, applied: false })
            .collect();
        self.step_index = 0;
    }

    /// Advance to the next unapplied effect. Skips already-applied effects
    /// so that prev+next doesn't double-dispatch.
    pub(crate) fn step_next(&mut self) -> Option<usize> {
        while self.step_index < self.step_pending.len() {
            let idx = self.step_index;
            self.step_index += 1;
            if !self.step_pending[idx].applied {
                self.step_pending[idx].applied = true;
                return Some(idx);
            }
        }
        None
    }

    /// Display-only rewind (does not unapply the effect).
    pub(crate) fn step_prev(&mut self) {
        if self.step_index > 0 {
            self.step_index -= 1;
        }
    }

    pub(crate) fn step_continue(&mut self) -> Vec<usize> {
        let remaining: Vec<usize> = (self.step_index..self.step_pending.len())
            .filter(|&i| !self.step_pending[i].applied)
            .collect();
        for idx in &remaining {
            self.step_pending[*idx].applied = true;
        }
        self.step_index = self.step_pending.len();
        remaining
    }

    pub(crate) fn step_quit(&mut self) {
        self.step_pending.clear();
        self.step_index = 0;
    }

    pub(crate) fn has_pending_steps(&self) -> bool {
        !self.step_pending.is_empty() && self.step_index < self.step_pending.len()
    }

    pub(crate) fn step_status_line(&self) -> Option<CompactString> {
        if self.step_pending.is_empty() {
            return None;
        }
        let total = self.step_pending.len();
        let current = self.step_index.min(total);
        let desc = if current < total {
            let d = &self.step_pending[current].description;
            if d.len() > STEP_DESC_MAX_LEN {
                let end = d.floor_char_boundary(STEP_DESC_MAX_LEN);
                &d[..end]
            } else {
                d.as_str()
            }
        } else {
            "(all applied)"
        };
        let display_idx = if current < total { current + 1 } else { total };
        Some(compact_str::format_compact!(
            "[{}/{}] {} | n:next p:prev c:all q:quit",
            display_idx, total, desc
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_off() {
        let state = VimdebugState::default();
        assert!(!state.is_enabled());
        assert!(!state.is_step_mode());
        assert_eq!(state.mode(), VimdebugMode::Off);
    }

    #[test]
    fn watch_mode_is_enabled() {
        let mut state = VimdebugState::default();
        state.set_mode(VimdebugMode::Watch);
        assert!(state.is_enabled());
        assert!(!state.is_step_mode());
    }

    #[test]
    fn step_mode_is_enabled_and_step() {
        let mut state = VimdebugState::default();
        state.set_mode(VimdebugMode::Step);
        assert!(state.is_enabled());
        assert!(state.is_step_mode());
    }

    #[test]
    fn set_off_clears_captures() {
        let mut state = VimdebugState::default();
        state.set_mode(VimdebugMode::Watch);
        state.capture_provenance(Some("dd"));
        state.set_mode(VimdebugMode::Off);
        assert!(state.provenance().is_none());
    }

    #[test]
    fn capture_only_when_enabled() {
        let mut state = VimdebugState::default();
        state.capture_provenance(Some("dd"));
        assert!(state.provenance().is_none());

        state.set_mode(VimdebugMode::Watch);
        state.capture_provenance(Some("dd"));
        assert_eq!(state.provenance().unwrap().as_str(), "dd");
    }

    #[test]
    fn step_navigation() {
        let mut state = VimdebugState::default();
        state.set_mode(VimdebugMode::Step);
        state.load_step_effects(vec![
            CompactString::from("SetCursor(5)"),
            CompactString::from("SetMode(Normal)"),
            CompactString::from("ShowMessage(ok)"),
        ]);
        assert!(state.has_pending_steps());

        assert_eq!(state.step_next(), Some(0));
        assert_eq!(state.step_next(), Some(1));

        state.step_prev();
        assert!(state.has_pending_steps());

        let remaining = state.step_continue();
        // step_prev is display-only — effect 1 was already applied, so
        // step_continue only returns the remaining unapplied effect (index 2).
        assert_eq!(remaining, vec![2]);
        assert!(!state.has_pending_steps());
    }

    #[test]
    fn step_next_returns_none_at_end() {
        let mut state = VimdebugState::default();
        state.set_mode(VimdebugMode::Step);
        state.load_step_effects(vec![CompactString::from("effect1")]);
        assert_eq!(state.step_next(), Some(0));
        assert_eq!(state.step_next(), None);
    }

    #[test]
    fn step_quit_clears() {
        let mut state = VimdebugState::default();
        state.set_mode(VimdebugMode::Step);
        state.load_step_effects(vec![CompactString::from("effect1")]);
        state.step_quit();
        assert!(!state.has_pending_steps());
        assert!(state.step_status_line().is_none());
    }

    #[test]
    fn step_status_line_format() {
        let mut state = VimdebugState::default();
        state.set_mode(VimdebugMode::Step);
        state.load_step_effects(vec![
            CompactString::from("SetCursor(5, 12)"),
            CompactString::from("SetMode(Normal)"),
        ]);
        let line = state.step_status_line().unwrap();
        assert!(line.contains("[1/2]"));
        assert!(line.contains("SetCursor"));
        assert!(line.contains("n:next"));
    }
}
