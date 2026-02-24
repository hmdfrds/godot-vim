//! Cursor extraction helpers for `VimController`.

use crate::bridge::vim_adapter::core::cast::i32_to_usize;
use crate::bridge::vim_wrapper::VimController;

use godot::classes::CodeEdit;
use godot::prelude::*;
use vim_core::domain::position::Position;

impl VimController {
    /// Extract logical cursor position from a `CodeEdit`.
    ///
    /// Godot caret positions can point to the exclusive end of a selection.
    /// This helper normalizes to the logical character position expected by core logic.
    #[inline]
    pub(crate) fn cursor_from_editor(editor: &Gd<CodeEdit>) -> Position {
        let line = editor.get_caret_line();
        let mut col = editor.get_caret_column();

        if editor.has_selection() {
            let to_line = editor.get_selection_to_line();
            let to_col = editor.get_selection_to_column();

            if line == to_line && col == to_col && col > 0 {
                col -= 1;
            }
        }

        Position::new(i32_to_usize(line), i32_to_usize(col))
    }
}
