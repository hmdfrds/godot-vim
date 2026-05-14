//! Undo/redo effect handlers using the changeset-based `UndoStore` pipeline.
//!
//! Applies `UndoApplyResult` changes to CodeEdit via targeted
//! `insert_text`/`remove_text` calls and restores cursor positions
//! from engine-computed offsets.

use crate::bridge::codec::{DocumentView, LineIndex};
use crate::bridge::port::TextEditorPort;
use crate::state::undo_store::UndoApplyResult;

/// Apply changeset changes to CodeEdit in REVERSE order.
///
/// Each `(from, to, replacement)` triple is processed back-to-front so that
/// earlier byte offsets remain valid while later regions are modified.
/// - `from < to` with `None` → pure deletion
/// - `from == to` with `Some(text)` → pure insertion
/// - `from < to` with `Some(text)` → replacement (delete then insert)
pub(crate) fn apply_changes_to_editor(
    editor: &mut impl TextEditorPort,
    doc: &DocumentView,
    result: &UndoApplyResult,
) {
    // Iterate in reverse so that mutations at higher offsets don't
    // invalidate the byte positions of earlier changes.
    for &(from, to, ref replacement) in result.changes.iter().rev() {
        if from < to {
            let from_pos = doc.line_index.byte_to_line_col(doc.text, from);
            let to_pos = doc.line_index.byte_to_line_col(doc.text, to);
            editor.remove_text(from_pos.line, from_pos.col, to_pos.line, to_pos.col);
        }
        if let Some(ref text) = replacement {
            let insert_pos = doc.line_index.byte_to_line_col(doc.text, from);
            editor.insert_text(text, insert_pos.line, insert_pos.col);
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
