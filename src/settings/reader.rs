//! Settings reader — reads all GodotVim settings from `EditorSettings` into a
//! typed Rust snapshot, with fallback to defaults on missing or wrong-type values.

use godot::classes::EditorSettings;
use godot::prelude::*;

use vim_core::keymap::KeyEvent;

use super::{
    defaults, keys, CursorSettings, FileAccessScope, InccommandMode, LineNumberMode, ProjectVimrc,
    SettingsSnapshot, ShellExecution, StatusBarColors,
};

/// Read all GodotVim settings from `EditorSettings` into a typed [`SettingsSnapshot`].
///
/// Every field is read independently with its own fallback, so a single
/// corrupt or missing key never poisons the rest of the snapshot. This is
/// important because users can hand-edit `editor_settings-*.tres` or
/// downgrade the plugin, leaving stale/missing keys behind.
pub(crate) fn read_all(settings: &EditorSettings) -> SettingsSnapshot {
    SettingsSnapshot {
        log_level: crate::logging::LogLevel::from_setting(&read_enum_string(
            settings,
            keys::LOG_LEVEL,
            defaults::LOG_LEVEL,
            defaults::LOG_LEVEL_OPTIONS,
        )),
        enabled: read_bool(settings, keys::ENABLED, defaults::ENABLED),
        scrolloff: read_int(settings, keys::SCROLLOFF, defaults::SCROLLOFF),
        textwidth: read_int(settings, keys::TEXTWIDTH, defaults::TEXTWIDTH),
        clipboard_enabled: read_bool(
            settings,
            keys::CLIPBOARD_ENABLED,
            defaults::CLIPBOARD_ENABLED,
        ),
        ignorecase: read_bool(settings, keys::IGNORECASE, defaults::IGNORECASE),
        smartcase: read_bool(settings, keys::SMARTCASE, defaults::SMARTCASE),
        code_complete_enabled: read_bool(
            settings,
            keys::CODE_COMPLETE_ENABLED,
            defaults::CODE_COMPLETE_ENABLED,
        ),
        line_number_mode: read_line_number_mode(settings),
        inccommand: read_inccommand(settings),
        highlight_yank_duration: read_int(
            settings,
            keys::HIGHLIGHT_YANK_DURATION,
            defaults::HIGHLIGHT_YANK_DURATION,
        )
        .max(0)
        .min(u32::MAX as i64) as u32,
        cursor: CursorSettings {
            normal: read_color(settings, keys::CURSOR_NORMAL, defaults::cursor_normal()),
            insert: read_color(settings, keys::CURSOR_INSERT, defaults::cursor_insert()),
            visual: read_color(settings, keys::CURSOR_VISUAL, defaults::cursor_visual()),
            replace: read_color(settings, keys::CURSOR_REPLACE, defaults::cursor_replace()),
            operator: read_color(settings, keys::CURSOR_OPERATOR, defaults::cursor_operator()),
            command: read_color(settings, keys::CURSOR_COMMAND, defaults::cursor_command()),
            enabled: read_bool(settings, keys::CURSOR_ENABLED, defaults::CURSOR_ENABLED),
            lerp_speed: read_float(
                settings,
                keys::CURSOR_LERP_SPEED,
                defaults::CURSOR_LERP_SPEED,
            ),
            underline_height: read_float(
                settings,
                keys::CURSOR_UNDERLINE_HEIGHT,
                defaults::CURSOR_UNDERLINE_HEIGHT,
            ),
        },
        status_bar: StatusBarColors {
            normal_bg: read_color(
                settings,
                keys::STATUS_BAR_NORMAL_BG,
                defaults::status_bar_normal_bg(),
            ),
            insert_bg: read_color(
                settings,
                keys::STATUS_BAR_INSERT_BG,
                defaults::status_bar_insert_bg(),
            ),
            visual_bg: read_color(
                settings,
                keys::STATUS_BAR_VISUAL_BG,
                defaults::status_bar_visual_bg(),
            ),
            replace_bg: read_color(
                settings,
                keys::STATUS_BAR_REPLACE_BG,
                defaults::status_bar_replace_bg(),
            ),
            command_bg: read_color(
                settings,
                keys::STATUS_BAR_COMMAND_BG,
                defaults::status_bar_command_bg(),
            ),
            recording_bg: read_color(
                settings,
                keys::STATUS_BAR_RECORDING_BG,
                defaults::status_bar_recording_bg(),
            ),
            text_fg: read_color(
                settings,
                keys::STATUS_BAR_TEXT_FG,
                defaults::status_bar_text_fg(),
            ),
            error_fg: read_color(
                settings,
                keys::STATUS_BAR_ERROR_FG,
                defaults::status_bar_error_fg(),
            ),
        },
        timeoutlen: read_int(settings, keys::TIMEOUTLEN, defaults::TIMEOUTLEN),
        passthrough_keys: read_passthrough_keys(settings),
        config_file_path: read_string(settings, keys::CONFIG_FILE_PATH, defaults::CONFIG_FILE_PATH),
        shell_execution: read_shell_execution(settings),
        file_access_scope: read_file_access_scope(settings),
        project_vimrc: read_project_vimrc(settings),
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Per-type reading helpers
// ─────────────────────────────────────────────────────────────────────────────

/// Generic typed read with fallback. The `type_name` parameter is only used
/// in the warning log message — Godot's `Variant` doesn't expose its type
/// name in a way that's useful for diagnostics.
fn read_setting<T: FromGodot>(
    settings: &EditorSettings,
    key: &str,
    type_name: &str,
    default: T,
) -> T {
    if !settings.has_setting(key) {
        return default;
    }
    let variant = settings.get_setting(key);
    match variant.try_to::<T>() {
        Ok(v) => v,
        Err(_) => {
            log::warn!(
                "Setting '{}' has unexpected type (expected {}), using default",
                key,
                type_name,
            );
            default
        }
    }
}

fn read_bool(settings: &EditorSettings, key: &str, default: bool) -> bool {
    read_setting(settings, key, "bool", default)
}

fn read_int(settings: &EditorSettings, key: &str, default: i64) -> i64 {
    read_setting(settings, key, "int", default)
}

fn read_float(settings: &EditorSettings, key: &str, default: f64) -> f64 {
    read_setting::<f64>(settings, key, "float", default)
}

fn read_color(settings: &EditorSettings, key: &str, default: Color) -> Color {
    read_setting(settings, key, "Color", default)
}

fn read_string(settings: &EditorSettings, key: &str, default: &str) -> String {
    read_enum_string(settings, key, default, &[])
}

/// Read a string setting that may be stored as an integer index by Godot.
///
/// **Why this exists:** Godot's `PropertyHint::ENUM` dropdowns silently store
/// the user's selection as an `INT` ordinal, not the `STRING` label -- even
/// when the setting was originally registered as a string via `set_setting`.
/// On first launch the value is a string; after the user touches the dropdown
/// it becomes an int. Both representations must round-trip correctly.
///
/// - **String variant**: returned directly (first-launch or hand-edited `.tres`).
/// - **Int variant**: mapped back to the label via `options` (must match the
///   comma-separated list passed to `register_enum`). Out-of-range indices
///   fall through to the default.
///
/// `options` may be empty for free-form string settings (passthrough keys,
/// config file path), in which case only the string path is attempted.
fn read_enum_string(
    settings: &EditorSettings,
    key: &str,
    default: &str,
    options: &[&str],
) -> String {
    if !settings.has_setting(key) {
        return default.to_owned();
    }
    let variant = settings.get_setting(key);

    // Godot's ENUM dropdowns silently convert STRING values to INT ordinals
    // after the user touches the dropdown. We must check the variant's actual
    // type first — `try_to::<GString>()` would succeed on an INT via Godot's
    // implicit conversion, returning "5" instead of "Trace".
    if variant.get_type() == godot::builtin::VariantType::INT && !options.is_empty() {
        if let Ok(idx) = variant.try_to::<i64>() {
            if let Some(label) = usize::try_from(idx).ok().and_then(|i| options.get(i)) {
                return (*label).to_owned();
            }
        }
    }

    // String path: first launch (value is still a string) or hand-edited .tres.
    if let Ok(v) = variant.try_to::<GString>() {
        return v.to_string();
    }

    log::warn!(
        "Setting '{}' has unexpected type (expected string), using default",
        key,
    );
    default.to_owned()
}

fn read_line_number_mode(settings: &EditorSettings) -> LineNumberMode {
    read_parsed_enum(
        settings,
        keys::LINE_NUMBER_MODE,
        defaults::LINE_NUMBER_MODE,
        defaults::LINE_NUMBER_MODE_OPTIONS,
        |s| match s {
            "None" => Some(LineNumberMode::None),
            "Absolute" => Some(LineNumberMode::Absolute),
            "Relative" => Some(LineNumberMode::Relative),
            "Hybrid" => Some(LineNumberMode::Hybrid),
            _ => None,
        },
    )
}

/// Parse the comma-separated passthrough key list into `Vec<KeyEvent>`.
/// Invalid tokens are warned and skipped so one typo doesn't break all passthrough.
fn read_passthrough_keys(settings: &EditorSettings) -> Vec<KeyEvent> {
    let raw = read_string(settings, keys::PASSTHROUGH_KEYS, defaults::PASSTHROUGH_KEYS);
    if raw.trim().is_empty() {
        return Vec::new();
    }
    raw.split(',')
        .filter_map(|token| {
            let token = token.trim();
            if token.is_empty() {
                return None;
            }
            match KeyEvent::from_vim_notation(token) {
                Some(key) => Some(key),
                None => {
                    log::warn!(
                        "Ignoring unrecognized passthrough key '{}' in setting '{}'",
                        token,
                        keys::PASSTHROUGH_KEYS,
                    );
                    None
                }
            }
        })
        .collect()
}

fn read_inccommand(settings: &EditorSettings) -> InccommandMode {
    read_parsed_enum(
        settings,
        keys::INCCOMMAND,
        defaults::INCCOMMAND,
        defaults::INCCOMMAND_OPTIONS,
        |s| match s {
            "nosplit" => Some(InccommandMode::Nosplit),
            "off" => Some(InccommandMode::Off),
            _ => None,
        },
    )
}

/// Read a string-backed enum setting, parsing via `parse` after handling
/// Godot's int-ordinal storage quirk (see [`read_enum_string`]).
/// Returns `T::default()` with a warning on unrecognized values.
fn read_parsed_enum<T: Default>(
    settings: &EditorSettings,
    key: &str,
    default_str: &str,
    options: &[&str],
    parse: impl Fn(&str) -> Option<T>,
) -> T {
    let value = read_enum_string(settings, key, default_str, options);
    parse(value.as_str()).unwrap_or_else(|| {
        log::warn!(
            "Setting '{}' has unrecognized value '{}', using default: {}",
            key,
            value,
            default_str
        );
        T::default()
    })
}

fn read_shell_execution(settings: &EditorSettings) -> ShellExecution {
    read_parsed_enum(
        settings,
        keys::SHELL_EXECUTION,
        defaults::SHELL_EXECUTION,
        defaults::SHELL_EXECUTION_OPTIONS,
        |s| match s {
            "Enabled" => Some(ShellExecution::Enabled),
            "Disabled" => Some(ShellExecution::Disabled),
            _ => None,
        },
    )
}

fn read_file_access_scope(settings: &EditorSettings) -> FileAccessScope {
    read_parsed_enum(
        settings,
        keys::FILE_ACCESS_SCOPE,
        defaults::FILE_ACCESS_SCOPE,
        defaults::FILE_ACCESS_SCOPE_OPTIONS,
        |s| match s.replace(' ', "").as_str() {
            "ProjectOnly" => Some(FileAccessScope::ProjectOnly),
            "Unrestricted" => Some(FileAccessScope::Unrestricted),
            _ => None,
        },
    )
}

fn read_project_vimrc(settings: &EditorSettings) -> ProjectVimrc {
    read_parsed_enum(
        settings,
        keys::PROJECT_VIMRC,
        defaults::PROJECT_VIMRC,
        defaults::PROJECT_VIMRC_OPTIONS,
        |s| match s {
            "Disabled" => Some(ProjectVimrc::Disabled),
            "Sandbox" => Some(ProjectVimrc::Sandbox),
            "Trusted" => Some(ProjectVimrc::Trusted),
            _ => None,
        },
    )
}
