//! Column conversion at the editor/core boundary.
//!
//! Godot `CodeEdit` exposes caret/selection columns as editor-native character columns.
//! `vim-core` uses UTF-8 byte columns. This module is the only translation boundary.

use godot::classes::CodeEdit;
use godot::obj::Gd;
use vim_core::domain::position::Position;
use vim_core::domain::selection::Selection;

use crate::bridge::vim_adapter::core::cast::{i32_to_usize, usize_to_i32};

/// Column index in editor-native character space.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct EditorCol(usize);

impl EditorCol {
    #[inline]
    #[must_use]
    pub const fn new(raw: usize) -> Self {
        Self(raw)
    }

    #[inline]
    #[must_use]
    pub const fn as_usize(self) -> usize {
        self.0
    }
}

/// Column index in vim-core UTF-8 byte space.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CoreByteCol(usize);

impl CoreByteCol {
    #[inline]
    #[must_use]
    pub const fn new(raw: usize) -> Self {
        Self(raw)
    }

    #[inline]
    #[must_use]
    pub const fn as_usize(self) -> usize {
        self.0
    }
}

#[inline]
fn clamp_byte_to_char_boundary(line: &str, byte_col: usize) -> usize {
    let mut clamped = byte_col.min(line.len());
    while clamped > 0 && !line.is_char_boundary(clamped) {
        clamped -= 1;
    }
    clamped
}

/// Convert an editor-native character column to a typed core byte column for a line.
#[must_use]
pub(crate) fn editor_col_to_core_byte(line: &str, editor_col: EditorCol) -> CoreByteCol {
    line.char_indices()
        .nth(editor_col.as_usize())
        .map_or(CoreByteCol::new(line.len()), |(byte_idx, _)| {
            CoreByteCol::new(byte_idx)
        })
}

/// Convert a typed core byte column to an editor-native character column for a line.
#[must_use]
pub(crate) fn core_byte_to_editor_col(line: &str, byte_col: CoreByteCol) -> EditorCol {
    let clamped = clamp_byte_to_char_boundary(line, byte_col.as_usize());
    EditorCol::new(line[..clamped].chars().count())
}

#[inline]
fn editor_line_text(editor: &Gd<CodeEdit>, line: usize) -> String {
    let line_count = i32_to_usize(editor.get_line_count());
    if line >= line_count {
        return String::new();
    }
    editor.get_line(usize_to_i32(line)).to_string()
}

/// Editor character length (not bytes) for a line.
#[must_use]
pub(crate) fn editor_line_char_len(editor: &Gd<CodeEdit>, line: usize) -> usize {
    editor_line_text(editor, line).chars().count()
}

/// Clamp editor character column into valid bounds for a line.
#[must_use]
pub(crate) fn clamp_editor_col(
    editor: &Gd<CodeEdit>,
    line: usize,
    editor_col: EditorCol,
) -> EditorCol {
    EditorCol::new(
        editor_col
            .as_usize()
            .min(editor_line_char_len(editor, line)),
    )
}

/// Last valid line index, or `None` for empty editors.
#[must_use]
pub(crate) fn last_line_index(editor: &Gd<CodeEdit>) -> Option<i32> {
    let count = editor.get_line_count();
    if count <= 0 {
        None
    } else {
        Some(count - 1)
    }
}

/// Convert editor column -> byte column using current `CodeEdit` line contents.
#[must_use]
pub(crate) fn editor_col_to_byte_in_editor(
    editor: &Gd<CodeEdit>,
    line: usize,
    editor_col: usize,
) -> usize {
    editor_col_to_core_byte_in_editor(editor, line, EditorCol::new(editor_col)).as_usize()
}

/// Convert editor column -> typed core byte column using current `CodeEdit` line contents.
#[must_use]
pub(crate) fn editor_col_to_core_byte_in_editor(
    editor: &Gd<CodeEdit>,
    line: usize,
    editor_col: EditorCol,
) -> CoreByteCol {
    let line_text = editor_line_text(editor, line);
    editor_col_to_core_byte(&line_text, editor_col)
}

/// Convert byte column -> editor column using current `CodeEdit` line contents.
#[must_use]
pub(crate) fn byte_to_editor_col_in_editor(
    editor: &Gd<CodeEdit>,
    line: usize,
    byte_col: usize,
) -> usize {
    core_byte_to_editor_col_in_editor(editor, line, CoreByteCol::new(byte_col)).as_usize()
}

/// Convert typed core byte column -> typed editor column using current `CodeEdit` line contents.
#[must_use]
pub(crate) fn core_byte_to_editor_col_in_editor(
    editor: &Gd<CodeEdit>,
    line: usize,
    byte_col: CoreByteCol,
) -> EditorCol {
    let line_text = editor_line_text(editor, line);
    core_byte_to_editor_col(&line_text, byte_col)
}

/// Read current caret as a core byte position.
#[must_use]
pub fn read_caret_core_position(editor: &Gd<CodeEdit>) -> Position {
    let line = i32_to_usize(editor.get_caret_line());
    let editor_col = clamp_editor_col(
        editor,
        line,
        EditorCol::new(i32_to_usize(editor.get_caret_column())),
    );
    let byte_col = editor_col_to_core_byte_in_editor(editor, line, editor_col).as_usize();
    Position::from_byte(line, byte_col)
}

/// Backward-compatible alias for existing call sites.
#[must_use]
pub(crate) fn caret_to_core_position(editor: &Gd<CodeEdit>) -> Position {
    read_caret_core_position(editor)
}

/// Apply a core byte position to the editor caret.
pub fn apply_core_position_to_editor(editor: &mut Gd<CodeEdit>, pos: Position) {
    editor
        .set_caret_line_ex(usize_to_i32(pos.line))
        .can_be_hidden(false)
        .done();
    let editor_col =
        core_byte_to_editor_col_in_editor(editor, pos.line, CoreByteCol::new(pos.col.as_usize()));
    editor.set_caret_column(usize_to_i32(editor_col.as_usize()));
}

/// Read current selection as a core byte-domain selection.
#[must_use]
pub fn read_selection_core(editor: &Gd<CodeEdit>) -> Selection {
    let caret = read_caret_core_position(editor);
    if !editor.has_selection() {
        return Selection::new(caret, caret);
    }

    let caret_line = i32_to_usize(editor.get_caret_line());
    let caret_editor_col = i32_to_usize(editor.get_caret_column());

    let from_line = i32_to_usize(editor.get_selection_from_line());
    let from_editor_col = i32_to_usize(editor.get_selection_from_column());
    let to_line = i32_to_usize(editor.get_selection_to_line());
    let to_editor_col = i32_to_usize(editor.get_selection_to_column());

    let from_byte =
        editor_col_to_core_byte_in_editor(editor, from_line, EditorCol::new(from_editor_col));
    let logical_to_byte = if to_editor_col > 0 {
        editor_col_to_core_byte_in_editor(editor, to_line, EditorCol::new(to_editor_col - 1))
    } else {
        CoreByteCol::new(0)
    };

    if (caret_line, caret_editor_col) == (to_line, to_editor_col) {
        Selection::new(
            Position::from_byte(from_line, from_byte.as_usize()),
            Position::from_byte(to_line, logical_to_byte.as_usize()),
        )
    } else {
        Selection::new(
            Position::from_byte(to_line, logical_to_byte.as_usize()),
            Position::from_byte(from_line, from_byte.as_usize()),
        )
    }
}
