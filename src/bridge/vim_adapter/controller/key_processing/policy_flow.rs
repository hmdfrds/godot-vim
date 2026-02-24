use crate::bridge::vim_adapter::handlers::mode::ModeHandler;
use crate::bridge::vim_wrapper::VimController;
use vim_core::inputs::{KeyCode, VimKey};
use vim_core::state::mode::Mode;

impl VimController {
    /// Handles CmdLine mode input: Esc exits, remaining keys bubble to LineEdit.
    /// Returns `true` when currently in command-line mode.
    pub(crate) fn try_handle_cmdline_mode(&mut self, vim_key: &VimKey) -> bool {
        if !self.engine.is_cmdline() {
            return false;
        }

        if vim_key.code == KeyCode::Esc {
            self.set_input_handled();
            self.engine.set_mode(Mode::Normal);
            self.handle_mode_change(Mode::Normal, None);

            if let Some(editor) = self.get_editor() {
                let mut control = editor.clone().upcast::<godot::classes::Control>();
                control.grab_focus();
            }
        }

        true
    }
}
