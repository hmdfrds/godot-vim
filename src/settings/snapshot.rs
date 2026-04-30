//! Typed settings snapshot and configuration enums.
//!
//! [`SettingsSnapshot`] is an atomic, fully-validated view of all user settings
//! at a single point in time. It is constructed once per settings-change event
//! by [`super::reader::read_all`] and then pushed immutably to the engine and
//! UI layer. This snapshot-based design avoids partial reads where some fields
//! reflect pre-change values and others reflect post-change values.
//!
//! Configuration enums ([`LineNumberMode`], [`ShellExecution`], etc.) live here
//! rather than in the UI or engine because they describe user *preferences*
//! that flow from settings to consumers, not the reverse.

use godot::prelude::*;
use vim_core::keymap::KeyEvent;

// ─────────────────────────────────────────────────────────────────────────────
// Security enums
// ─────────────────────────────────────────────────────────────────────────────

/// Controls whether `:!` and `:{range}!` shell execution is allowed.
/// Defaults to `Disabled` -- users must explicitly opt in via Editor Settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum ShellExecution {
    #[default]
    Disabled,
    Enabled,
}

/// Controls which filesystem paths `:w`, `:r`, `:e` can access.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum FileAccessScope {
    /// Only `res://` and `user://` Godot virtual paths are allowed.
    #[default]
    ProjectOnly,
    Unrestricted,
}

/// Controls how `res://.godot-vimrc` (project-level config) is treated.
///
/// Project-level vimrc is a security concern because cloning a repo could
/// execute arbitrary shell commands. `Sandbox` strips `:!` from mappings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum ProjectVimrc {
    Disabled,
    /// Loads the vimrc but strips shell-invoking mappings (`:!` in RHS).
    #[default]
    Sandbox,
    Trusted,
}

// ─────────────────────────────────────────────────────────────────────────────
// LineNumberMode
// ─────────────────────────────────────────────────────────────────────────────

/// Display mode for line numbers in the gutter.
///
/// - `Absolute`: standard 1-based line numbers (like Vim `set number`).
/// - `Relative`: distance from the cursor line (like Vim `set relativenumber`).
/// - `Hybrid`: current line shows its absolute number, all others show relative
///   distance (like Vim `set number relativenumber`).
/// - `None`: no line numbers displayed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum LineNumberMode {
    None,
    Absolute,
    Relative,
    #[default]
    Hybrid,
}

// ─────────────────────────────────────────────────────────────────────────────
// InccommandMode
// ─────────────────────────────────────────────────────────────────────────────

/// Live-preview mode for substitute commands (Vim `inccommand` option).
///
/// - `Off`: no live preview (like Vim `set inccommand=` / empty string).
/// - `Nosplit`: preview substitutions inline without opening a split (like Vim
///   `set inccommand=nosplit`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum InccommandMode {
    Off,
    #[default]
    Nosplit,
}

impl InccommandMode {
    #[inline]
    #[must_use]
    pub(crate) fn is_enabled(self) -> bool {
        self != Self::Off
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// CursorSettings
// ─────────────────────────────────────────────────────────────────────────────

/// Cursor appearance and animation settings, grouped to keep
/// [`SettingsSnapshot`] from becoming a 30-field flat struct.
#[derive(Debug, Clone)]
pub(crate) struct CursorSettings {
    // ── Per-mode colors ─────────────────────────────────────────────────
    pub(crate) normal: Color,
    pub(crate) insert: Color,
    pub(crate) visual: Color,
    pub(crate) replace: Color,
    pub(crate) operator: Color,
    pub(crate) command: Color,

    // ── Dimensions / animation ──────────────────────────────────────────
    pub(crate) lerp_speed: f64,
    pub(crate) underline_height: f64,

    // ── Toggles ─────────────────────────────────────────────────────────
    pub(crate) enabled: bool,
}

// ─────────────────────────────────────────────────────────────────────────────
// StatusBarColors
// ─────────────────────────────────────────────────────────────────────────────

/// Status bar colors, one background per Vim mode plus shared foregrounds.
#[derive(Debug, Clone)]
pub(crate) struct StatusBarColors {
    pub(crate) normal_bg: Color,
    pub(crate) insert_bg: Color,
    pub(crate) visual_bg: Color,
    pub(crate) replace_bg: Color,
    pub(crate) command_bg: Color,
    pub(crate) recording_bg: Color,
    pub(crate) text_fg: Color,
    pub(crate) error_fg: Color,
}

// ─────────────────────────────────────────────────────────────────────────────
// SettingsSnapshot
// ─────────────────────────────────────────────────────────────────────────────

/// Immutable, point-in-time view of all user settings.
///
/// Constructed atomically by [`reader::read_all`] on every settings-change
/// event, then pushed to the engine and UI. Fields are ordered by alignment
/// (8-byte, 4-byte, 1-byte) to minimize struct padding.
#[derive(Debug, Clone)]
pub(crate) struct SettingsSnapshot {
    // ── 8-byte aligned ──────────────────────────────────────────────────
    pub(crate) scrolloff: i64,
    pub(crate) textwidth: i64,
    pub(crate) timeoutlen: i64,
    /// Pre-parsed from the comma-separated setting string so the input
    /// handler can do O(n) lookup without re-parsing on every keystroke.
    pub(crate) passthrough_keys: Vec<KeyEvent>,
    /// Empty string triggers auto-resolution: `res://.godot-vimrc` first,
    /// then `user://.godot-vimrc`.
    pub(crate) config_file_path: String,

    // ── Composite structs ───────────────────────────────────────────────
    pub(crate) cursor: CursorSettings,
    pub(crate) status_bar: StatusBarColors,

    // ── 4-byte aligned ──────────────────────────────────────────────────
    pub(crate) highlight_yank_duration: u32,

    // ── 1-byte aligned (bool + small enums) ─────────────────────────────
    pub(crate) clipboard_enabled: bool,
    pub(crate) ignorecase: bool,
    pub(crate) smartcase: bool,
    /// Whether Godot's native code completion should auto-trigger on typing.
    /// Read from `text_editor/completion/code_complete_enabled` (native Godot
    /// EditorSetting, not registered by GodotVim).
    pub(crate) code_complete_enabled: bool,
    pub(crate) inccommand: InccommandMode,
    pub(crate) log_level: crate::logging::LogLevel,
    pub(crate) line_number_mode: LineNumberMode,
    pub(crate) shell_execution: ShellExecution,
    pub(crate) file_access_scope: FileAccessScope,
    pub(crate) project_vimrc: ProjectVimrc,
}

impl SettingsSnapshot {
    /// Push snapshot values into the engine's `VimOptions`.
    ///
    /// **Mutates in place** rather than replacing `VimOptions` wholesale,
    /// because `VimOptions` also contains indent settings (`expandtab`,
    /// `tabstop`, `shiftwidth`) and `commentstring` that are synced from
    /// Godot's CodeEdit on attach -- not from Editor Settings. Replacing
    /// the whole struct would clobber those per-editor values.
    pub(crate) fn apply_to_options(&self, opts: &mut vim_core::VimOptions) {
        use super::defaults;
        opts.set_scrolloff(usize::try_from(self.scrolloff.max(0)).unwrap_or(0));
        opts.set_textwidth(usize::try_from(self.textwidth.max(0)).unwrap_or(0));
        opts.set_timeoutlen_ms(
            u32::try_from(
                self.timeoutlen
                    .clamp(defaults::TIMEOUTLEN_MIN, defaults::TIMEOUTLEN_MAX),
            )
            .unwrap_or(u32::MAX),
        );
        if self.clipboard_enabled {
            opts.set_clipboard("unnamedplus");
        } else {
            opts.set_clipboard("");
        }
        opts.set_inccommand(match self.inccommand {
            InccommandMode::Nosplit => "nosplit",
            InccommandMode::Off => "",
        });
        opts.set_ignorecase(self.ignorecase);
        opts.set_smartcase(self.smartcase);
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a snapshot with parametrized numeric fields; everything else
    /// uses hardcoded defaults to isolate what each test is verifying.
    fn make_snapshot(scrolloff: i64, textwidth: i64, timeoutlen: i64) -> SettingsSnapshot {
        SettingsSnapshot {
            log_level: crate::logging::LogLevel::Info,
            scrolloff,
            textwidth,
            clipboard_enabled: false,
            ignorecase: false,
            smartcase: false,
            code_complete_enabled: true,
            line_number_mode: LineNumberMode::Hybrid,
            inccommand: InccommandMode::Nosplit,
            highlight_yank_duration: 150,
            cursor: CursorSettings {
                normal: Color::WHITE,
                insert: Color::WHITE,
                visual: Color::WHITE,
                replace: Color::WHITE,
                operator: Color::WHITE,
                command: Color::WHITE,
                enabled: true,
                lerp_speed: 25.0,
                underline_height: 4.0,
            },
            status_bar: StatusBarColors {
                normal_bg: Color::from_rgb(0.5, 0.6, 0.8),
                insert_bg: Color::from_rgb(0.6, 0.8, 0.5),
                visual_bg: Color::from_rgb(0.8, 0.5, 0.5),
                replace_bg: Color::from_rgb(0.9, 0.6, 0.3),
                command_bg: Color::from_rgb(0.157, 0.173, 0.204),
                recording_bg: Color::from_rgb(0.9, 0.2, 0.2),
                text_fg: Color::WHITE,
                error_fg: Color::from_rgb(1.0, 0.3, 0.3),
            },
            timeoutlen,
            passthrough_keys: Vec::new(),
            config_file_path: String::new(),
            shell_execution: ShellExecution::Disabled,
            file_access_scope: FileAccessScope::ProjectOnly,
            project_vimrc: ProjectVimrc::Sandbox,
        }
    }

    #[test]
    fn typical_values_map_correctly() {
        let snap = make_snapshot(5, 80, 1000);
        let mut opts = vim_core::VimOptions::default();
        snap.apply_to_options(&mut opts);

        assert_eq!(opts.scrolloff(), 5);
        assert_eq!(opts.textwidth(), 80);
        assert_eq!(opts.timeoutlen_ms(), 1000);
    }

    #[test]
    fn custom_values_map_correctly() {
        let snap = make_snapshot(10, 120, 500);
        let mut opts = vim_core::VimOptions::default();
        snap.apply_to_options(&mut opts);

        assert_eq!(opts.scrolloff(), 10);
        assert_eq!(opts.textwidth(), 120);
        assert_eq!(opts.timeoutlen_ms(), 500);
    }

    // ── Clamping behavior ─────────────────────────────────────────────────
    // tabstop/shiftwidth are synced from Godot's CodeEdit, not from the
    // snapshot, so clamping tests for those live elsewhere.

    #[test]
    fn scrolloff_zero_is_valid() {
        let snap = make_snapshot(0, 80, 1000);
        let mut opts = vim_core::VimOptions::default();
        snap.apply_to_options(&mut opts);
        assert_eq!(opts.scrolloff(), 0);
    }

    #[test]
    fn scrolloff_negative_clamped_to_zero() {
        let snap = make_snapshot(-3, 80, 1000);
        let mut opts = vim_core::VimOptions::default();
        snap.apply_to_options(&mut opts);
        assert_eq!(opts.scrolloff(), 0);
    }

    #[test]
    fn textwidth_zero_means_no_limit() {
        let snap = make_snapshot(5, 0, 1000);
        let mut opts = vim_core::VimOptions::default();
        snap.apply_to_options(&mut opts);
        assert_eq!(opts.textwidth(), 0);
    }

    #[test]
    fn textwidth_negative_clamped_to_zero() {
        let snap = make_snapshot(5, -10, 1000);
        let mut opts = vim_core::VimOptions::default();
        snap.apply_to_options(&mut opts);
        assert_eq!(opts.textwidth(), 0);
    }

    #[test]
    fn timeoutlen_at_minimum_boundary() {
        let snap = make_snapshot(5, 80, 100);
        let mut opts = vim_core::VimOptions::default();
        snap.apply_to_options(&mut opts);
        assert_eq!(opts.timeoutlen_ms(), 100);
    }

    #[test]
    fn timeoutlen_at_maximum_boundary() {
        let snap = make_snapshot(5, 80, 5000);
        let mut opts = vim_core::VimOptions::default();
        snap.apply_to_options(&mut opts);
        assert_eq!(opts.timeoutlen_ms(), 5000);
    }

    #[test]
    fn timeoutlen_below_minimum_clamped() {
        let snap = make_snapshot(5, 80, 50);
        let mut opts = vim_core::VimOptions::default();
        snap.apply_to_options(&mut opts);
        assert_eq!(opts.timeoutlen_ms(), 100);
    }

    #[test]
    fn timeoutlen_above_maximum_clamped() {
        let snap = make_snapshot(5, 80, 10000);
        let mut opts = vim_core::VimOptions::default();
        snap.apply_to_options(&mut opts);
        assert_eq!(opts.timeoutlen_ms(), 5000);
    }

    #[test]
    fn timeoutlen_zero_clamped_to_minimum() {
        let snap = make_snapshot(5, 80, 0);
        let mut opts = vim_core::VimOptions::default();
        snap.apply_to_options(&mut opts);
        assert_eq!(opts.timeoutlen_ms(), 100);
    }

    #[test]
    fn timeoutlen_negative_clamped_to_minimum() {
        let snap = make_snapshot(5, 80, -100);
        let mut opts = vim_core::VimOptions::default();
        snap.apply_to_options(&mut opts);
        assert_eq!(opts.timeoutlen_ms(), 100);
    }

    #[test]
    fn timeoutlen_within_range() {
        let snap = make_snapshot(5, 80, 750);
        let mut opts = vim_core::VimOptions::default();
        snap.apply_to_options(&mut opts);
        assert_eq!(opts.timeoutlen_ms(), 750);
    }

    #[test]
    fn cursor_enabled_default_is_true() {
        let snap = make_snapshot(5, 80, 1000);
        assert!(snap.cursor.enabled);
    }

    #[test]
    fn cursor_lerp_speed_default_is_25() {
        let snap = make_snapshot(5, 80, 1000);
        assert_eq!(snap.cursor.lerp_speed, 25.0);
    }

    #[test]
    fn cursor_underline_height_default_is_4() {
        let snap = make_snapshot(5, 80, 1000);
        assert_eq!(snap.cursor.underline_height, 4.0);
    }

    // The UI range slider caps these, but the engine must handle any i64.

    #[test]
    fn large_scrolloff() {
        let snap = make_snapshot(999, 80, 1000);
        let mut opts = vim_core::VimOptions::default();
        snap.apply_to_options(&mut opts);
        assert_eq!(opts.scrolloff(), 999);
    }

    #[test]
    fn large_textwidth() {
        let snap = make_snapshot(5, 200, 1000);
        let mut opts = vim_core::VimOptions::default();
        snap.apply_to_options(&mut opts);
        assert_eq!(opts.textwidth(), 200);
    }

    #[test]
    fn inccommand_mode_default_is_nosplit() {
        assert_eq!(InccommandMode::default(), InccommandMode::Nosplit);
    }

    #[test]
    fn inccommand_mode_off_is_not_enabled() {
        assert!(!InccommandMode::Off.is_enabled());
    }

    #[test]
    fn inccommand_mode_nosplit_is_enabled() {
        assert!(InccommandMode::Nosplit.is_enabled());
    }

    #[test]
    fn all_clamped_to_minimums() {
        let snap = make_snapshot(-1, -1, -1);
        let mut opts = vim_core::VimOptions::default();
        snap.apply_to_options(&mut opts);
        assert_eq!(opts.scrolloff(), 0);
        assert_eq!(opts.textwidth(), 0);
        assert_eq!(opts.timeoutlen_ms(), 100);
    }
}
