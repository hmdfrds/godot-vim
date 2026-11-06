//! Undo/redo effect handlers using the changeset-based `UndoStore` pipeline.
//!
//! Applies `UndoApplyResult` changes to CodeEdit via targeted
//! `insert_text`/`remove_text` calls and restores cursor positions
//! from engine-computed offsets.

use crate::bridge::codec::{DocumentView, LineIndex};
use crate::bridge::port::TextEditorPort;

/// Apply `(from, to, Option<replacement>)` changes to a CodeEdit, last
/// first, so the byte offsets of `doc` stay valid for the ones before.
/// - `None` deletes `from..to`;
/// - `Some(text)` at `from == to` inserts;
/// - `Some(text)` over `from < to` replaces (delete, then insert).
pub(crate) fn apply_changes(
    editor: &mut impl TextEditorPort,
    doc: &DocumentView,
    changes: &[(usize, usize, Option<String>)],
) {
    for (from, to, replacement) in changes.iter().rev() {
        match replacement {
            None => super::text::handle_delete(editor, doc, *from, *to),
            Some(text) if from == to => super::text::handle_insert(editor, doc, *from, text),
            Some(text) => super::text::handle_replace(editor, doc, *from, *to, text),
        }
    }
}

/// Restore cursor positions from engine-computed byte offsets.
///
/// Removes secondary carets, then sets the primary cursor from `cursors[0]`
/// and adds secondary carets from `cursors[1..]`. Byte offsets are clamped
/// to the text length to handle edge cases.
pub(crate) fn restore_cursors(
    editor: &mut impl TextEditorPort,
    new_text: &str,
    cursors: &[vim_core::primitives::Offset],
) {
    if cursors.is_empty() {
        return;
    }

    let line_index = LineIndex::new(new_text);
    let text_len = new_text.len();

    editor.remove_secondary_carets();

    // Primary cursor.
    let primary_byte = cursors[0].get().min(text_len.saturating_sub(1));
    let primary_pos = line_index.byte_to_line_col(new_text, primary_byte);
    editor.set_caret_line(primary_pos.line);
    editor.set_caret_column(primary_pos.col);

    // Secondary cursors.
    for cursor in &cursors[1..] {
        let byte = cursor.get().min(text_len.saturating_sub(1));
        let pos = line_index.byte_to_line_col(new_text, byte);
        editor.add_caret(pos.line, pos.col);
    }
}

/// `U` (per-line undo) — not supported by Godot's CodeEdit.
pub(super) fn handle_undo_line(_count: u32) {
    use std::sync::atomic::{AtomicBool, Ordering};
    static LOGGED: AtomicBool = AtomicBool::new(false);
    if !LOGGED.swap(true, Ordering::Relaxed) {
        log::info!("U (undo line) not supported — CodeEdit provides only global undo");
    }
}
