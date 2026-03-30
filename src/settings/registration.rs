//! Settings registration -- ensures all GodotVim keys exist in `EditorSettings`
//! with correct default values, types, and property hints.
//!
//! Called once during `enter_tree`. Each setting goes through a three-step
//! Godot `EditorSettings` protocol:
//!
//! 1. **`has_setting` guard + `set_setting`**: only writes the default if the
//!    key is absent, so user customizations in `editor_settings-*.tres` survive
//!    plugin reloads and editor restarts.
//! 2. **`set_initial_value`**: always called (even if the key exists) so Godot
//!    knows what value to show for the "Revert" action in the Inspector.
//! 3. **`add_property_info`**: attaches type/hint metadata so the Inspector
//!    renders the correct widget (slider, enum dropdown, color picker, etc.).

use godot::classes::EditorSettings;
use godot::global::PropertyHint;
use godot::prelude::*;

use super::{defaults, keys};

/// Register all GodotVim settings into `EditorSettings`.
///
/// Idempotent: `has_setting` guards prevent overwriting user customizations,
/// so this is safe to call on every `enter_tree` (e.g., after plugin reload).
pub(crate) fn register_all(settings: &mut EditorSettings) {
    // ── Top-level ────────────────────────────────────────────────────────
    register_enum(
        settings,
        keys::LOG_LEVEL,
        defaults::LOG_LEVEL,
        defaults::LOG_LEVEL_OPTIONS,
    );

    // ── Editor behavior ─────────────────────────────────────────────────
    // tabstop/shiftwidth/expandtab: not registered — synced from Godot's
    // CodeEdit on each editor attach (see plugin/attach.rs).
    register_int_range(settings, keys::SCROLLOFF, defaults::SCROLLOFF, 0, 20);
    register_int_range(settings, keys::TEXTWIDTH, defaults::TEXTWIDTH, 0, 200);
    register_bool(settings, keys::CLIPBOARD_ENABLED, defaults::CLIPBOARD_ENABLED);
    register_bool(settings, keys::IGNORECASE, defaults::IGNORECASE);
    register_bool(settings, keys::SMARTCASE, defaults::SMARTCASE);
    register_enum(
        settings,
        keys::LINE_NUMBER_MODE,
        defaults::LINE_NUMBER_MODE,
        defaults::LINE_NUMBER_MODE_OPTIONS,
    );
    register_enum(
        settings,
        keys::INCCOMMAND,
        defaults::INCCOMMAND,
        defaults::INCCOMMAND_OPTIONS,
    );
    register_int_range(
        settings,
        keys::HIGHLIGHT_YANK_DURATION,
        defaults::HIGHLIGHT_YANK_DURATION,
        0,
        5000,
    );

    // ── Cursor colors ───────────────────────────────────────────────────
    register_color(settings, keys::CURSOR_NORMAL, defaults::cursor_normal());
    register_color(settings, keys::CURSOR_INSERT, defaults::cursor_insert());
    register_color(settings, keys::CURSOR_VISUAL, defaults::cursor_visual());
    register_color(settings, keys::CURSOR_REPLACE, defaults::cursor_replace());
    register_color(settings, keys::CURSOR_OPERATOR, defaults::cursor_operator());
    register_color(settings, keys::CURSOR_COMMAND, defaults::cursor_command());

    // ── Cursor behavior ─────────────────────────────────────────────────────
    register_bool(settings, keys::CURSOR_ENABLED, defaults::CURSOR_ENABLED);
    register_float_range(settings, keys::CURSOR_LERP_SPEED, defaults::CURSOR_LERP_SPEED, 1.0, 100.0, 0.1);
    register_float_range(settings, keys::CURSOR_UNDERLINE_HEIGHT, defaults::CURSOR_UNDERLINE_HEIGHT, 1.0, 10.0, 0.5);

    // ── Key mapping ─────────────────────────────────────────────────────
    register_int_range(settings, keys::TIMEOUTLEN, defaults::TIMEOUTLEN, defaults::TIMEOUTLEN_MIN, defaults::TIMEOUTLEN_MAX);
    register_string(settings, keys::CONFIG_FILE_PATH, defaults::CONFIG_FILE_PATH);

    // ── Input ─────────────────────────────────────────────────────────────
    register_string(settings, keys::PASSTHROUGH_KEYS, defaults::PASSTHROUGH_KEYS);

    // ── Security ─────────────────────────────────────────────────────────
    register_enum(
        settings,
        keys::SHELL_EXECUTION,
        defaults::SHELL_EXECUTION,
        defaults::SHELL_EXECUTION_OPTIONS,
    );
    register_enum(
        settings,
        keys::FILE_ACCESS_SCOPE,
        defaults::FILE_ACCESS_SCOPE,
        defaults::FILE_ACCESS_SCOPE_OPTIONS,
    );
    register_enum(
        settings,
        keys::PROJECT_VIMRC,
        defaults::PROJECT_VIMRC,
        defaults::PROJECT_VIMRC_OPTIONS,
    );

    // ── Status bar colors ─────────────────────────────────────────────────
    register_color(settings, keys::STATUS_BAR_NORMAL_BG, defaults::status_bar_normal_bg());
    register_color(settings, keys::STATUS_BAR_INSERT_BG, defaults::status_bar_insert_bg());
    register_color(settings, keys::STATUS_BAR_VISUAL_BG, defaults::status_bar_visual_bg());
    register_color(settings, keys::STATUS_BAR_REPLACE_BG, defaults::status_bar_replace_bg());
    register_color(settings, keys::STATUS_BAR_COMMAND_BG, defaults::status_bar_command_bg());
    register_color(settings, keys::STATUS_BAR_RECORDING_BG, defaults::status_bar_recording_bg());
    register_color(settings, keys::STATUS_BAR_TEXT_FG, defaults::status_bar_text_fg());
    register_color(settings, keys::STATUS_BAR_ERROR_FG, defaults::status_bar_error_fg());

    log::debug!("settings: registered all EditorSettings keys");
}

// ─────────────────────────────────────────────────────────────────────────────
// Per-type registration helpers
//
// Each helper encodes the three-step protocol (guard + initial + hint) for a
// specific VariantType. They look repetitive, but factoring further would
// obscure the Godot API calls and make debugging registration issues harder.
// ─────────────────────────────────────────────────────────────────────────────

fn register_bool(settings: &mut EditorSettings, key: &str, default: bool) {
    if !settings.has_setting(key) {
        settings.set_setting(key, &default.to_variant());
    }
    settings.set_initial_value(key, &default.to_variant(), false);
    add_property_info(settings, key, VariantType::BOOL, PropertyHint::NONE, "");
}

fn register_int_range(settings: &mut EditorSettings, key: &str, default: i64, min: i64, max: i64) {
    if !settings.has_setting(key) {
        settings.set_setting(key, &default.to_variant());
    }
    settings.set_initial_value(key, &default.to_variant(), false);
    let hint_string = format!("{min},{max},1");
    add_property_info(
        settings,
        key,
        VariantType::INT,
        PropertyHint::RANGE,
        &hint_string,
    );
}

fn register_float_range(
    settings: &mut EditorSettings,
    key: &str,
    default: f64,
    min: f64,
    max: f64,
    step: f64,
) {
    if !settings.has_setting(key) {
        settings.set_setting(key, &default.to_variant());
    }
    settings.set_initial_value(key, &default.to_variant(), false);
    let hint_string = format!("{min},{max},{step}");
    add_property_info(
        settings,
        key,
        VariantType::FLOAT,
        PropertyHint::RANGE,
        &hint_string,
    );
}

/// Register a string-typed setting with an `ENUM` hint dropdown.
///
/// Accepts a `&[&str]` slice (shared with `reader::read_enum_string` via
/// `defaults::*_OPTIONS` constants) and joins it into Godot's comma-separated
/// hint format. This ensures registration and reading always agree on the
/// option order — a mismatch would silently map dropdown indices to wrong labels.
fn register_enum(settings: &mut EditorSettings, key: &str, default: &str, options: &[&str]) {
    if !settings.has_setting(key) {
        settings.set_setting(key, &default.to_variant());
    }
    settings.set_initial_value(key, &default.to_variant(), false);
    let hint_string = options.join(",");
    add_property_info(
        settings,
        key,
        VariantType::STRING,
        PropertyHint::ENUM,
        &hint_string,
    );
}

fn register_string(settings: &mut EditorSettings, key: &str, default: &str) {
    if !settings.has_setting(key) {
        settings.set_setting(key, &default.to_variant());
    }
    settings.set_initial_value(key, &default.to_variant(), false);
    add_property_info(
        settings,
        key,
        VariantType::STRING,
        PropertyHint::NONE,
        "",
    );
}

fn register_color(settings: &mut EditorSettings, key: &str, default: Color) {
    if !settings.has_setting(key) {
        settings.set_setting(key, &default.to_variant());
    }
    settings.set_initial_value(key, &default.to_variant(), false);
    add_property_info(settings, key, VariantType::COLOR, PropertyHint::NONE, "");
}

/// Build and attach the property hint dictionary that Godot's Inspector uses
/// to render the correct widget. The dictionary schema (`name`, `type`,
/// `hint`, `hint_string`) mirrors `PropertyInfo` in Godot's C++ API.
fn add_property_info(
    settings: &mut EditorSettings,
    key: &str,
    variant_type: VariantType,
    hint: PropertyHint,
    hint_string: &str,
) {
    let mut info = VarDictionary::new();
    info.set("name", key);
    info.set("type", variant_type.ord() as i64);
    info.set("hint", hint.ord() as i64);
    info.set("hint_string", hint_string);
    settings.add_property_info(&info);
}
