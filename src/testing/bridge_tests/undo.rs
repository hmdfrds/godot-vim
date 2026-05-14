//! Undo/redo tests for the changeset-based `UndoStore` model.
//!
//! Each test uses `DispatchCtx` to drive the full dispatch pipeline:
//! `begin_undo` captures T0 text, text effects mutate the editor,
//! `end_undo(node_id)` computes and stores forward/inverse changesets,
//! and `undo_steps(node_id, cursor)` / `redo_steps(node_id, cursor)`
//! apply the stored changesets to restore/re-apply text.

use super::macros::DispatchCtx;
use crate::bridge::port::TextEditorPort;
use crate::testing::MockTextEdit;
use smallvec::smallvec;
use vim_core::primitives::{NodeId, Offset, UndoNavStep};

// ── Single insert: undo restores original text ──────────────────────────

#[test]
fn undo_single_insert_via_changeset() {
    let mut mock = MockTextEdit::new("hello");
    let mut ctx = DispatchCtx::new();

    // Insert " world" at offset 5 inside an undo group tagged as node 1.
    let fx = effects![
        begin_undo;
        insert(5, " world");
        set_cursor(11);
        end_undo(1)
    ];
    ctx.dispatch(&mut mock, fx);
    assert_editor!(mock, text: "hello world");

    // Undo node 1: should restore "hello".
    ctx.dispatch(&mut mock, effects![undo_steps(1, 0)]);
    assert_editor!(mock, text: "hello");
}

// ── Redo after undo ─────────────────────────────────────────────────────

#[test]
fn redo_after_undo_via_changeset() {
    let mut mock = MockTextEdit::new("hello");
    let mut ctx = DispatchCtx::new();

    let fx = effects![
        begin_undo;
        insert(5, " world");
        set_cursor(11);
        end_undo(1)
    ];
    ctx.dispatch(&mut mock, fx);
    assert_editor!(mock, text: "hello world");

    // Undo
    ctx.dispatch(&mut mock, effects![undo_steps(1, 0)]);
    assert_editor!(mock, text: "hello");

    // Redo
    ctx.dispatch(&mut mock, effects![redo_steps(1, 0)]);
    assert_editor!(mock, text: "hello world");
}

// ── Delete: undo restores deleted text ──────────────────────────────────

#[test]
fn undo_delete_via_changeset() {
    let mut mock = MockTextEdit::new("hello world");
    let mut ctx = DispatchCtx::new();

    // Delete " world" (bytes 5..11).
    let fx = effects![
        begin_undo;
        delete(5, 11);
        set_cursor(4);
        end_undo(1)
    ];
    ctx.dispatch(&mut mock, fx);
    assert_editor!(mock, text: "hello");

    // Undo: should restore "hello world".
    ctx.dispatch(&mut mock, effects![undo_steps(1, 0)]);
    assert_editor!(mock, text: "hello world");
}

// ── Replace: undo restores original text ────────────────────────────────

#[test]
fn undo_replace_via_changeset() {
    let mut mock = MockTextEdit::new("the quick brown fox");
    let mut ctx = DispatchCtx::new();

    // Replace "quick" (bytes 4..9) with "slow".
    let fx = effects![
        begin_undo;
        replace(4, 9, "slow");
        set_cursor(7);
        end_undo(1)
    ];
    ctx.dispatch(&mut mock, fx);
    assert_editor!(mock, text: "the slow brown fox");

    // Undo: restore "the quick brown fox".
    ctx.dispatch(&mut mock, effects![undo_steps(1, 0)]);
    assert_editor!(mock, text: "the quick brown fox");
}

// ── Multiple independent undo groups ────────────────────────────────────

#[test]
fn multiple_undo_groups_independent() {
    let mut mock = MockTextEdit::new("aaa");
    let mut ctx = DispatchCtx::new();

    // Group 1 (node 1): append " bbb"
    let fx = effects![
        begin_undo;
        insert(3, " bbb");
        set_cursor(7);
        end_undo(1)
    ];
    ctx.dispatch(&mut mock, fx);
    assert_editor!(mock, text: "aaa bbb");

    // Group 2 (node 2): append " ccc"
    let fx = effects![
        begin_undo;
        insert(7, " ccc");
        set_cursor(11);
        end_undo(2)
    ];
    ctx.dispatch(&mut mock, fx);
    assert_editor!(mock, text: "aaa bbb ccc");

    // Undo group 2 only: "aaa bbb ccc" -> "aaa bbb"
    ctx.dispatch(&mut mock, effects![undo_steps(2, 0)]);
    assert_editor!(mock, text: "aaa bbb");

    // Undo group 1: "aaa bbb" -> "aaa"
    ctx.dispatch(&mut mock, effects![undo_steps(1, 0)]);
    assert_editor!(mock, text: "aaa");

    // Redo group 1: "aaa" -> "aaa bbb"
    ctx.dispatch(&mut mock, effects![redo_steps(1, 0)]);
    assert_editor!(mock, text: "aaa bbb");

    // Redo group 2: "aaa bbb" -> "aaa bbb ccc"
    ctx.dispatch(&mut mock, effects![redo_steps(2, 0)]);
    assert_editor!(mock, text: "aaa bbb ccc");
}

// ── Undo no-op when no matching node ────────────────────────────────────

#[test]
fn undo_noop_when_no_matching_node() {
    let mut mock = MockTextEdit::new("hello");
    let mut ctx = DispatchCtx::new();

    // Record an edit under node 1.
    let fx = effects![
        begin_undo;
        insert(5, " world");
        set_cursor(11);
        end_undo(1)
    ];
    ctx.dispatch(&mut mock, fx);
    assert_editor!(mock, text: "hello world");

    // Undo with a non-existent node ID (99) — should be a no-op.
    ctx.dispatch(&mut mock, effects![undo_steps(99, 0)]);
    assert_editor!(mock, text: "hello world");
}

// ── Identity edit: undo is a no-op ─────────────────────────────────────

#[test]
fn identity_edit_undo_is_noop() {
    let mut mock = MockTextEdit::new("hello");
    let mut ctx = DispatchCtx::new();

    // Begin and end an undo group without any text changes.
    let fx = effects![
        begin_undo;
        end_undo(1)
    ];
    ctx.dispatch(&mut mock, fx);
    assert_editor!(mock, text: "hello");

    // Undo the identity edit — text should remain unchanged.
    ctx.dispatch(&mut mock, effects![undo_steps(1, 0)]);
    assert_editor!(mock, text: "hello");
}

// ── Empty undo group (EndUndoGroup with None node_id) ──────────────────

#[test]
fn empty_undo_group_no_snapshot() {
    let mut mock = MockTextEdit::new("hello");
    let mut ctx = DispatchCtx::new();

    // Dispatch begin_undo + end_undo (bare, no node_id = produces None).
    // This exercises the EndUndoGroup { node_id: None } path which
    // discards the pending text without creating a snapshot.
    let fx = effects![
        begin_undo;
        end_undo
    ];
    ctx.dispatch(&mut mock, fx);
    assert_editor!(mock, text: "hello");

    // No snapshot was stored, so undoing should be a no-op.
    // Use undo(1) with empty steps — this is the old form that exercises
    // the empty-steps-vector path in dispatch.
    ctx.dispatch(&mut mock, effects![undo(1)]);
    assert_editor!(mock, text: "hello");
}

// ── Undo with empty steps vector is a no-op ────────────────────────────

#[test]
fn undo_with_empty_steps_is_noop() {
    let mut mock = MockTextEdit::new("hello world");
    let mut ctx = DispatchCtx::new();

    // Record an edit so there IS something in the undo store.
    let fx = effects![
        begin_undo;
        insert(5, "!");
        set_cursor(6);
        end_undo(1)
    ];
    ctx.dispatch(&mut mock, fx);
    assert_editor!(mock, text: "hello! world");

    // Dispatch Undo with empty steps vector — the `for step in &steps`
    // loop body never executes, so text is unchanged.
    ctx.dispatch(&mut mock, effects![undo(1)]);
    assert_editor!(mock, text: "hello! world");
}

// ── Undo restores multi-cursor positions ───────────────────────────────

#[test]
fn undo_restores_multi_cursor_positions() {
    let mut mock = MockTextEdit::new("aaa\nbbb\nccc");
    let mut ctx = DispatchCtx::new();

    // Record an edit under node 1.
    let fx = effects![
        begin_undo;
        replace(0, 3, "AAA");
        set_cursor(3);
        end_undo(1)
    ];
    ctx.dispatch(&mut mock, fx);
    assert_editor!(mock, text: "AAA\nbbb\nccc");

    // Construct an Undo effect with multiple cursors in the step.
    // After undo, cursor 0 should go to offset 0 (line 0 col 0)
    // and cursor 1 should go to offset 4 (line 1 col 0).
    //
    // Use dispatch_multi with cursor_count=2 because the post-dispatch
    // cleanup removes secondary carets when cursor_count is 1 (matching
    // production behavior: multi-cursor undo only triggers when multi-cursor
    // mode is active).
    let undo_fx = vec![vim_core::effects::Effect::Undo {
        count: 1,
        steps: vec![UndoNavStep {
            node_id: NodeId::new(1),
            cursors: smallvec![Offset::new(0), Offset::new(4)],
        }],
    }];
    ctx.dispatch_multi(&mut mock, undo_fx, 2);

    // Text should be restored to original.
    assert_editor!(mock, text: "aaa\nbbb\nccc");

    // Primary caret at offset 0 -> line 0, col 0.
    assert_eq!(mock.get_caret_line(), 0);
    assert_eq!(mock.get_caret_column(), 0);

    // Secondary caret at offset 4 -> line 1, col 0.
    assert_eq!(mock.get_caret_count(), 2);
    assert_eq!(mock.get_caret_line_for(1), 1);
    assert_eq!(mock.get_caret_column_for(1), 0);
}

// ── Undo clamps cursor beyond text length ──────────────────────────────

#[test]
fn undo_clamps_cursor_beyond_text_length() {
    let mut mock = MockTextEdit::new("ab");
    let mut ctx = DispatchCtx::new();

    // Record an edit under node 1.
    let fx = effects![
        begin_undo;
        insert(2, "cd");
        set_cursor(4);
        end_undo(1)
    ];
    ctx.dispatch(&mut mock, fx);
    assert_editor!(mock, text: "abcd");

    // Construct Undo with cursor offset = 9999, way beyond text "ab".
    let undo_fx = vec![vim_core::effects::Effect::Undo {
        count: 1,
        steps: vec![UndoNavStep {
            node_id: NodeId::new(1),
            cursors: smallvec![Offset::new(9999)],
        }],
    }];
    ctx.dispatch(&mut mock, undo_fx);

    // Text should be restored to "ab".
    assert_editor!(mock, text: "ab");

    // Cursor should be clamped to a valid position within the text.
    // restore_cursors clamps to text_len.saturating_sub(1) = 1,
    // which is line 0, col 1.
    let line = mock.get_caret_line();
    let col = mock.get_caret_column();
    assert!(line >= 0, "caret line should be non-negative");
    assert!(
        col <= 2,
        "caret column should be within text bounds, got {}",
        col
    );
    // The clamped offset is min(9999, 2-1) = 1 -> line 0, col 1.
    assert_eq!(line, 0);
    assert_eq!(col, 1);
}
