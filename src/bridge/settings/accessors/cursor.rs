use super::internal::{get_bool_setting, get_color_setting};
use super::VimSettings;
use crate::bridge::settings::{defaults, keys};
use godot::prelude::*;

impl VimSettings {
    /// Returns whether mode-based cursor colors are enabled.
    #[must_use]
    pub fn mode_colors_enabled() -> bool {
        get_bool_setting(keys::MODE_COLORS_ENABLED, defaults::MODE_COLORS_ENABLED)
    }

    /// Returns whether the smooth/blinking custom cursor overlay is enabled.
    #[must_use]
    pub fn premium_cursor_enabled() -> bool {
        get_bool_setting(
            keys::PREMIUM_CURSOR_ENABLED,
            defaults::PREMIUM_CURSOR_ENABLED,
        )
    }

    /// Returns the cursor color for Normal mode.
    #[must_use]
    pub fn normal_mode_color() -> Color {
        get_color_setting(keys::NORMAL_MODE_COLOR, defaults::NORMAL_MODE_COLOR)
    }

    /// Returns the cursor color for Insert mode.
    #[must_use]
    pub fn insert_mode_color() -> Color {
        get_color_setting(keys::INSERT_MODE_COLOR, defaults::INSERT_MODE_COLOR)
    }

    #[must_use]
    pub fn visual_mode_color() -> Color {
        get_color_setting(keys::VISUAL_MODE_COLOR, defaults::VISUAL_MODE_COLOR)
    }
}
