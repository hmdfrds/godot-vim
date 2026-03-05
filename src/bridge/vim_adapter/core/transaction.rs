//! Transaction application for Godot `CodeEdit`.
//!
//! This module provides the imperative shell function that applies
//! Transaction edits to a Godot `CodeEdit` instance.

use godot::classes::CodeEdit;
use godot::obj::Gd;

use crate::bridge::vim_adapter::core::cast::{i32_to_usize, usize_to_i32};
use crate::bridge::vim_adapter::core::column_codec;
use vim_core::prelude::{EditOp, Transaction};
use vim_core::protocol::messages::{TextOperation, TransactionPatch};

/// Applies a unified Transaction to a `CodeEdit` editor.
///
/// Converts `Transaction` edits into `TransactionPatch` style operations.
pub fn apply_transaction(editor: &mut Gd<CodeEdit>, transaction: &Transaction) {
    if !transaction.has_edits() {
        return;
    }

    for edit in &transaction.edits {
        match edit {
            EditOp::Insert { pos, text } => {
                apply_insert(editor, (pos.line, pos.col.as_usize()), text);
            }
            EditOp::Delete { start, end } => {
                apply_delete(
                    editor,
                    (start.line, start.col.as_usize()),
                    (end.line, end.col.as_usize()),
                );
            }
            EditOp::Replace { start, end, text } => {
                apply_delete(
                    editor,
                    (start.line, start.col.as_usize()),
                    (end.line, end.col.as_usize()),
                );
                apply_insert(editor, (start.line, start.col.as_usize()), text);
            }
            EditOp::BlockDelete { .. } | EditOp::BlockInsert { .. } => {
                log::warn!("Block operations not supported in transaction adapter");
            }
        }
    }
}

/// Applies a transaction patch to a `CodeEdit` editor.
///
/// This applies the atomic edits from a TransactionPatch.
/// Selection and cursor updates are handled via `StateDiff`, not here.
pub fn apply_transaction_patch(editor: &mut Gd<CodeEdit>, patch: &TransactionPatch) {
    // Apply edits in batch for better performance
    apply_text_ops_batch(editor, &patch.operations);

    // Selection is handled via StateDiff; no update needed here.
}

/// Apply a single text operation to the editor.
fn apply_text_op(editor: &mut Gd<CodeEdit>, op: &TextOperation) {
    match op {
        TextOperation::Insert { pos, text } => {
            apply_insert(editor, (pos.0 as usize, pos.1 as usize), text);
        }
        TextOperation::Delete { range, .. } => {
            // range is Range { start: (line, col), end: (line, col) }
            apply_delete(
                editor,
                (range.start.0 as usize, range.start.1 as usize),
                (range.end.0 as usize, range.end.1 as usize),
            );
        }
        TextOperation::Replace { range, text } => {
            apply_delete(
                editor,
                (range.start.0 as usize, range.start.1 as usize),
                (range.end.0 as usize, range.end.1 as usize),
            );
            apply_insert(
                editor,
                (range.start.0 as usize, range.start.1 as usize),
                text,
            );
        }
    }
}

/// Apply multiple text operations in batch.
fn apply_text_ops_batch(editor: &mut Gd<CodeEdit>, ops: &[TextOperation]) {
    if ops.is_empty() {
        return;
    }

    // Apply sequentially
    for op in ops {
        apply_text_op(editor, op);
    }
}

/// Insert text at a position.
fn apply_insert(editor: &mut Gd<CodeEdit>, pos: (usize, usize), text: &str) {
    column_codec::apply_core_position_to_editor(
        editor,
        vim_core::domain::position::Position::from_byte(pos.0, pos.1),
    );

    // Insert text
    editor.insert_text_at_caret(text);
}

/// Delete text between two positions.
fn apply_delete(editor: &mut Gd<CodeEdit>, start: (usize, usize), end: (usize, usize)) {
    let (start_line, start_col) = start;
    let (end_line, end_col) = end;

    // Check if this is a linewise delete (entire lines)
    if start_col == 0 && end_col == 0 && end_line > start_line {
        // Linewise: remove complete lines one at a time
        for _ in start_line..end_line {
            let line_count = i32_to_usize(editor.get_line_count());

            if start_line < line_count.saturating_sub(1) {
                // Not the last line - delete including newline
                editor.select(usize_to_i32(start_line), 0, usize_to_i32(start_line + 1), 0);
            } else {
                // Last line - select to end of line
                let line_len = editor
                    .get_line(usize_to_i32(start_line))
                    .to_string()
                    .chars()
                    .count();
                editor.select(
                    usize_to_i32(start_line),
                    0,
                    usize_to_i32(start_line),
                    usize_to_i32(line_len),
                );
            }
            editor.delete_selection();
        }
    } else if start_line == end_line {
        // Same line: simple character range delete
        let start_editor_col =
            column_codec::byte_to_editor_col_in_editor(editor, start_line, start_col);
        let end_editor_col = column_codec::byte_to_editor_col_in_editor(editor, end_line, end_col);
        editor.select(
            usize_to_i32(start_line),
            usize_to_i32(start_editor_col),
            usize_to_i32(end_line),
            usize_to_i32(end_editor_col),
        );
        editor.delete_selection();
    } else {
        // Multi-line but not linewise: delete from start to end
        let start_editor_col =
            column_codec::byte_to_editor_col_in_editor(editor, start_line, start_col);
        let end_editor_col = column_codec::byte_to_editor_col_in_editor(editor, end_line, end_col);
        editor.select(
            usize_to_i32(start_line),
            usize_to_i32(start_editor_col),
            usize_to_i32(end_line),
            usize_to_i32(end_editor_col),
        );
        editor.delete_selection();
    }
}
