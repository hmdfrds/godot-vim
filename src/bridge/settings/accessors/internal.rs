use godot::builtin::{Array, VarDictionary};
use godot::classes::{EditorInterface, EditorSettings};
use godot::prelude::*;

/// Gets EditorSettings singleton. Returns None if not in editor context.
fn get_editor_settings() -> Option<Gd<EditorSettings>> {
    // EditorInterface is only available in editor context
    if !godot::classes::Engine::singleton().is_editor_hint() {
        return None;
    }
    EditorInterface::singleton().get_editor_settings()
}

/// Retrieves a boolean setting with fallback to default.
pub(super) fn get_bool_setting(key: &str, default: bool) -> bool {
    let Some(settings) = get_editor_settings() else {
        return default;
    };
    let key_gstring: GString = key.into();

    if settings.has_setting(&key_gstring) {
        settings
            .get_setting(&key_gstring)
            .try_to::<bool>()
            .unwrap_or_else(|_| {
                log::warn!(
                    "Type mismatch for setting key={}, using default={}",
                    key,
                    default
                );
                default
            })
    } else {
        default
    }
}

/// Sets a boolean setting.
pub(super) fn set_bool_setting(key: &str, value: bool) {
    let Some(mut settings) = get_editor_settings() else {
        return;
    };
    let key_gstring: GString = key.into();
    settings.set_setting(&key_gstring, &Variant::from(value));
}

/// Retrieves a color setting with fallback to default.
pub(super) fn get_color_setting(key: &str, default: Color) -> Color {
    let Some(settings) = get_editor_settings() else {
        return default;
    };
    let key_gstring: GString = key.into();

    if settings.has_setting(&key_gstring) {
        settings
            .get_setting(&key_gstring)
            .try_to::<Color>()
            .unwrap_or_else(|_| {
                log::warn!(
                    "Type mismatch for setting key={}, using default color",
                    key
                );
                default
            })
    } else {
        default
    }
}

/// Retrieves an integer setting with fallback to default.
pub(super) fn get_int_setting(key: &str, default: i64) -> i64 {
    let Some(settings) = get_editor_settings() else {
        return default;
    };
    let key_gstring: GString = key.into();

    if settings.has_setting(&key_gstring) {
        settings
            .get_setting(&key_gstring)
            .try_to::<i64>()
            .unwrap_or_else(|_| {
                log::warn!(
                    "Type mismatch for setting key={}, using default={}",
                    key,
                    default
                );
                default
            })
    } else {
        default
    }
}

/// Retrieves a string setting with fallback to default.
pub(super) fn get_string_setting(key: &str, default: &str) -> String {
    let Some(settings) = get_editor_settings() else {
        return default.to_string();
    };
    let key_gstring: GString = key.into();

    if settings.has_setting(&key_gstring) {
        settings
            .get_setting(&key_gstring)
            .try_to::<String>()
            .unwrap_or_else(|_| default.to_string())
    } else {
        default.to_string()
    }
}

/// Retrieves a string array setting.
pub(super) fn get_string_array_setting(key: &str) -> Vec<String> {
    let Some(settings) = get_editor_settings() else {
        return Vec::new();
    };
    let key_gstring: GString = key.into();

    if settings.has_setting(&key_gstring) {
        if let Ok(array) = settings
            .get_setting(&key_gstring)
            .try_to::<PackedStringArray>()
        {
            return array.to_vec().into_iter().map(|s| s.to_string()).collect();
        }
    }
    Vec::new()
}

/// Retrieves a mapping dictionary from `EditorSettings`.
pub(super) fn get_mapping_dictionary(key: &str) -> Vec<(String, String)> {
    let Some(settings) = get_editor_settings() else {
        return Vec::new();
    };
    let key_gstring: GString = key.into();

    if !settings.has_setting(&key_gstring) {
        return Vec::new();
    }

    let variant = settings.get_setting(&key_gstring);
    if let Ok(dict) = variant.try_to::<VarDictionary>() {
        dict.keys_array()
            .iter_shared()
            .filter_map(|k| {
                let from = k.try_to::<GString>().ok()?.to_string();
                let to = dict.get(k.clone())?.try_to::<GString>().ok()?.to_string();
                Some((from, to))
            })
            .collect()
    } else {
        Vec::new()
    }
}

/// Retrieves the unified mapping array from `EditorSettings`.
pub(super) fn get_mapping_array(key: &str) -> Vec<(String, String, String)> {
    let Some(settings) = get_editor_settings() else {
        return Vec::new();
    };
    let key_gstring: GString = key.into();

    if !settings.has_setting(&key_gstring) {
        return Vec::new();
    }

    let variant = settings.get_setting(&key_gstring);
    if let Ok(array) = variant.try_to::<Array<Variant>>() {
        array
            .iter_shared()
            .filter_map(|v| {
                let dict = v.try_to::<VarDictionary>().ok()?;
                let from = dict.get("from")?.try_to::<GString>().ok()?.to_string();
                let to = dict.get("to")?.try_to::<GString>().ok()?.to_string();
                let modes = dict.get("modes")?.try_to::<GString>().ok()?.to_string();
                Some((from, to, modes))
            })
            .collect()
    } else {
        Vec::new()
    }
}

/// Normalizes a key string to a canonical format for comparison.
///
/// Converts both formats to lowercase with standardized separators:
/// - "Ctrl+S" -> "ctrl+s"
/// - `"<C-s>"` -> "ctrl+s"
/// - `"<C-S>"` -> "ctrl+s"
/// - "F5" -> "f5"
/// - `"<F5>"` -> "f5"
#[must_use]
pub(super) fn normalize_key_string(key: &str) -> String {
    let lowercased = key.trim().to_lowercase();

    // Remove angle brackets if present
    let without_prefix = lowercased.strip_prefix('<').unwrap_or(&lowercased);
    let without_brackets = without_prefix.strip_suffix('>').unwrap_or(without_prefix);

    // Replace Vim notation with user-friendly format
    without_brackets
        .replace("c-", "ctrl+")
        .replace("s-", "shift+")
        .replace("a-", "alt+")
        .replace("m-", "meta+")
        .replace("d-", "meta+") // D is Cmd/Super/Meta
}
