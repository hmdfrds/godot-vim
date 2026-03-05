//! Default values for `GodotVim` settings.

use godot::prelude::Color;

// ─────────────────────────────────────────────────────────────────────────────
// General
// ─────────────────────────────────────────────────────────────────────────────

/// Plugin enabled by default.
pub const ENABLED: bool = true;

/// Default log level: Off
pub const LOG_LEVEL: i64 = 0;

// ─────────────────────────────────────────────────────────────────────────────
// Appearance
// ─────────────────────────────────────────────────────────────────────────────

// Default to Absolute (1)
pub const LINE_NUMBER_MODE: i64 = 1;

// ─────────────────────────────────────────────────────────────────────────────
// Cursor
// ─────────────────────────────────────────────────────────────────────────────

pub const MODE_COLORS_ENABLED: bool = true;

/// Normal mode: white
pub const NORMAL_MODE_COLOR: Color = Color::WHITE;

/// Insert mode: green (active editing)
pub const INSERT_MODE_COLOR: Color = Color::from_rgb(0.33, 1.0, 0.5); // #55FF7F

/// Visual mode: orange (selection)
pub const VISUAL_MODE_COLOR: Color = Color::from_rgb(1.0, 0.72, 0.33); // #FFB855

/// Custom cursor rendering (smooth/blink) enabled by default.
pub const PREMIUM_CURSOR_ENABLED: bool = true;

// ─────────────────────────────────────────────────────────────────────────────
// Formatting
// ─────────────────────────────────────────────────────────────────────────────

/// Default iskeyword: @,48-57,_,192-255 (standard Vim)
pub const IS_KEYWORD: &str = "@,48-57,_,192-255";

// ─────────────────────────────────────────────────────────────────────────────
// Status Bar
// ─────────────────────────────────────────────────────────────────────────────

pub const HUD_CMDLINE_ENABLED: bool = true;

// ─────────────────────────────────────────────────────────────────────────────
// Behavior
// ─────────────────────────────────────────────────────────────────────────────

pub const SCROLL_OFFSET: i64 = 5;
pub const HIGHLIGHT_CURRENT_LINE: bool = true;

// ─────────────────────────────────────────────────────────────────────────────
// Mapping
// ─────────────────────────────────────────────────────────────────────────────

pub const MAPPING_ENABLED: bool = false;

/// Default timeout: 500ms like Vim's timeoutlen
pub const MAPPING_TIMEOUTLEN: i64 = 500;
// The imap/nmap/vmap/cmap dictionaries are initialized at runtime.

// ─────────────────────────────────────────────────────────────────────────────
// Clipboard
// ─────────────────────────────────────────────────────────────────────────────

pub const YANK_TO_CLIPBOARD: bool = false;
pub const DELETE_TO_CLIPBOARD: bool = false;

// ─────────────────────────────────────────────────────────────────────────────
// Window Navigation (Intercept) - Toggle
// ─────────────────────────────────────────────────────────────────────────────

pub const WINDOW_NAV_ENABLED: bool = true;

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_colors_are_visible() {
        // All mode colors should have full alpha
        assert!((NORMAL_MODE_COLOR.a - 1.0).abs() < f32::EPSILON);
        assert!((INSERT_MODE_COLOR.a - 1.0).abs() < f32::EPSILON);
        assert!((VISUAL_MODE_COLOR.a - 1.0).abs() < f32::EPSILON);
    }
}
