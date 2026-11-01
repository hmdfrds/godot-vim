//! The few places where the Android editor needs other behaviour.

use std::sync::OnceLock;

use godot::classes::Os;
use godot::prelude::*;

/// Asked of Godot at run time, as `host::external` asks for Windows.
pub(crate) fn is_android() -> bool {
    static ANDROID: OnceLock<bool> = OnceLock::new();
    *ANDROID.get_or_init(|| Os::singleton().get_name() == "Android")
}
