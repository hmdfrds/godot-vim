//! Command line handling methods for VimController.
//!
//! Handles ESC, history navigation, and cmdline input events.

use crate::bridge::vim_adapter::handlers::cmdline::IncsearchHandler;
use crate::bridge::vim_adapter::handlers::mode::ModeHandler;
use crate::bridge::vim_wrapper::VimController;
use vim_core::state::mode::Mode;

use godot::obj::WithBaseField;

impl VimController {
    /// Handles ESC key in command line mode - exits to Normal and clears search.
    pub(crate) fn handle_cmdline_escape(&mut self) {
        // Consume the event so LineEdit doesn't process it
        if let Some(mut vp) = self.base().get_viewport() {
            vp.set_input_as_handled();
        }

        // Clear incremental search highlights (user cancelled the search)
        self.clear_incsearch_highlights();

        // Reset history navigation
        self.engine.reset_history_nav();

        // Return to Normal mode
        self.engine.set_mode(Mode::Normal);
        self.handle_mode_change(Mode::Normal, None);

        // Return focus to editor
        if let Some(editor) = self.get_editor() {
            let mut control = editor.clone().upcast::<godot::classes::Control>();
            control.grab_focus();
        }
    }

    /// Handles UP key in command line - navigates to older history entry.
    pub(crate) fn handle_cmdline_history_up(&mut self) {
        if let Some(mut vp) = self.base().get_viewport() {
            vp.set_input_as_handled();
        }

        let Some(cmdline) = self.ui.cmdline.as_ref().filter(|c| c.is_instance_valid()) else {
            return;
        };
        let Some(mut input) = cmdline.bind().get_command_input() else {
            return;
        };

        // GString -> String conversion required for history_up (takes &str)
        let current_text = input.get_text().to_string();

        if let Some(history_entry) = self.engine.history_up(&current_text) {
            // history_entry is &str, can pass directly - no allocation
            input.set_text(history_entry);
            // Use char count for correct caret positioning with Unicode
            let count: usize = history_entry.chars().count();
            input.set_caret_column(count as i32);
        }
    }

    /// Handles DOWN key in command line - navigates to newer history entry.
    pub(crate) fn handle_cmdline_history_down(&mut self) {
        if let Some(mut vp) = self.base().get_viewport() {
            vp.set_input_as_handled();
        }

        let Some(cmdline) = self.ui.cmdline.as_ref().filter(|c| c.is_instance_valid()) else {
            return;
        };
        let Some(mut input) = cmdline.bind().get_command_input() else {
            return;
        };

        if let Some(history_entry) = self.engine.history_down() {
            // history_entry is &str, can pass directly - no allocation
            input.set_text(history_entry);
            // Use char count for correct caret positioning with Unicode
            let count: usize = history_entry.chars().count();
            input.set_caret_column(count as i32);
        }
    }
}
