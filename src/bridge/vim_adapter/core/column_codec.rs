//! Column conversion at the editor/core boundary.
//!
//! Godot `CodeEdit` exposes caret/selection columns as editor-native character columns.
//! `vim-core` uses UTF-8 byte columns. This module is the only translation boundary.

use godot::classes::CodeEdit;
use godot::obj::Gd;
use vim_core::domain::position::Position;

use crate::bridge::vim_adapter::core::cast::{i32_to_usize, usize_to_i32};

#[inline]
fn clamp_byte_to_char_boundary(line: &str, byte_col: usize) -> usize {
    let mut clamped = byte_col.min(line.len());
    while clamped > 0 && !line.is_char_boundary(clamped) {
        clamped -= 1;
    }
    clamped
}

/// Convert an editor-native character column to a core byte column for a line.
#[must_use]
pub fn editor_col_to_byte(line: &str, editor_col: usize) -> usize {
    line.char_indices()
        .nth(editor_col)
        .map_or(line.len(), |(byte_idx, _)| byte_idx)
}

/// Convert a core byte column to an editor-native character column for a line.
#[must_use]
pub fn byte_to_editor_col(line: &str, byte_col: usize) -> usize {
    let clamped = clamp_byte_to_char_boundary(line, byte_col);
    line[..clamped].chars().count()
}

#[inline]
fn editor_line_text(editor: &Gd<CodeEdit>, line: usize) -> String {
    let line_count = i32_to_usize(editor.get_line_count());
    if line >= line_count {
        return String::new();
    }
    editor.get_line(usize_to_i32(line)).to_string()
}

/// Convert editor column -> byte column using current `CodeEdit` line contents.
#[must_use]
pub fn editor_col_to_byte_in_editor(editor: &Gd<CodeEdit>, line: usize, editor_col: usize) -> usize {
    let line_text = editor_line_text(editor, line);
    editor_col_to_byte(&line_text, editor_col)
}

/// Convert byte column -> editor column using current `CodeEdit` line contents.
#[must_use]
pub fn byte_to_editor_col_in_editor(editor: &Gd<CodeEdit>, line: usize, byte_col: usize) -> usize {
    let line_text = editor_line_text(editor, line);
    byte_to_editor_col(&line_text, byte_col)
}

/// Read current caret as a core byte position.
#[must_use]
pub fn caret_to_core_position(editor: &Gd<CodeEdit>) -> Position {
    let line = i32_to_usize(editor.get_caret_line());
    let editor_col = i32_to_usize(editor.get_caret_column());
    let byte_col = editor_col_to_byte_in_editor(editor, line, editor_col);
    Position::from_byte(line, byte_col)
}
