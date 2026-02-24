//! Global Error Handling and Panic Safety
//!
//! This module provides the "Uncrashable Kernel" mechanisms:
//! 1. `install_panic_hook`: Pipes Rust panics to Godot's console.
//! 2. `guard`: Wraps FFI boundaries to catch unwinds and prevent Godot crashes.

use godot::prelude::*;
use std::panic::{self, AssertUnwindSafe};

/// Installs a custom panic hook that logs to Godot's error console.
///
/// Chains with the previous hook instead of replacing it, so other plugins'
/// panic hooks are preserved.
///
/// This should be called once at plugin initialization.
pub fn install_panic_hook() {
    let previous_hook = panic::take_hook();
    panic::set_hook(Box::new(move |info| {
        let location = info
            .location()
            .map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column()))
            .unwrap_or_else(|| "unknown location".to_string());

        let msg = if let Some(s) = info.payload().downcast_ref::<&str>() {
            format!("Panic occurred: {}", s)
        } else if let Some(s) = info.payload().downcast_ref::<String>() {
            format!("Panic occurred: {}", s)
        } else {
            "Panic occurred (unknown payload)".to_string()
        };

        let err_msg = format!("CRITICAL RUST PANIC at {}: {}", location, msg);

        // Log to Godot's error console (visible in Debugger)
        godot_error!("{}", err_msg);

        // Also print to stderr for terminal users
        eprintln!("{}", err_msg);

        // Chain to previous hook (preserves other plugins' hooks)
        previous_hook(info);
    }));
}

/// Guards a closure against panics, returning a default value on failure.
///
/// Use this at FFI boundaries (signals, exposed methods) to prevent
/// a Rust panic from crashing the Godot engine.
///
/// # Type Parameters
/// * `F`: The closure to execute. Must be `UnwindSafe`.
/// * `T`: The return type.
///
/// # Arguments
/// * `f`: The closure to guards.
/// * `fallback`: The value to return if a panic occurs.
pub fn guard<F, T>(f: F, fallback: T) -> T
where
    F: FnOnce() -> T,
{
    match panic::catch_unwind(AssertUnwindSafe(f)) {
        Ok(result) => result,
        Err(payload) => {
            let msg = if let Some(s) = payload.downcast_ref::<&str>() {
                format!("Recovered from panic in guarded block: {}", s)
            } else if let Some(s) = payload.downcast_ref::<String>() {
                format!("Recovered from panic in guarded block: {}", s)
            } else {
                "Recovered from panic in guarded block (unknown payload). Utilizing fallback."
                    .to_string()
            };
            godot_error!("{}", msg);
            fallback
        }
    }
}
