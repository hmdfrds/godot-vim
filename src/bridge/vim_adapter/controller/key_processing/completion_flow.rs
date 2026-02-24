use crate::bridge::vim_wrapper::VimController;
use vim_core::inputs::{KeyCode, VimKey, VimModifiers};

impl VimController {
    /// Handles code completion popup interactions.
    ///
    /// Returns `true` if the key was consumed.
    pub(crate) fn handle_code_completion(&mut self, vim_key: &VimKey) -> bool {
        let Some(editor) = self.get_editor() else {
            return false;
        };

        let active = self.input.completion_manager.is_active(&editor);
        self.engine.sync_completion_visible(active);

        if self.engine.is_insert()
            && vim_key.modifiers.contains(VimModifiers::CTRL)
            && vim_key.code == KeyCode::Char(' ')
        {
            let mut editor = editor.clone();
            editor.request_code_completion();
            self.set_input_handled();
            return true;
        }

        false
    }
}
