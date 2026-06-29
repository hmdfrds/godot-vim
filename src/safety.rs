//! Panic safety boundary for the Godot FFI.
//!
//! Rust panics that unwind across the C FFI boundary into Godot are undefined
//! behavior. This module provides two layers of defense:
//!
//! 1. **Panic hook** ([`install_panic_hook`]) -- routes panic messages to
//!    Godot's Debugger/Output panels with file:line:column context.
//! 2. **Catch guard** ([`panic_guard`]) -- wraps every signal handler callback
//!    in `catch_unwind`, returning a safe default instead of unwinding into C.
//!
//! Both layers fire on the same panic (intentionally): the hook provides source
//! location, while the guard identifies which signal handler caught it.

use std::any::Any;
use std::panic::AssertUnwindSafe;
use std::sync::Once;

use godot::prelude::*;

fn extract_panic_message(payload: &dyn Any) -> String {
    if let Some(s) = payload.downcast_ref::<&str>() {
        s.to_string()
    } else if let Some(s) = payload.downcast_ref::<String>() {
        s.clone()
    } else {
        "unknown panic".to_string()
    }
}

/// Install a panic hook that routes messages to Godot's Debugger panel.
///
/// Chains with the previously installed hook (via `take_hook` + forward) so
/// other gdext plugins and Rust libraries keep working. `Once` prevents
/// hook accumulation across hot-reloads or multiple plugin instances.
pub(crate) fn install_panic_hook() {
    static HOOK_INSTALLED: Once = Once::new();
    HOOK_INSTALLED.call_once(|| {
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            let msg = extract_panic_message(info.payload());
            let location = info.location().map_or_else(String::new, |loc| {
                format!(" at {}:{}:{}", loc.file(), loc.line(), loc.column(),)
            });
            // Gate: only report if the engine is up, we're inside one of our
            // guards (foreign panics are not mislabeled), and the signature
            // hasn't been seen within the dedup window.
            if godot::sys::is_initialized()
                && GUARD_DEPTH.with(|d| d.get()) > 0
                && DEDUP_HOOK.with(|d| {
                    d.borrow_mut().should_report(&format!("{location}:{msg}"), now_ms())
                })
            {
                godot_error!("GodotVim panic{location}: {msg}");
            }
            // Chain to the previous hook (typically gdext's default handler).
            previous(info);
        }));
    });
}

/// Bounded dedup: suppress identical consecutive panic signatures within a
/// short window. `now_ms` is supplied by the caller (monotonic millis) so the
/// core is pure and testable. Single-threaded editor use → thread-local, no lock.
struct DedupState {
    last_sig: Option<String>,
    last_ms: u64,
}
const DEDUP_WINDOW_MS: u64 = 1_000;
impl DedupState {
    fn new() -> Self {
        Self { last_sig: None, last_ms: 0 }
    }
    fn should_report(&mut self, sig: &str, now_ms: u64) -> bool {
        let dup = self.last_sig.as_deref() == Some(sig)
            && now_ms.saturating_sub(self.last_ms) < DEDUP_WINDOW_MS;
        self.last_sig = Some(sig.to_string());
        self.last_ms = now_ms;
        !dup
    }
}

thread_local! {
    static GUARD_DEPTH: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
    // SEPARATE dedup state per emit site. The hook (:49) and the guard (:73) BOTH
    // fire for every panic and interleave (hook, guard, hook, guard, ...); a SHARED
    // DedupState would see alternating signatures and never suppress anything.
    static DEDUP_HOOK: std::cell::RefCell<DedupState> =
        std::cell::RefCell::new(DedupState::new());
    static DEDUP_GUARD: std::cell::RefCell<DedupState> =
        std::cell::RefCell::new(DedupState::new());
}

fn now_ms() -> u64 {
    // godot Time is available; OS time also fine. Keep it panic-free.
    godot::classes::Time::singleton().get_ticks_msec()
}

/// Wrap a closure in `catch_unwind`, returning `default` on panic.
///
/// Every Godot signal handler and virtual override in this plugin should be
/// wrapped with this guard. The `label` identifies which handler caught the
/// panic in error messages. The `AssertUnwindSafe` is sound because every
/// engine-mutating callsite performs comprehensive state recovery after a
/// panic, restoring invariants before the next operation.
pub(crate) fn panic_guard<F, R>(label: &str, f: F, default: R) -> R
where
    F: FnOnce() -> R,
{
    GUARD_DEPTH.with(|d| d.set(d.get() + 1));
    let result = std::panic::catch_unwind(AssertUnwindSafe(f));
    GUARD_DEPTH.with(|d| d.set(d.get().saturating_sub(1)));
    match result {
        Ok(r) => r,
        Err(e) => {
            let msg = extract_panic_message(e.as_ref());
            if godot::sys::is_initialized()
                && DEDUP_GUARD.with(|d| {
                    d.borrow_mut()
                        .should_report(&format!("{label}:{msg}"), now_ms())
                })
            {
                godot_error!("GodotVim panic in {label}: {msg}");
            }
            default
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dedup_suppresses_identical_within_window() {
        let mut state = DedupState::new();
        assert!(state.should_report("sig-A", 0));          // first: report
        assert!(!state.should_report("sig-A", 100));       // dup within window: suppress
        assert!(state.should_report("sig-B", 150));        // different sig: report
        assert!(state.should_report("sig-A", 10_000));     // window elapsed: report again
    }

    // Models the REAL wiring: hook and guard are SEPARATE DedupState streams that
    // interleave (hook, guard, hook, guard, ...) for a repeated panic. A SHARED
    // state would never suppress (alternating sigs); separate states must.
    #[test]
    fn separate_streams_each_suppress_repeats() {
        let mut hook = DedupState::new();
        let mut guard = DedupState::new();
        // First panic: both streams report (informative first-occurrence pair).
        assert!(hook.should_report("loc:msg", 0));
        assert!(guard.should_report("label:msg", 1));
        // Same panic repeats within the window: BOTH streams suppress.
        assert!(!hook.should_report("loc:msg", 2));
        assert!(!guard.should_report("label:msg", 3));
        assert!(!hook.should_report("loc:msg", 4));
        assert!(!guard.should_report("label:msg", 5));
    }
}
