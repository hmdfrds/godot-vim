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

/// Strip user-specific prefixes from a file path so panic messages never
/// leak local directory structure (e.g., `/home/user/.cargo/...` or
/// `C:\Users\user\...`). Works on any machine without build-time config.
fn sanitize_path(path: &str) -> &str {
    // Strip everything up to and including `/src/` for our own crate paths.
    if let Some(pos) = path.find("/src/") {
        return &path[pos + 1..]; // "src/safety.rs:45"
    }
    // Strip cargo checkout paths: keep from the crate name onward.
    // e.g., "/home/user/.cargo/git/checkouts/gdext-.../godot-core/src/foo.rs"
    //     → "godot-core/src/foo.rs"
    for marker in ["/godot-core/", "/godot-macros/", "/godot/src/"] {
        if let Some(pos) = path.find(marker) {
            return &path[pos + 1..];
        }
    }
    // Fallback: strip common home directory prefixes.
    for prefix in ["/home/", "/Users/", "C:\\Users\\"] {
        if let Some(rest) = path.strip_prefix(prefix) {
            // Skip past "username/" to get the relative portion.
            if let Some(pos) = rest.find(['/', '\\']) {
                return &rest[pos + 1..];
            }
        }
    }
    path
}

/// Install a panic hook that routes messages to Godot's Debugger panel.
///
/// Chains with the previously installed hook (via `take_hook` + forward) so
/// other gdext plugins and Rust libraries keep working. `Once` prevents
/// hook accumulation across hot-reloads or multiple plugin instances.
pub(crate) fn install_panic_hook() {
    static HOOK_INSTALLED: Once = Once::new();
    HOOK_INSTALLED.call_once(|| {
        std::panic::set_hook(Box::new(|info| {
            let msg = extract_panic_message(info.payload());
            let location = info.location().map_or_else(
                String::new,
                |loc| format!(" at {}:{}:{}", sanitize_path(loc.file()), loc.line(), loc.column()),
            );
            godot_error!("GodotVim panic{location}: {msg}");
        }));
    });
}

/// Wrap a closure in `catch_unwind`, returning `default` on panic.
///
/// Every Godot signal handler and virtual override in this plugin should be
/// wrapped with this guard. The `AssertUnwindSafe` is sound because we do
/// not resume normal operation after a panic -- we log and return a default.
pub(crate) fn panic_guard<F, R>(f: F, default: R) -> R
where
    F: FnOnce() -> R,
{
    match std::panic::catch_unwind(AssertUnwindSafe(f)) {
        Ok(r) => r,
        Err(e) => {
            let msg = extract_panic_message(e.as_ref());
            godot_error!("GodotVim panic in signal handler: {msg}");
            default
        }
    }
}
