//! EditorSettings key path constants for all GodotVim settings.
//!
//! The `plugins/GodotVim/` prefix is a Godot convention: `EditorSettings` uses
//! the slash-separated path as a hierarchy, so these keys appear as a dedicated
//! collapsible section in Editor > Editor Settings. Changing the prefix would
//! orphan any previously-saved user values.

// ─────────────────────────────────────────────────────────────────────────────
// Top-level
// ─────────────────────────────────────────────────────────────────────────────

pub(crate) const LOG_LEVEL: &str = "plugins/GodotVim/log_level";
pub(crate) const ENABLED: &str = "plugins/GodotVim/enabled";

// ─────────────────────────────────────────────────────────────────────────────
// Editor behavior
// ─────────────────────────────────────────────────────────────────────────────

// tabstop, shiftwidth, expandtab: not registered here — Godot's CodeEdit
// is the source of truth for indent settings. Synced in plugin/attach.rs.

pub(crate) const SCROLLOFF: &str = "plugins/GodotVim/editor/scrolloff";
pub(crate) const TEXTWIDTH: &str = "plugins/GodotVim/editor/textwidth";
pub(crate) const CLIPBOARD_ENABLED: &str = "plugins/GodotVim/editor/clipboard_enabled";
pub(crate) const IGNORECASE: &str = "plugins/GodotVim/editor/ignorecase";
pub(crate) const SMARTCASE: &str = "plugins/GodotVim/editor/smartcase";
pub(crate) const LINE_NUMBER_MODE: &str = "plugins/GodotVim/editor/line_number_mode";
pub(crate) const INCCOMMAND: &str = "plugins/GodotVim/editor/inccommand";
pub(crate) const HIGHLIGHT_YANK_DURATION: &str = "plugins/GodotVim/editor/highlight_yank_duration";

// ─────────────────────────────────────────────────────────────────────────────
// Cursor colors
// ─────────────────────────────────────────────────────────────────────────────

pub(crate) const CURSOR_NORMAL: &str = "plugins/GodotVim/cursor/normal_mode_color";
pub(crate) const CURSOR_INSERT: &str = "plugins/GodotVim/cursor/insert_mode_color";
pub(crate) const CURSOR_VISUAL: &str = "plugins/GodotVim/cursor/visual_mode_color";
pub(crate) const CURSOR_REPLACE: &str = "plugins/GodotVim/cursor/replace_mode_color";
pub(crate) const CURSOR_OPERATOR: &str = "plugins/GodotVim/cursor/operator_mode_color";
pub(crate) const CURSOR_COMMAND: &str = "plugins/GodotVim/cursor/command_mode_color";

// ─────────────────────────────────────────────────────────────────────────────
// Cursor behavior
// ─────────────────────────────────────────────────────────────────────────────

pub(crate) const CURSOR_ENABLED: &str = "plugins/GodotVim/cursor/enabled";

pub(crate) const CURSOR_LERP_SPEED: &str = "plugins/GodotVim/cursor/lerp_speed";
pub(crate) const CURSOR_UNDERLINE_HEIGHT: &str = "plugins/GodotVim/cursor/underline_height";

// ─────────────────────────────────────────────────────────────────────────────
// Key mapping
// ─────────────────────────────────────────────────────────────────────────────

pub(crate) const TIMEOUTLEN: &str = "plugins/GodotVim/mapping/timeoutlen";
pub(crate) const CONFIG_FILE_PATH: &str = "plugins/GodotVim/mapping/config_file_path";

// ─────────────────────────────────────────────────────────────────────────────
// Native Godot settings (read-only — NOT registered by GodotVim)
// ─────────────────────────────────────────────────────────────────────────────

/// Whether the user wants code completion to auto-trigger as they type.
/// This is Godot's native EditorSetting, not a GodotVim setting.
pub(crate) const CODE_COMPLETE_ENABLED: &str = "text_editor/completion/code_complete_enabled";

// ─────────────────────────────────────────────────────────────────────────────
// Input
// ─────────────────────────────────────────────────────────────────────────────

pub(crate) const PASSTHROUGH_KEYS: &str = "plugins/GodotVim/input/passthrough_keys";

// ─────────────────────────────────────────────────────────────────────────────
// Security
// ─────────────────────────────────────────────────────────────────────────────

pub(crate) const SHELL_EXECUTION: &str = "plugins/GodotVim/security/shell_execution";
pub(crate) const FILE_ACCESS_SCOPE: &str = "plugins/GodotVim/security/file_access_scope";
pub(crate) const PROJECT_VIMRC: &str = "plugins/GodotVim/security/project_vimrc";

// ─────────────────────────────────────────────────────────────────────────────
// Status bar colors
// ─────────────────────────────────────────────────────────────────────────────

pub(crate) const STATUS_BAR_NORMAL_BG: &str = "plugins/GodotVim/status_bar/normal_bg";
pub(crate) const STATUS_BAR_INSERT_BG: &str = "plugins/GodotVim/status_bar/insert_bg";
pub(crate) const STATUS_BAR_VISUAL_BG: &str = "plugins/GodotVim/status_bar/visual_bg";
pub(crate) const STATUS_BAR_REPLACE_BG: &str = "plugins/GodotVim/status_bar/replace_bg";
pub(crate) const STATUS_BAR_COMMAND_BG: &str = "plugins/GodotVim/status_bar/command_bg";
pub(crate) const STATUS_BAR_RECORDING_BG: &str = "plugins/GodotVim/status_bar/recording_bg";
pub(crate) const STATUS_BAR_TEXT_FG: &str = "plugins/GodotVim/status_bar/text_fg";
pub(crate) const STATUS_BAR_ERROR_FG: &str = "plugins/GodotVim/status_bar/error_fg";
