use super::internal::{get_bool_setting, get_int_setting, get_string_array_setting, get_string_setting, normalize_key_string, set_bool_setting};
use super::{LineNumberMode, LogLevel, VimSettings};
use crate::bridge::settings::{defaults, keys};

impl VimSettings {
    /// Returns the line number display mode.
    #[must_use]
    pub fn line_number_mode() -> LineNumberMode {
        LineNumberMode::from(get_int_setting(
            keys::LINE_NUMBER_MODE,
            defaults::LINE_NUMBER_MODE,
        ))
    }

    /// Returns whether Vim functionality is enabled.
    /// When false, all input passes through to Godot.
    #[must_use]
    pub fn enabled() -> bool {
        get_bool_setting(keys::ENABLED, defaults::ENABLED)
    }

    /// Sets whether Vim functionality is enabled.
    pub fn set_enabled(enabled: bool) {
        set_bool_setting(keys::ENABLED, enabled);
    }

    /// Returns the current log level.
    #[must_use]
    pub fn log_level() -> LogLevel {
        LogLevel::from(get_int_setting(keys::LOG_LEVEL, defaults::LOG_LEVEL))
    }

    /// Returns the 'iskeyword' option string.
    #[must_use]
    pub fn is_keyword() -> String {
        get_string_setting(keys::IS_KEYWORD, defaults::IS_KEYWORD)
    }

    /// Returns whether the floating command line HUD is enabled.
    #[must_use]
    pub fn cmdline_enabled() -> bool {
        get_bool_setting(keys::HUD_CMDLINE_ENABLED, defaults::HUD_CMDLINE_ENABLED)
    }

    /// Returns the scroll offset (lines to keep visible above/below cursor).
    #[must_use]
    pub fn scroll_offset() -> i32 {
        #[expect(
            clippy::cast_possible_truncation,
            reason = "i64 to i32 is safe for small range 0-20"
        )]
        let offset = get_int_setting(keys::SCROLL_OFFSET, defaults::SCROLL_OFFSET) as i32;
        offset.clamp(0, 20)
    }

    /// Returns whether current line highlighting is enabled.
    #[must_use]
    pub fn highlight_current_line() -> bool {
        get_bool_setting(
            keys::HIGHLIGHT_CURRENT_LINE,
            defaults::HIGHLIGHT_CURRENT_LINE,
        )
    }

    /// Returns the list of keys that should bypass Vim.
    /// Supported formats:
    /// - User-friendly: "Ctrl+S", "Shift+F5", "Alt+Tab"
    /// - Vim notation: `"<C-s>"`, `"<S-F5>"`, `"<A-Tab>"`
    #[must_use]
    pub fn key_passthrough_list() -> Vec<String> {
        get_string_array_setting(keys::KEY_PASSTHROUGH_LIST)
    }

    /// Checks if a key combination should bypass Vim.
    ///
    /// Supports matching both user-friendly format (Ctrl+S) and Vim notation (`<C-s>`).
    ///
    /// # Arguments
    /// * `vim_key_string` - Key in Vim notation format (e.g., `"<C-s>"`, `"<F5>"`)
    #[must_use]
    #[allow(dead_code)]
    pub fn should_passthrough(vim_key_string: &str) -> bool {
        let normalized_input = normalize_key_string(vim_key_string);
        Self::key_passthrough_list()
            .iter()
            .any(|k| normalize_key_string(k).eq_ignore_ascii_case(&normalized_input))
    }

    /// Returns whether the Window Navigation Intercept (<C-h/j/k/l>) is enabled.
    #[must_use]
    pub fn window_nav_enabled() -> bool {
        get_bool_setting(keys::WINDOW_NAV_ENABLED, defaults::WINDOW_NAV_ENABLED)
    }

    /// Returns whether yank operations should automatically copy to system clipboard.
    /// When enabled, `yy`, `yw`, etc. will also update the system clipboard.
    #[must_use]
    pub fn yank_to_clipboard() -> bool {
        get_bool_setting(keys::YANK_TO_CLIPBOARD, defaults::YANK_TO_CLIPBOARD)
    }

    /// Returns whether delete/cut operations should automatically copy to system clipboard.
    /// When enabled, `dd`, `dw`, `x`, etc. will also update the system clipboard.
    #[must_use]
    pub fn delete_to_clipboard() -> bool {
        get_bool_setting(keys::DELETE_TO_CLIPBOARD, defaults::DELETE_TO_CLIPBOARD)
    }
}
