use vim_core::state::config::Config as VimConfig;

use super::VimEngine;

impl VimEngine {
    /// Update the engine config from editor settings.
    pub fn update_config(
        &mut self,
        indent_size: usize,
        use_tabs: bool,
        is_keyword: String,
        scroll_offset: usize,
        yank_to_clipboard: bool,
        delete_to_clipboard: bool,
    ) {
        let mut config = VimConfig::new(indent_size, use_tabs, is_keyword, scroll_offset);
        config.yank_to_clipboard = yank_to_clipboard;
        config.delete_to_clipboard = delete_to_clipboard;
        self.config = config;
    }
}
