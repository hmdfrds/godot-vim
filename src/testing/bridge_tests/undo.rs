//! Undo/redo tests for the changeset-based `UndoStore` model.
//!
//! Each test uses `DispatchCtx` to drive the full dispatch pipeline:
//! `begin_undo` captures T0 text, text effects mutate the editor,
//! `end_undo(node_id)` computes and stores forward/inverse changesets,
//! and `undo_steps(node_id, cursor)` / `redo_steps(node_id, cursor)`
//! apply the stored changesets to restore/re-apply text.

use super::macros::DispatchCtx;
use crate::bridge::port::{FoldCapable, TextEditorPort};
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

/// No Godot step, but the step's cursor still lands.
#[test]
fn identity_undo_still_restores_the_cursor() {
    let mut mock = MockTextEdit::new("hello");
    let mut ctx = DispatchCtx::new();
    ctx.dispatch(&mut mock, effects![begin_undo; end_undo(1)]);
    mock.set_caret_column(4);

    ctx.dispatch(&mut mock, effects![undo_steps(1, 2)]);
    assert_editor!(mock, text: "hello", cursor: (0, 2));
    assert!(!mock.has_undo());
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

// ── An empty Insert (`x`, `i<Esc>`, `u`) in both of vim-core's shapes ─────

/// `x` on "hello": node 1.
fn x_on_hello(ctx: &mut DispatchCtx, mock: &mut MockTextEdit) {
    let fx = effects![
        begin_undo;
        delete(0, 1);
        set_cursor(0);
        end_undo(1)
    ];
    ctx.dispatch(mock, fx);
    assert_editor!(mock, text: "ello");
}

/// vim-core v0.9.1 commits an empty Insert as a node of its own.
#[test]
fn empty_insert_as_a_node_with_no_change() {
    let mut mock = MockTextEdit::new("hello");
    let mut ctx = DispatchCtx::new();
    x_on_hello(&mut ctx, &mut mock);

    ctx.dispatch(&mut mock, effects![begin_undo; end_undo(2)]);
    assert!(!ctx.undo_pending());

    ctx.dispatch(&mut mock, effects![undo_steps(2, 0)]);
    assert_editor!(mock, text: "ello");
    ctx.dispatch(&mut mock, effects![undo_steps(1, 0)]);
    assert_editor!(mock, text: "hello");
}

/// The shape vim-core v0.10.0 sends: no node, `EndUndoGroup { node_id: None }`.
#[test]
fn empty_insert_with_no_node() {
    let mut mock = MockTextEdit::new("hello");
    let mut ctx = DispatchCtx::new();
    x_on_hello(&mut ctx, &mut mock);

    ctx.dispatch(&mut mock, effects![begin_undo; end_undo]);
    assert!(!ctx.undo_pending());

    ctx.dispatch(&mut mock, effects![undo_steps(1, 0)]);
    assert_editor!(mock, text: "hello");
}

/// The controller's engine sends that shape for `i<Esc>`, and its `u` then
/// undoes the `x`.
#[test]
fn empty_insert_from_the_controllers_engine() {
    use vim_core::effects::Effect;
    use vim_core::execution::InputContext;
    use vim_core::keymap::KeyEvent;
    let mut controller = crate::controller::VimController::new();
    let mut mock = MockTextEdit::new("hello");
    let mut ctx = DispatchCtx::new();
    let mut ends = Vec::new();
    for key in [
        KeyEvent::char('x'),
        KeyEvent::char('i'),
        KeyEvent::escape(),
        KeyEvent::char('u'),
    ] {
        let text = mock.get_text();
        let doc = crate::bridge::document::GodotDocument::new(&text);
        let input = InputContext::new(&doc, 0).validate_clamped();
        let effects = controller.engine_mut().process(key, input).take_effects();
        ends.extend(effects.iter().filter_map(|effect| match effect {
            Effect::EndUndoGroup { node_id } => Some(node_id.is_some()),
            _ => None,
        }));
        ctx.dispatch(&mut mock, effects);
    }
    assert_eq!(ends, [true, false], "x commits a node, i<Esc> none");
    assert!(!ctx.undo_pending());
    assert_editor!(mock, text: "hello");
}

/// An empty pair inside an open Insert session leaves the session's start.
#[test]
fn empty_pair_inside_a_session_keeps_its_start() {
    let mut mock = MockTextEdit::new("hello");
    let mut ctx = DispatchCtx::new();

    ctx.dispatch(&mut mock, effects![begin_undo; insert(5, " world")]);
    ctx.dispatch(&mut mock, effects![begin_undo; end_undo]);
    assert!(ctx.undo_pending());
    ctx.dispatch(&mut mock, effects![insert(11, "!"); end_undo(1)]);
    assert_editor!(mock, text: "hello world!");

    ctx.dispatch(&mut mock, effects![undo_steps(1, 0)]);
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
    // place_undo_caret clamps to text_len.saturating_sub(1) = 1,
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

// ── A change that changes nothing reaches Godot as nothing ─────────────

#[test]
fn noop_change_pushes_no_op() {
    for fx in [
        effects![begin_undo; replace(0, 3, "abc"); end_undo(1)],
        effects![begin_undo; insert(1, ""); end_undo(1)],
        effects![begin_undo; delete(2, 2); end_undo(1)],
        effects![begin_undo; insert(1, "x"); delete(1, 2); end_undo(1)],
    ] {
        let mut mock = MockTextEdit::new("abc");
        let mut ctx = DispatchCtx::new();
        ctx.dispatch(&mut mock, fx);
        assert_editor!(mock, text: "abc");
        assert!(!mock.has_undo());
    }
}

/// A replacement keeps its own Godot op even when part of it is unchanged.
#[test]
fn partial_replace_is_one_godot_step() {
    let mut mock = MockTextEdit::new("abc");
    let mut ctx = DispatchCtx::new();
    ctx.dispatch(
        &mut mock,
        effects![begin_undo; replace(0, 3, "abd"); end_undo(1)],
    );
    assert_editor!(mock, text: "abd");

    mock.undo();
    assert_editor!(mock, text: "abc");
    assert!(!mock.has_undo());
}

// ── The caret after u: folds and scrolloff ──────────────────────────────

fn numbered(lines: usize) -> String {
    (0..lines)
        .map(|i| format!("line {i}"))
        .collect::<Vec<_>>()
        .join("\n")
}

fn offset_of(mock: &MockTextEdit, line: i32) -> usize {
    let text = mock.get_text();
    crate::bridge::codec::LineIndex::new(&text).line_col_to_byte(&text, line, 0)
}

/// `x` at the start of `line` as undo node 1, then `fold` closes and the
/// view goes back to the top, so `u` must find the line again.
fn change_then_leave(lines: usize, fold: (i32, i32), line: i32) -> (MockTextEdit, DispatchCtx) {
    let mut mock = MockTextEdit::new(&numbered(lines));
    mock.set_visible_line_count(15);
    mock.add_fold_region(fold.0, fold.1);
    let mut ctx = DispatchCtx::new().with_scrolloff(5);
    let at = offset_of(&mock, line);
    ctx.dispatch(
        &mut mock,
        effects![begin_undo; delete(at, at + 1); set_cursor(at); end_undo(1)],
    );
    mock.fold_line(fold.0);
    mock.set_caret_line(0);
    mock.set_v_scroll(0.0);
    (mock, ctx)
}

/// Godot's own undo opens a fold that hides a caret (code_edit.cpp
/// `_unhide_carets`); `u` does the same and keeps `scrolloff`.
#[test]
fn u_into_closed_fold_opens_it() {
    let (mut mock, mut ctx) = change_then_leave(40, (5, 14), 12);
    let at = offset_of(&mock, 12);
    ctx.dispatch(&mut mock, effects![undo_steps(1, at)]);
    assert_eq!(mock.get_line(12), "line 12");
    assert_editor!(mock, cursor: (12, 0));
    assert!(mock.folded_lines().is_empty());
    // Row 12 of 15 is past the bottom margin of 5: the caret goes to row 9.
    assert_eq!(mock.get_first_visible_line(), 3);
}

/// A folded header is drawn outside its fold, so a change on it shows
/// without opening the fold, and Ctrl+Z leaves it closed too.
#[test]
fn u_on_folded_header_keeps_it_closed() {
    let (mut mock, mut ctx) = change_then_leave(40, (5, 14), 5);
    assert_eq!(mock.folded_lines(), [5]);
    let at = offset_of(&mock, 5);
    ctx.dispatch(&mut mock, effects![undo_steps(1, at)]);
    assert_eq!(mock.get_line(5), "line 5");
    assert_editor!(mock, cursor: (5, 0));
    assert_eq!(mock.folded_lines(), [5]);
}

/// The margin counts screen rows: the closed fold above is one row.
#[test]
fn u_below_closed_fold_keeps_scrolloff_in_rows() {
    let (mut mock, mut ctx) = change_then_leave(60, (5, 14), 40);
    let at = offset_of(&mock, 40);
    ctx.dispatch(&mut mock, effects![undo_steps(1, at)]);
    assert_editor!(mock, cursor: (40, 0));
    assert_eq!(mock.folded_lines(), [5]);
    // Line 40 is row 31; on screen row 9 the top row is 22, line 31.
    assert_eq!(mock.get_first_visible_line(), 31);
}

#[test]
fn u_above_the_view_keeps_scrolloff() {
    let (mut mock, mut ctx) = change_then_leave(60, (50, 55), 20);
    mock.set_caret_line(45);
    mock.set_v_scroll(40.0);
    let at = offset_of(&mock, 20);
    ctx.dispatch(&mut mock, effects![undo_steps(1, at)]);
    assert_editor!(mock, cursor: (20, 0));
    assert_eq!(mock.get_first_visible_line(), 15);
}
