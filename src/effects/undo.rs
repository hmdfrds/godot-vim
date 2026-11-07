//! Undo/redo effect handlers using the changeset-based `UndoStore` pipeline.
//!
//! Applies `UndoApplyResult` changes to CodeEdit via targeted
//! `insert_text`/`remove_text` calls and places the carets from
//! engine-computed offsets.

use crate::bridge::codec::{DocumentView, LineIndex};
use crate::bridge::port::{FoldCapable, TextEditorPort};

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

/// Place the carets of `u` or `<C-r>` from engine-computed byte offsets,
/// clamped to the text, and follow the primary one keeping `scrolloff`.
///
/// A closed fold hiding the primary caret's line opens, as Godot's own undo
/// does (code_edit.cpp `_unhide_carets`). A folded header stays closed: its
/// change is visible without opening it. Vim opens that one too (undo.c
/// `u_undo_end`, fold.c `foldOpenCursor`), but Godot draws the header
/// outside its fold.
pub(crate) fn place_undo_caret(
    editor: &mut impl FoldCapable,
    new_text: &str,
    cursors: &[vim_core::primitives::Offset],
    scrolloff: i32,
) {
    let Some((primary, secondary)) = cursors.split_first() else {
        return;
    };
    let line_index = LineIndex::new(new_text);
    let last_byte = new_text.len().saturating_sub(1);
    let pos = |offset: &vim_core::primitives::Offset| {
        line_index.byte_to_line_col(new_text, offset.get().min(last_byte))
    };

    editor.remove_secondary_carets();

    let primary = pos(primary);
    if !editor.is_line_folded(primary.line) {
        editor.unfold_line(primary.line);
    }
    editor.set_caret_line(primary.line);
    editor.set_caret_column(primary.col);
    for cursor in secondary {
        let at = pos(cursor);
        editor.add_caret(at.line, at.col);
    }

    editor.adjust_viewport_to_caret();
    super::cursor::enforce_scrolloff(editor, scrolloff);
}

/// `U` (per-line undo) — not supported by Godot's CodeEdit.
pub(super) fn handle_undo_line(_count: u32) {
    use std::sync::atomic::{AtomicBool, Ordering};
    static LOGGED: AtomicBool = AtomicBool::new(false);
    if !LOGGED.swap(true, Ordering::Relaxed) {
        log::info!("U (undo line) not supported — CodeEdit provides only global undo");
    }
}
