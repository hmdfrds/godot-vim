use crate::bridge::vim_wrapper::VimController;
use godot::prelude::*;

impl VimController {
    /// Runs a closure with the attached editor, if available.
    pub(super) fn with_editor(
        &mut self,
        f: impl FnOnce(&mut Self, &mut Gd<godot::classes::CodeEdit>),
    ) {
        if let Some(mut editor) = self.get_editor() {
            f(self, &mut editor);
        }
    }

    /// Tags the current editor version as saved (for undo sync).
    pub(super) fn handle_undo_sync(&mut self) {
        if let Some(mut editor) = self.get_editor() {
            editor.tag_saved_version();
        }
    }

    /// Scrolls the editor viewport by one line.
    pub(super) fn handle_scroll_window(&mut self, up: bool) {
        if let Some(mut editor) = self.get_editor() {
            let line = editor.get_v_scroll();
            editor.set_v_scroll(if up { line - 1.0 } else { line + 1.0 });
        }
    }

    /// Sets the editor viewport to a specific line.
    pub(super) fn handle_viewport_update(&mut self, top_line: usize) {
        if let Some(mut editor) = self.get_editor() {
            #[expect(
                clippy::cast_precision_loss,
                reason = "line numbers do not need 64-bit precision"
            )]
            editor.set_v_scroll(top_line as f64);
        }
    }
}
