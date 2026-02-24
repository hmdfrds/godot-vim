//! Setting keys for `GodotVim` configuration.
//!
//! All settings are prefixed with `plugins/GodotVim/` to appear under the
//! "Plugins → GodotVim" category in Editor Settings.
//!
//! Structure:
//! - `plugins/GodotVim/enabled` - Master toggle (top level for visibility)
//! - `plugins/GodotVim/editor/` - Editor behavior settings
//! - `plugins/GodotVim/cursor/` - Cursor appearance settings
//! - `plugins/GodotVim/mapping/` - Key mapping settings

// ═══════════════════════════════════════════════════════════════════════════════
// Core Settings (Top Level)
// ═══════════════════════════════════════════════════════════════════════════════

/// Master toggle to enable/disable Vim functionality.
pub const ENABLED: &str = "plugins/GodotVim/enabled";

/// Log verbosity level (0=Error, 1=Warn, 2=Info, 3=Debug).
pub const LOG_LEVEL: &str = "plugins/GodotVim/log_level";

// ═══════════════════════════════════════════════════════════════════════════════
// Editor Settings
// ═══════════════════════════════════════════════════════════════════════════════

/// Lines to keep visible above/below cursor when scrolling (like Vim's scrolloff).
pub const SCROLL_OFFSET: &str = "plugins/GodotVim/editor/scroll_offset";

/// Toggle Godot's current line highlighting.
pub const HIGHLIGHT_CURRENT_LINE: &str = "plugins/GodotVim/editor/highlight_current_line";

/// Line number mode (0=None, 1=Absolute, 2=Relative, 3=Hybrid).
pub const LINE_NUMBER_MODE: &str = "plugins/GodotVim/editor/line_number_mode";

/// Show command line HUD at bottom of editor.
pub const HUD_CMDLINE_ENABLED: &str = "plugins/GodotVim/editor/cmdline_enabled";

/// Keys that bypass Vim and go directly to Godot.
/// Format: `["Ctrl+S", "Ctrl+Z", "F5"]`
pub const KEY_PASSTHROUGH_LIST: &str = "plugins/GodotVim/editor/key_passthrough_list";

/// Keywords definition (Vim 'iskeyword' option).
/// Standard Vim default: "@,48-57,_,192-255"
pub const IS_KEYWORD: &str = "plugins/GodotVim/editor/is_keyword";

// ═══════════════════════════════════════════════════════════════════════════════
// Cursor Settings
// ═══════════════════════════════════════════════════════════════════════════════

/// Enable mode-based cursor colors.
pub const MODE_COLORS_ENABLED: &str = "plugins/GodotVim/cursor/mode_colors_enabled";

/// Enable the smooth/blinking custom cursor overlay.
pub const PREMIUM_CURSOR_ENABLED: &str = "plugins/GodotVim/cursor/premium_cursor_enabled";

/// Cursor color in Normal mode.
pub const NORMAL_MODE_COLOR: &str = "plugins/GodotVim/cursor/normal_mode_color";

/// Cursor color in Insert mode.
pub const INSERT_MODE_COLOR: &str = "plugins/GodotVim/cursor/insert_mode_color";

/// Cursor color in Visual mode.
pub const VISUAL_MODE_COLOR: &str = "plugins/GodotVim/cursor/visual_mode_color";

// ═══════════════════════════════════════════════════════════════════════════════
// Clipboard Settings
// ═══════════════════════════════════════════════════════════════════════════════

/// Automatically copy yanked text to system clipboard.
pub const YANK_TO_CLIPBOARD: &str = "plugins/GodotVim/clipboard/yank_to_clipboard";

/// Automatically copy deleted/cut text to system clipboard.
pub const DELETE_TO_CLIPBOARD: &str = "plugins/GodotVim/clipboard/delete_to_clipboard";

// ═══════════════════════════════════════════════════════════════════════════════
// Mapping Settings
// ═══════════════════════════════════════════════════════════════════════════════

/// Enable custom keymappings (like jj -> Esc).
pub const MAPPING_ENABLED: &str = "plugins/GodotVim/mapping/enabled";

/// Timeout for key sequences in milliseconds (like Vim's timeoutlen).
/// If another key isn't pressed within this time, pending keys are processed literally.
pub const MAPPING_TIMEOUTLEN: &str = "plugins/GodotVim/mapping/timeoutlen";

/// Insert mode mappings: `Dictionary<from, to>` e.g. `{"jj": "<Esc>"}`
pub const IMAP: &str = "plugins/GodotVim/mapping/imap";

/// Normal mode mappings: Dictionary<from, to>
pub const NMAP: &str = "plugins/GodotVim/mapping/nmap";

/// Visual mode mappings: Dictionary<from, to>
pub const VMAP: &str = "plugins/GodotVim/mapping/vmap";

/// Global mode mappings: Dictionary<from, to>
/// These apply throughout the editor, including outside CodeEdit.
pub const GMAP: &str = "plugins/GodotVim/mapping/gmap";

/// Unified storage for all mappings (including disabled ones).
/// Array of Dictionary: `[{from: "jj", to: "<Esc>", modes: "invg"}, ...]`
/// modes is a string where i=insert, n=normal, v=visual, g=global
pub const ALL_MAPPINGS: &str = "plugins/GodotVim/mapping/all";

/// Window Navigation: Master Toggle (default: true)
pub const WINDOW_NAV_ENABLED: &str = "plugins/GodotVim/mapping/window_nav_enabled";

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_setting_keys_have_correct_prefix() {
        assert!(ENABLED.starts_with("plugins/GodotVim/"));
        assert!(LOG_LEVEL.starts_with("plugins/GodotVim/"));
        assert!(MODE_COLORS_ENABLED.starts_with("plugins/GodotVim/"));
        assert!(SCROLL_OFFSET.starts_with("plugins/GodotVim/"));
        assert!(KEY_PASSTHROUGH_LIST.starts_with("plugins/GodotVim/"));
    }

    #[test]
    fn test_setting_keys_organized_by_category() {
        // Core settings at top level
        assert_eq!(ENABLED, "plugins/GodotVim/enabled");
        assert_eq!(LOG_LEVEL, "plugins/GodotVim/log_level");
        // Editor settings
        assert!(SCROLL_OFFSET.contains("/editor/"));
        assert!(KEY_PASSTHROUGH_LIST.contains("/editor/"));
        assert!(LINE_NUMBER_MODE.contains("/editor/"));
        assert!(HUD_CMDLINE_ENABLED.contains("/editor/"));
        // Cursor settings
        assert!(MODE_COLORS_ENABLED.contains("/cursor/"));
        // Clipboard settings
        assert!(YANK_TO_CLIPBOARD.contains("/clipboard/"));
        // Mapping settings
        assert!(MAPPING_ENABLED.contains("/mapping/"));
    }
}
