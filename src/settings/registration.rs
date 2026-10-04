//! Settings registration -- ensures all GodotVim keys exist in `EditorSettings`
//! with correct default values, types, and property hints.
//!
//! Called once during `enter_tree`. Each setting goes through a three-step
//! Godot `EditorSettings` protocol:
//!
//! 1. **Untouched guard + `set_setting`**: writes the default only if the
//!    user never changed the value (see [`is_untouched`]), so user
//!    customizations in `editor_settings-*.tres` survive plugin reloads and
//!    editor restarts, while a changed default reaches everyone else.
//!    Godot stores every value of a setting that has a property hint, the
//!    old default included, so a changed default names its predecessor (see
//!    [`legacy_defaults`]) and [`SETTINGS_REVISION`] records that the stored
//!    values have been moved past it.
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
/// Idempotent: the untouched guard never overwrites a user customization,
/// so this is safe to call on every `enter_tree` (e.g., after plugin reload).
pub(crate) fn register_all(settings: &mut EditorSettings) {
    let revision = stored_revision(settings);

    // ── Top-level ────────────────────────────────────────────────────────
    register_enum(
        settings,
        keys::LOG_LEVEL,
        defaults::LOG_LEVEL,
        defaults::LOG_LEVEL_OPTIONS,
    );
    register_bool(settings, keys::ENABLED, defaults::ENABLED);

    // ── Editor behavior ─────────────────────────────────────────────────
    // tabstop/shiftwidth/expandtab: not registered — synced from Godot's
    // CodeEdit on each editor attach (see plugin/attach.rs).
    register_int_range(settings, keys::SCROLLOFF, defaults::SCROLLOFF, 0, 20);
    register_int_range_from(
        settings,
        keys::TEXTWIDTH,
        defaults::TEXTWIDTH,
        0,
        200,
        legacy_defaults(revision, 1, &[defaults::TEXTWIDTH_UNTIL_1_8_0]),
    );
    register_bool(
        settings,
        keys::CLIPBOARD_ENABLED,
        defaults::CLIPBOARD_ENABLED,
    );
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
    register_float_range_hinted(
        settings,
        keys::CURSOR_LERP_SPEED,
        defaults::CURSOR_LERP_SPEED,
        defaults::CURSOR_LERP_SPEED_MIN,
        defaults::CURSOR_LERP_SPEED_MAX,
        0.1,
        ",or_greater,exp",
    );
    register_float_range(
        settings,
        keys::CURSOR_UNDERLINE_HEIGHT,
        defaults::CURSOR_UNDERLINE_HEIGHT,
        1.0,
        10.0,
        0.5,
    );

    // ── Key mapping ─────────────────────────────────────────────────────
    register_int_range(
        settings,
        keys::TIMEOUTLEN,
        defaults::TIMEOUTLEN,
        defaults::TIMEOUTLEN_MIN,
        defaults::TIMEOUTLEN_MAX,
    );
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
    register_color(
        settings,
        keys::STATUS_BAR_NORMAL_BG,
        defaults::status_bar_normal_bg(),
    );
    register_color(
        settings,
        keys::STATUS_BAR_INSERT_BG,
        defaults::status_bar_insert_bg(),
    );
    register_color(
        settings,
        keys::STATUS_BAR_VISUAL_BG,
        defaults::status_bar_visual_bg(),
    );
    register_color(
        settings,
        keys::STATUS_BAR_REPLACE_BG,
        defaults::status_bar_replace_bg(),
    );
    register_color(
        settings,
        keys::STATUS_BAR_COMMAND_BG,
        defaults::status_bar_command_bg(),
    );
    register_color(
        settings,
        keys::STATUS_BAR_RECORDING_BG,
        defaults::status_bar_recording_bg(),
    );
    register_color(
        settings,
        keys::STATUS_BAR_TEXT_FG,
        defaults::status_bar_text_fg(),
    );
    register_color(
        settings,
        keys::STATUS_BAR_ERROR_FG,
        defaults::status_bar_error_fg(),
    );

    if revision < SETTINGS_REVISION {
        settings.set_setting(keys::SETTINGS_REVISION, &SETTINGS_REVISION.to_variant());
    }

    log::debug!("settings: registered all EditorSettings keys");
}

// ─────────────────────────────────────────────────────────────────────────────
// Changed defaults
//
// Godot saves a setting whenever it has a property hint, even when the value
// equals its initial value: `_get_property_list` replaces the usage it worked
// out, STORAGE dropped for an unchanged value, with the hint's own
// PropertyInfo, whose default usage includes STORAGE. Every GodotVim setting
// has a hint, so every old default is in the user's `.tres`, loaded before
// registration and, on a cold start, with no revert value yet. Such a value
// can only be recognised by naming the old default, and only once: after the
// move, the same value is a choice the user made.
// ─────────────────────────────────────────────────────────────────────────────

/// The revision the stored settings are at once this version has registered.
/// A stored revision below a setting's `changed_in` means its old default may
/// still be stored. A missing key reads as 0, which is every release up to
/// 1.8.0.
///
/// - 1: Textwidth's default moved from 80 to 0.
const SETTINGS_REVISION: i64 = 1;

/// The revision recorded in `EditorSettings`, or 0 when there is none.
fn stored_revision(settings: &EditorSettings) -> i64 {
    if !settings.has_setting(keys::SETTINGS_REVISION) {
        return 0;
    }
    settings
        .get(keys::SETTINGS_REVISION)
        .try_to::<i64>()
        .unwrap_or(0)
}

/// The old defaults that count as untouched for a setting whose default
/// changed in revision `changed_in`: `old` while the stored revision is
/// below it, and nothing afterwards, so that a user who sets the old default
/// again after the move keeps it.
fn legacy_defaults<T>(revision: i64, changed_in: i64, old: &[T]) -> &[T] {
    if revision < changed_in {
        old
    } else {
        &[]
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Per-type registration helpers
//
// Each helper encodes the three-step protocol (guard + initial + hint) for a
// specific VariantType. Steps 1 and 2 are shared by `seed_default`, because
// the untouched decision must be identical for every type; the hints stay
// inline, since factoring them further would obscure the Godot API calls and
// make debugging registration issues harder.
// ─────────────────────────────────────────────────────────────────────────────

/// Whether a setting still holds a default, and so may be moved to the
/// current one.
///
/// - Missing key: untouched. This is a fresh install.
/// - Current value equals the revert value: untouched. Godot holds a revert
///   value only after `set_initial_value`, so this arm fires when the plugin
///   registers again within one editor session, for example when it is
///   disabled and enabled again under Project Settings > Plugins after a
///   new version was loaded. Reloading the extension alone does not
///   register again.
/// - Current value is one of `legacy`, the setting's previous defaults while
///   they have not been migrated (see [`legacy_defaults`]): untouched. This
///   is the cold start after an update, where the old default was loaded
///   from the `.tres` and there is no revert value yet.
/// - Anything else is the user's: a different value, or a value of another
///   Variant type than the default (`None` here). An enum dropdown stores an
///   INT ordinal where the default is a STRING, and a hand-edited file can
///   hold anything; neither is ours to rewrite.
///
/// A user who deliberately picked exactly an old default cannot be told
/// apart from one who never touched it, and moves with the default once.
fn is_untouched<T: PartialEq>(
    exists: bool,
    current: Option<&T>,
    revert: Option<&T>,
    legacy: &[T],
) -> bool {
    if !exists {
        return true;
    }
    let Some(current) = current else {
        return false;
    };
    revert == Some(current) || legacy.contains(current)
}

/// Steps 1 and 2 of the protocol for every helper: write `default` unless
/// the user owns the value, then make it the revert value. `legacy` lists
/// the old defaults that still count as untouched.
fn seed_default<T>(settings: &mut EditorSettings, key: &str, default: T, legacy: &[T])
where
    T: ToGodot + FromGodot + PartialEq,
{
    let exists = settings.has_setting(key);
    // `get`, not `get_setting`: `get_setting` returns a project's editor
    // setting override when one exists, while `set_setting` below writes the
    // user's own stored value. The decision must be made on what it writes.
    let current = if exists {
        settings.get(key).try_to::<T>().ok()
    } else {
        None
    };
    let revert = if settings.property_can_revert(key) {
        settings.property_get_revert(key).try_to::<T>().ok()
    } else {
        None
    };
    let default = default.to_variant();
    if is_untouched(exists, current.as_ref(), revert.as_ref(), legacy) {
        settings.set_setting(key, &default);
    }
    settings.set_initial_value(key, &default, false);
}

fn register_bool(settings: &mut EditorSettings, key: &str, default: bool) {
    seed_default(settings, key, default, &[]);
    add_property_info(settings, key, VariantType::BOOL, PropertyHint::NONE, "");
}

fn register_int_range(settings: &mut EditorSettings, key: &str, default: i64, min: i64, max: i64) {
    register_int_range_from(settings, key, default, min, max, &[]);
}

/// [`register_int_range`] for a setting whose default changed: a stored value
/// in `legacy` is moved to `default`.
fn register_int_range_from(
    settings: &mut EditorSettings,
    key: &str,
    default: i64,
    min: i64,
    max: i64,
    legacy: &[i64],
) {
    seed_default(settings, key, default, legacy);
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
    register_float_range_hinted(settings, key, default, min, max, step, "");
}

/// `flags` appends Godot RANGE modifiers, e.g. `",or_greater,exp"`.
///
/// `exp` requires `min > 0`: the slider's exp_ratio mapping is uniform in
/// log2(value), and a min of 0 collapses the whole lower half of the bar onto
/// one value.
fn register_float_range_hinted(
    settings: &mut EditorSettings,
    key: &str,
    default: f64,
    min: f64,
    max: f64,
    step: f64,
    flags: &str,
) {
    seed_default(settings, key, default, &[]);
    let hint_string = format!("{min},{max},{step}{flags}");
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
    seed_default(settings, key, GString::from(default), &[]);
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
    seed_default(settings, key, GString::from(default), &[]);
    add_property_info(settings, key, VariantType::STRING, PropertyHint::NONE, "");
}

fn register_color(settings: &mut EditorSettings, key: &str, default: Color) {
    seed_default(settings, key, default, &[]);
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

#[cfg(test)]
mod tests {
    use super::{is_untouched, legacy_defaults};

    #[test]
    fn missing_key_is_untouched() {
        assert!(is_untouched::<i64>(false, None, None, &[]));
        // A missing key has nothing stored, whatever revert value Godot holds.
        assert!(is_untouched(false, None, Some(&80_i64), &[]));
    }

    #[test]
    fn value_equal_to_revert_is_untouched() {
        // Registering again in the same session: the previous registration's
        // default is still in memory as value and initial value.
        assert!(is_untouched(true, Some(&80_i64), Some(&80_i64), &[]));
    }

    #[test]
    fn user_value_is_kept() {
        assert!(!is_untouched(true, Some(&100_i64), Some(&80_i64), &[]));
        // A value the user set to what is now the new default is still theirs.
        assert!(!is_untouched(true, Some(&0_i64), Some(&80_i64), &[]));
    }

    /// Issue #77: the cold start after an update. Godot stored the old
    /// default, because the setting has a property hint, and loaded it
    /// before registration, so there is no revert value yet.
    #[test]
    fn stored_legacy_default_without_revert_is_untouched() {
        assert!(is_untouched(true, Some(&80_i64), None, &[80]));
    }

    #[test]
    fn stored_value_without_revert_is_kept() {
        assert!(!is_untouched(true, Some(&100_i64), None, &[80]));
        assert!(!is_untouched(true, Some(&80_i64), None, &[]));
    }

    #[test]
    fn variant_type_mismatch_is_kept() {
        // The stored Variant did not convert to the default's type, for
        // example an enum dropdown's INT ordinal against a STRING default.
        assert!(!is_untouched(true, None, Some(&"Hybrid"), &[]));
        assert!(!is_untouched::<i64>(true, None, None, &[80]));
    }

    #[test]
    fn legacy_defaults_apply_until_the_revision_that_moved_them() {
        // No revision stored: every release up to 1.8.0.
        assert_eq!(legacy_defaults(0, 1, &[80_i64]), &[80]);
        // Already migrated: an 80 now is the user's choice.
        assert!(legacy_defaults(1, 1, &[80_i64]).is_empty());
        assert!(legacy_defaults(2, 1, &[80_i64]).is_empty());
    }

    /// The whole decision across two cold starts: the first moves an old
    /// stored 80, the second keeps an 80 the user set after that.
    #[test]
    fn a_deliberate_old_default_survives_after_the_migration() {
        let first = legacy_defaults(0, 1, &[80_i64]);
        assert!(is_untouched(true, Some(&80_i64), None, first));
        let later = legacy_defaults(super::SETTINGS_REVISION, 1, &[80_i64]);
        assert!(!is_untouched(true, Some(&80_i64), None, later));
    }
}
