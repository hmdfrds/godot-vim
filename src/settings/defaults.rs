//! Default value constants for all GodotVim editor settings.
//!
//! Each constant serves double duty: it seeds `EditorSettings` during registration
//! (so the key exists on first launch) and acts as the fallback when a read returns
//! a missing or wrong-typed value. Keeping both paths in sync via a single constant
//! prevents drift between the UI default and the runtime fallback.

use godot::prelude::*;

// ─────────────────────────────────────────────────────────────────────────────
// Top-level
// ─────────────────────────────────────────────────────────────────────────────

pub(crate) const LOG_LEVEL: &str = "Off";
pub(crate) const LOG_LEVEL_OPTIONS: &[&str] = &["Off", "Error", "Warn", "Info", "Debug", "Trace"];
pub(crate) const ENABLED: bool = true;

// ─────────────────────────────────────────────────────────────────────────────
// Editor behavior
// ─────────────────────────────────────────────────────────────────────────────

pub(crate) const SCROLLOFF: i64 = 5;
pub(crate) const TEXTWIDTH: i64 = 80;
pub(crate) const CLIPBOARD_ENABLED: bool = false;
pub(crate) const IGNORECASE: bool = false;
pub(crate) const SMARTCASE: bool = false;
pub(crate) const LINE_NUMBER_MODE: &str = "Hybrid";
pub(crate) const LINE_NUMBER_MODE_OPTIONS: &[&str] = &["None", "Absolute", "Relative", "Hybrid"];

pub(crate) const INCCOMMAND: &str = "nosplit";
pub(crate) const INCCOMMAND_OPTIONS: &[&str] = &["nosplit", "off"];
pub(crate) const HIGHLIGHT_YANK_DURATION: i64 = 150;

// ─────────────────────────────────────────────────────────────────────────────
// Cursor colors
// ─────────────────────────────────────────────────────────────────────────────

// Vivid, saturated colors work best here because the cursor overlay uses a
// difference-blend shader — muted tones become nearly invisible against
// dark editor backgrounds.

pub(crate) fn cursor_normal() -> Color {
    Color::from_rgb(1.0, 1.0, 1.0)
}

pub(crate) fn cursor_insert() -> Color {
    Color::from_rgb(0.33, 1.0, 0.5)
}

pub(crate) fn cursor_visual() -> Color {
    Color::from_rgb(1.0, 0.72, 0.33)
}

/// Alpha < 1.0 so the underlying character remains readable under the block cursor.
pub(crate) fn cursor_replace() -> Color {
    Color::from_rgba(1.0, 0.2, 0.2, 0.6)
}

pub(crate) fn cursor_operator() -> Color {
    Color::from_rgb(1.0, 0.72, 0.33)
}

pub(crate) fn cursor_command() -> Color {
    Color::from_rgb(1.0, 1.0, 1.0)
}

// ─────────────────────────────────────────────────────────────────────────────
// Cursor behavior
// ─────────────────────────────────────────────────────────────────────────────

pub(crate) const CURSOR_ENABLED: bool = true;
pub(crate) const CURSOR_LERP_SPEED: f64 = 25.0;
pub(crate) const CURSOR_UNDERLINE_HEIGHT: f64 = 4.0;

// ─────────────────────────────────────────────────────────────────────────────
// Key mapping
// ─────────────────────────────────────────────────────────────────────────────

pub(crate) const TIMEOUTLEN: i64 = 1000;
pub(crate) const TIMEOUTLEN_MIN: i64 = 100;
pub(crate) const TIMEOUTLEN_MAX: i64 = 5000;

pub(crate) const CONFIG_FILE_PATH: &str = "";

// ─────────────────────────────────────────────────────────────────────────────
// Native Godot settings (read-only fallback defaults)
// ─────────────────────────────────────────────────────────────────────────────

/// Fallback when the native `text_editor/completion/code_complete_enabled`
/// setting is missing or has an unexpected type. Godot defaults this to `true`.
pub(crate) const CODE_COMPLETE_ENABLED: bool = true;

// ─────────────────────────────────────────────────────────────────────────────
// Input
// ─────────────────────────────────────────────────────────────────────────────

pub(crate) const PASSTHROUGH_KEYS: &str = "";

// ─────────────────────────────────────────────────────────────────────────────
// Status bar colors
// ─────────────────────────────────────────────────────────────────────────────

pub(crate) fn status_bar_normal_bg() -> Color {
    Color::from_rgb(0.5, 0.6, 0.8)
}

pub(crate) fn status_bar_insert_bg() -> Color {
    Color::from_rgb(0.6, 0.8, 0.5)
}

pub(crate) fn status_bar_visual_bg() -> Color {
    Color::from_rgb(0.8, 0.5, 0.5)
}

pub(crate) fn status_bar_replace_bg() -> Color {
    Color::from_rgb(0.9, 0.6, 0.3)
}

pub(crate) fn status_bar_command_bg() -> Color {
    Color::from_rgb(0.157, 0.173, 0.204)
}

pub(crate) fn status_bar_recording_bg() -> Color {
    Color::from_rgb(0.9, 0.2, 0.2)
}

pub(crate) fn status_bar_text_fg() -> Color {
    Color::WHITE
}

pub(crate) fn status_bar_error_fg() -> Color {
    Color::from_rgb(1.0, 0.3, 0.3)
}

// ─────────────────────────────────────────────────────────────────────────────
// Security
// ─────────────────────────────────────────────────────────────────────────────

// Security defaults are restrictive — users must explicitly opt in to
// shell access and unrestricted filesystem paths.
pub(crate) const SHELL_EXECUTION: &str = "Disabled";
pub(crate) const SHELL_EXECUTION_OPTIONS: &[&str] = &["Disabled", "Enabled"];
pub(crate) const FILE_ACCESS_SCOPE: &str = "ProjectOnly";
pub(crate) const FILE_ACCESS_SCOPE_OPTIONS: &[&str] = &["ProjectOnly", "Unrestricted"];
pub(crate) const PROJECT_VIMRC: &str = "Sandbox";
pub(crate) const PROJECT_VIMRC_OPTIONS: &[&str] = &["Disabled", "Sandbox", "Trusted"];
