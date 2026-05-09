//! Applies text mutation effects (insert, delete, replace) to CodeEdit,
//! converting vim-core byte offsets to Godot `(line, col)` coordinates.
//!
//! All coordinate lookups use the caller-provided `LineIndex` for O(log n)
//! binary search, avoiding the O(n) linear scan of the free-function fallback.

use crate::bridge::codec::DocumentView;
use crate::bridge::port::TextEditorPort;

/// Coordinate-addressed insert — no caret/selection side effects.
pub(super) fn insert_at(editor: &mut impl TextEditorPort, line: i32, col: i32, content: &str) {
    editor.insert_text(content, line, col);
}

pub(crate) fn handle_insert(
    editor: &mut impl TextEditorPort,
    doc: &DocumentView,
    offset: usize,
    content: &str,
) {
    let pos = doc.line_index.byte_to_line_col(doc.text, offset);
    log::trace!(
        "text_insert: offset={} -> line={} col={} len={}",
        offset,
        pos.line,
        pos.col,
        content.len()
    );
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
    log::trace!(
        "text_delete: range={}..{} -> ({},{})..({},{})",
        start,
        end,
        start_pos.line,
        start_pos.col,
        end_pos.line,
        end_pos.col
    );
    editor.remove_text(start_pos.line, start_pos.col, end_pos.line, end_pos.col);
}

/// Replace `[start, end)` with `content`. Wrapped in a single complex
/// operation so both steps form one undo entry.
pub(crate) fn handle_replace(
    editor: &mut impl TextEditorPort,
    doc: &DocumentView,
    start: usize,
    end: usize,
    content: &str,
) {
    let start_pos = doc.line_index.byte_to_line_col(doc.text, start);
    let end_pos = doc.line_index.byte_to_line_col(doc.text, end);
    log::trace!(
        "text_replace: range={}..{} -> ({},{})..({},{}) new_len={}",
        start,
        end,
        start_pos.line,
        start_pos.col,
        end_pos.line,
        end_pos.col,
        content.len()
    );

    editor.begin_complex_operation();
    editor.remove_text(start_pos.line, start_pos.col, end_pos.line, end_pos.col);
    editor.insert_text(content, start_pos.line, start_pos.col);
    editor.end_complex_operation();
}
