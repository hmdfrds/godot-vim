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
            // Guard: godot_error! requires the Godot engine to be
            // initialized. During shutdown a panic can fire after the
            // engine has de-initialized; calling godot_error! then would
            // double-panic and abort.
            if godot::sys::is_initialized() {
                godot_error!("GodotVim panic{location}: {msg}");
            }
            // Chain to the previous hook (typically gdext's default handler).
            previous(info);
        }));
    });
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
    match std::panic::catch_unwind(AssertUnwindSafe(f)) {
        Ok(r) => r,
        Err(e) => {
            let msg = extract_panic_message(e.as_ref());
            if godot::sys::is_initialized() {
                godot_error!("GodotVim panic in {label}: {msg}");
            }
            default
        }
    }
}
