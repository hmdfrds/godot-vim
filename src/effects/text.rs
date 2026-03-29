//! Applies text mutation effects (insert, delete, replace) to CodeEdit,
//! converting vim-core byte offsets to Godot `(line, col)` coordinates.
//!
//! All coordinate lookups use the caller-provided `LineIndex` for O(log n)
//! binary search, avoiding the O(n) linear scan of the free-function fallback.

use crate::bridge::port::TextEditorPort;
use crate::bridge::codec::DocumentView;
use crate::types::CharLineCol;

/// Canonical single-point insert. Collapses any existing selection first
/// (via `select(pos, pos)`) so that `insert_text_at_caret` does a pure
/// insert rather than a replace. Used by both `handle_insert` and `auto_brace`.
pub(super) fn insert_at(editor: &mut impl TextEditorPort, line: i32, col: i32, content: &str) {
    let pos = CharLineCol::new(line, col);
    editor.set_caret_line(line);
    editor.set_caret_column(col);
    editor.select(pos, pos);
    editor.insert_text_at_caret(content);
}

pub(crate) fn handle_insert(
    editor: &mut impl TextEditorPort,
    doc: &DocumentView,
    offset: usize,
    content: &str,
) {
    let pos = doc.line_index.byte_to_line_col(doc.text, offset);
    log::trace!("text_insert: offset={} -> line={} col={} len={}", offset, pos.line, pos.col, content.len());
    insert_at(editor, pos.line, pos.col, content);
}

pub(crate) fn handle_delete(
    editor: &mut impl TextEditorPort,
    doc: &DocumentView,
    start: usize,
    end: usize,
) {
    let start_pos = doc.line_index.byte_to_line_col(doc.text, start);
    let end_pos = doc.line_index.byte_to_line_col(doc.text, end);
    log::trace!("text_delete: range={}..{} -> ({},{})..({},{})", start, end, start_pos.line, start_pos.col, end_pos.line, end_pos.col);
    editor.select(start_pos, end_pos);
    editor.delete_selection();
}

/// Replace `[start, end)` with `content`. Leverages Godot's behavior where
/// `insert_text_at_caret` replaces any active selection.
pub(crate) fn handle_replace(
    editor: &mut impl TextEditorPort,
    doc: &DocumentView,
    start: usize,
    end: usize,
    content: &str,
) {
    let start_pos = doc.line_index.byte_to_line_col(doc.text, start);
    let end_pos = doc.line_index.byte_to_line_col(doc.text, end);
    log::trace!("text_replace: range={}..{} -> ({},{})..({},{}) new_len={}", start, end, start_pos.line, start_pos.col, end_pos.line, end_pos.col, content.len());

    editor.select(start_pos, end_pos);
    editor.insert_text_at_caret(content);
}
