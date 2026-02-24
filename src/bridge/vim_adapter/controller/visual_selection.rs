use crate::bridge::vim_adapter::handlers::visual;
use crate::bridge::vim_wrapper::VimController;

impl VimController {
    /// Updates visual selection highlighting for all visual modes.
    ///
    /// In visual modes, `vim_state.cursor_state.position` is the source of truth
    /// because Godot's `editor.select()` moves the caret to the selection endpoint,
    /// corrupting the cursor position. For non-visual modes the editor is the source.
    pub(crate) fn update_visual_selection(&mut self) {
        if let Some(mut editor) = self.get_editor() {
            let current_pos = if self.engine.is_visual() {
                self.engine.cursor_pos()
            } else {
                Self::cursor_from_editor(&editor)
            };
            visual::render_visual_selection(&mut editor, &self.engine.mode(), current_pos);
        }
    }
}
