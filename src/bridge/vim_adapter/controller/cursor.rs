//! Cursor extraction helpers for `VimController`.

use crate::bridge::vim_adapter::core::cast::i32_to_usize;
use crate::bridge::vim_adapter::core::column_codec;
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
        let mut editor_col = editor.get_caret_column();

        if editor.has_selection() {
            let to_line = editor.get_selection_to_line();
            let to_col = editor.get_selection_to_column();

            if line == to_line && editor_col == to_col && editor_col > 0 {
                editor_col -= 1;
            }
        }

        let line_usize = i32_to_usize(line);
        let col_usize = i32_to_usize(editor_col);
        let byte_col = column_codec::editor_col_to_byte_in_editor(editor, line_usize, col_usize);
        Position::from_byte(line_usize, byte_col)
    }
}
