//! Signal connection helpers that encapsulate the `is_connected` guard +
//! connect/disconnect pattern used across `attach.rs`, `lifecycle.rs`,
//! and `floating.rs`.

use godot::classes::object::ConnectFlags;
use godot::classes::Object;
use godot::prelude::*;

/// Connect with DEFERRED delivery (idempotent). Required for signals that
/// fire during re-entrant contexts (e.g. `caret_changed` during text edits).
pub(super) fn connect_deferred(
    target: &mut Gd<impl Inherits<Object>>,
    signal: &str,
    callable: &Callable,
) {
    let mut obj = target.clone().upcast::<Object>();
    if !obj.is_connected(signal, callable) {
        let err = obj.connect_flags(signal, callable, ConnectFlags::DEFERRED);
        if err != godot::global::Error::OK {
            log::warn!("Failed to connect signal '{}' (deferred): {:?}", signal, err);
        }
    }
}

/// Connect with immediate delivery (idempotent). Used for signals that must
/// be handled synchronously (e.g. `gui_input` -- deferred delivery would miss
/// the `set_input_as_handled` window).
pub(super) fn connect_immediate(
    target: &mut Gd<impl Inherits<Object>>,
    signal: &str,
    callable: &Callable,
) {
    let mut obj = target.clone().upcast::<Object>();
    if !obj.is_connected(signal, callable) {
        let err = obj.connect(signal, callable);
        if err != godot::global::Error::OK {
            log::warn!("Failed to connect signal '{}' (immediate): {:?}", signal, err);
        }
    }
}

/// Idempotent disconnect. Prevents Godot's "signal not connected" error
/// when detaching from a partially-attached or already-cleaned-up editor.
pub(super) fn safe_disconnect(
    target: &mut Gd<impl Inherits<Object>>,
    signal: &str,
    callable: &Callable,
) {
    if !target.is_instance_valid() {
        return;
    }
    let mut obj = target.clone().upcast::<Object>();
    if obj.is_connected(signal, callable) {
        obj.disconnect(signal, callable);
    }
}
