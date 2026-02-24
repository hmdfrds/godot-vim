//! Settings registration in Godot's `EditorSettings`.

use godot::builtin::VarDictionary;
use godot::classes::{EditorInterface, EditorSettings};
use godot::global::PropertyHint;
use godot::prelude::*;

use super::{defaults, keys, presets};

/// Registers all `GodotVim` settings in `EditorSettings`.
///
/// This should be called once during plugin initialization.
/// Settings will appear in Editor → Editor Settings → Plugins → GodotVim
///
/// Structure:
/// - `plugins/GodotVim/enabled` - Master toggle
/// - `plugins/GodotVim/log_level` - Logging verbosity
/// - `plugins/GodotVim/editor/` - Editor behavior (scroll, line numbers, etc.)
/// - `plugins/GodotVim/cursor/` - Cursor appearance
/// - `plugins/GodotVim/clipboard/` - System clipboard integration
/// - `plugins/GodotVim/mapping/` - Key mappings
pub fn register_settings() {
    // EditorSettings is only available in editor context
    if !godot::classes::Engine::singleton().is_editor_hint() {
        log::warn!("Cannot register settings: not in editor context");
        return;
    }

    let Some(mut settings) = EditorInterface::singleton().get_editor_settings() else {
        log::warn!("Cannot register settings: EditorSettings not available");
        return;
    };

    // ═══════════════════════════════════════════════════════════════════════════════
    // Core Settings (Top Level - Most Important)
    // ═══════════════════════════════════════════════════════════════════════════════
    register_bool_setting(&mut settings, keys::ENABLED, defaults::ENABLED);
    register_enum_setting(
        &mut settings,
        keys::LOG_LEVEL,
        defaults::LOG_LEVEL,
        "OFF,ERROR,WARNING,INFO,DEBUG,TRACE",
    );

    // ═══════════════════════════════════════════════════════════════════════════════
    // Editor Settings
    // ═══════════════════════════════════════════════════════════════════════════════
    register_int_setting(
        &mut settings,
        keys::SCROLL_OFFSET,
        defaults::SCROLL_OFFSET,
        0,
        20,
    );
    register_bool_setting(
        &mut settings,
        keys::HIGHLIGHT_CURRENT_LINE,
        defaults::HIGHLIGHT_CURRENT_LINE,
    );
    register_enum_setting(
        &mut settings,
        keys::LINE_NUMBER_MODE,
        defaults::LINE_NUMBER_MODE,
        "None,Absolute,Relative,Hybrid",
    );
    register_bool_setting(
        &mut settings,
        keys::HUD_CMDLINE_ENABLED,
        defaults::HUD_CMDLINE_ENABLED,
    );
    register_string_array_setting(&mut settings, keys::KEY_PASSTHROUGH_LIST);
    register_string_setting(&mut settings, keys::IS_KEYWORD, defaults::IS_KEYWORD);

    // ═══════════════════════════════════════════════════════════════════════════════
    // Cursor Settings
    // ═══════════════════════════════════════════════════════════════════════════════
    register_bool_setting(
        &mut settings,
        keys::MODE_COLORS_ENABLED,
        defaults::MODE_COLORS_ENABLED,
    );
    register_bool_setting(
        &mut settings,
        keys::PREMIUM_CURSOR_ENABLED,
        defaults::PREMIUM_CURSOR_ENABLED,
    );
    register_color_setting(
        &mut settings,
        keys::NORMAL_MODE_COLOR,
        defaults::NORMAL_MODE_COLOR,
    );
    register_color_setting(
        &mut settings,
        keys::INSERT_MODE_COLOR,
        defaults::INSERT_MODE_COLOR,
    );
    register_color_setting(
        &mut settings,
        keys::VISUAL_MODE_COLOR,
        defaults::VISUAL_MODE_COLOR,
    );

    // ═══════════════════════════════════════════════════════════════════════════════
    // Clipboard Settings
    // ═══════════════════════════════════════════════════════════════════════════════
    register_bool_setting(
        &mut settings,
        keys::YANK_TO_CLIPBOARD,
        defaults::YANK_TO_CLIPBOARD,
    );
    register_bool_setting(
        &mut settings,
        keys::DELETE_TO_CLIPBOARD,
        defaults::DELETE_TO_CLIPBOARD,
    );

    // ═══════════════════════════════════════════════════════════════════════════════
    // Mapping Settings
    // ═══════════════════════════════════════════════════════════════════════════════
    register_bool_setting(
        &mut settings,
        keys::MAPPING_ENABLED,
        defaults::MAPPING_ENABLED,
    );
    register_int_setting(
        &mut settings,
        keys::MAPPING_TIMEOUTLEN,
        defaults::MAPPING_TIMEOUTLEN,
        100,  // min: 100ms
        5000, // max: 5000ms
    );

    // Register mapping dictionaries (imap, nmap, vmap, all)
    register_mapping_dictionaries(&mut settings);
}

/// Registers a string setting with the given key and default value.
fn register_string_setting(settings: &mut Gd<EditorSettings>, key: &str, default: &str) {
    let key_gstring: GString = key.into();
    let key_stringname: StringName = key.into();

    if !settings.has_setting(&key_gstring) {
        settings.set_setting(&key_gstring, &Variant::from(default));
    }

    // EditorSettings uses set_initial_value for defaults
    settings.set_initial_value(&key_stringname, &Variant::from(default), false);

    let property_info = vdict! {
        "name": key,
        "type": VariantType::STRING.ord(),
        "hint": PropertyHint::NONE.ord(),
        "hint_string": "",
    };
    settings.add_property_info(&property_info);
}

/// Register mapping dictionaries with preset defaults.
fn register_mapping_dictionaries(settings: &mut Gd<EditorSettings>) {
    // Register empty dictionaries by default (user populates via panel)
    let empty_dict = VarDictionary::new();
    register_dictionary_setting(settings, keys::IMAP, &empty_dict);
    register_dictionary_setting(settings, keys::NMAP, &empty_dict);
    register_dictionary_setting(settings, keys::VMAP, &empty_dict);
    register_dictionary_setting(settings, keys::GMAP, &empty_dict);

    // Register ALL_MAPPINGS unified array with PRESETS as default
    // All mappings are disabled by default (modes = "")
    // Users enable them via the Mappings panel
    let mut all_mappings_default = godot::builtin::Array::<Variant>::new();
    for preset in presets::get_recommended_mappings() {
        let mut dict = VarDictionary::new();
        dict.set("from", preset.from);
        dict.set("to", preset.to);
        dict.set("modes", preset.modes); // Empty string = disabled
        all_mappings_default.push(&dict.to_variant());
    }

    register_array_setting(settings, keys::ALL_MAPPINGS, &all_mappings_default);
}

// ═══════════════════════════════════════════════════════════════════════════════
// Registration Helpers
// ═══════════════════════════════════════════════════════════════════════════════

/// Registers a boolean setting with the given key and default value.
fn register_bool_setting(settings: &mut Gd<EditorSettings>, key: &str, default: bool) {
    let key_gstring: GString = key.into();
    let key_stringname: StringName = key.into();

    if !settings.has_setting(&key_gstring) {
        settings.set_setting(&key_gstring, &Variant::from(default));
    }

    settings.set_initial_value(&key_stringname, &Variant::from(default), false);

    let property_info = vdict! {
        "name": key,
        "type": VariantType::BOOL.ord(),
        "hint": PropertyHint::NONE.ord(),
        "hint_string": "",
    };
    settings.add_property_info(&property_info);
}

/// Registers a color setting with the given key and default value.
fn register_color_setting(settings: &mut Gd<EditorSettings>, key: &str, default: Color) {
    let key_gstring: GString = key.into();
    let key_stringname: StringName = key.into();

    if !settings.has_setting(&key_gstring) {
        settings.set_setting(&key_gstring, &Variant::from(default));
    }

    settings.set_initial_value(&key_stringname, &Variant::from(default), false);

    let property_info = vdict! {
        "name": key,
        "type": VariantType::COLOR.ord(),
        "hint": PropertyHint::NONE.ord(),
        "hint_string": "",
    };
    settings.add_property_info(&property_info);
}

/// Registers an integer setting with range constraints.
fn register_int_setting(
    settings: &mut Gd<EditorSettings>,
    key: &str,
    default: i64,
    min: i64,
    max: i64,
) {
    let key_gstring: GString = key.into();
    let key_stringname: StringName = key.into();

    if !settings.has_setting(&key_gstring) {
        settings.set_setting(&key_gstring, &Variant::from(default));
    }

    settings.set_initial_value(&key_stringname, &Variant::from(default), false);

    let hint_string = format!("{min},{max}");
    let property_info = vdict! {
        "name": key,
        "type": VariantType::INT.ord(),
        "hint": PropertyHint::RANGE.ord(),
        "hint_string": hint_string,
    };
    settings.add_property_info(&property_info);
}

/// Registers an enum setting (displayed as dropdown).
fn register_enum_setting(
    settings: &mut Gd<EditorSettings>,
    key: &str,
    default: i64,
    options: &str, // "Option1,Option2,Option3"
) {
    let key_gstring: GString = key.into();
    let key_stringname: StringName = key.into();

    if !settings.has_setting(&key_gstring) {
        settings.set_setting(&key_gstring, &Variant::from(default));
    }

    settings.set_initial_value(&key_stringname, &Variant::from(default), false);

    let property_info = vdict! {
        "name": key,
        "type": VariantType::INT.ord(),
        "hint": PropertyHint::ENUM.ord(),
        "hint_string": options,
    };
    settings.add_property_info(&property_info);
}

/// Registers a string array setting.
fn register_string_array_setting(settings: &mut Gd<EditorSettings>, key: &str) {
    let key_gstring: GString = key.into();
    let key_stringname: StringName = key.into();

    if !settings.has_setting(&key_gstring) {
        let empty_array = PackedStringArray::new();
        settings.set_setting(&key_gstring, &Variant::from(empty_array));
    }

    let empty_array = PackedStringArray::new();
    settings.set_initial_value(&key_stringname, &Variant::from(empty_array), false);

    let property_info = vdict! {
        "name": key,
        "type": VariantType::PACKED_STRING_ARRAY.ord(),
        "hint": PropertyHint::NONE.ord(),
        "hint_string": "",
    };
    settings.add_property_info(&property_info);
}

/// Registers a dictionary setting for key mappings.
fn register_dictionary_setting(
    settings: &mut Gd<EditorSettings>,
    key: &str,
    default: &godot::builtin::VarDictionary,
) {
    let key_gstring: GString = key.into();
    let key_stringname: StringName = key.into();

    if !settings.has_setting(&key_gstring) {
        settings.set_setting(&key_gstring, &default.to_variant());
    }

    settings.set_initial_value(&key_stringname, &default.to_variant(), false);

    let property_info = vdict! {
        "name": key,
        "type": VariantType::DICTIONARY.ord(),
        "hint": PropertyHint::NONE.ord(),
        "hint_string": "",
    };
    settings.add_property_info(&property_info);
}

/// Registers a generic array setting.
fn register_array_setting(
    settings: &mut Gd<EditorSettings>,
    key: &str,
    default: &godot::builtin::Array<Variant>,
) {
    let key_gstring: GString = key.into();
    let key_stringname: StringName = key.into();

    if !settings.has_setting(&key_gstring) {
        settings.set_setting(&key_gstring, &default.to_variant());
    }

    settings.set_initial_value(&key_stringname, &default.to_variant(), false);

    let property_info = vdict! {
        "name": key,
        "type": VariantType::ARRAY.ord(),
        "hint": PropertyHint::NONE.ord(),
        "hint_string": "",
    };
    settings.add_property_info(&property_info);
}
