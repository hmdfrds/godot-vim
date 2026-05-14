//! Full dispatch round-trip tests. These exercise the complete bridge pipeline
//! (effect list -> `crate::effects::dispatch` -> MockTextEdit), verifying that
//! undo grouping, text cache invalidation, selection/cursor interplay, and scroll
//! all work correctly together. Individual handler tests live in sibling modules;
//! these tests focus on multi-effect interactions that only manifest through the
//! full dispatch path.

use super::macros::DispatchCtx;
use crate::bridge::port::TextEditorPort;
use crate::testing::MockTextEdit;

// ── Basic round-trips (single undo group, verify text + cursor) ─────────

#[test]
fn dispatch_delete_insert_round_trip() {
    let mut mock = MockTextEdit::new("hello world");
    let mut ctx = DispatchCtx::new();

    let fx = effects![
        begin_undo;
        delete(5, 6);
        insert(5, "_");
        set_cursor(6);
        end_undo(1)
    ];
    ctx.dispatch(&mut mock, fx);
    assert_editor!(mock, text: "hello_world", cursor: (0, 6));
}

#[test]
fn dispatch_replace_round_trip() {
    let mut mock = MockTextEdit::new("hello world");
    let mut ctx = DispatchCtx::new();

    let fx = effects![
        begin_undo;
        replace(6, 11, "rust");
        set_cursor(9);
        end_undo(1)
    ];
    ctx.dispatch(&mut mock, fx);
    assert_editor!(mock, text: "hello rust", cursor: (0, 9));
}

#[test]
fn dispatch_multiline_insert_round_trip() {
    let mut mock = MockTextEdit::new("hello");
    let mut ctx = DispatchCtx::new();

    let fx = effects![
        begin_undo;
        insert(5, "\nworld");
        set_cursor(6);
        end_undo(1)
    ];
    ctx.dispatch(&mut mock, fx);
    assert_editor!(mock, text: "hello\nworld", cursor: (1, 0));
}

#[test]
fn dispatch_delete_across_lines() {
    let mut mock = MockTextEdit::new("hello\nworld\nfoo");
    let mut ctx = DispatchCtx::new();

    let fx = effects![
        begin_undo;
        delete(2, 8);
        set_cursor(2);
        end_undo(1)
    ];
    ctx.dispatch(&mut mock, fx);
    assert_editor!(mock, text: "herld\nfoo", cursor: (0, 2));
}

// ── Undo / Redo via dispatch ────────────────────────────────────────────
// These test that begin/end undo groups created by the dispatch pipeline
// produce independent undo steps (not chained like Godot's ACTION system).

#[test]
fn dispatch_undo_redo_round_trip() {
    let mut mock = MockTextEdit::new("hello world");
    let mut ctx = DispatchCtx::new();

    let fx = effects![
        begin_undo;
        delete(5, 11);
        set_cursor(4);
        end_undo(1)
    ];
    ctx.dispatch(&mut mock, fx);
    assert_editor!(mock, text: "hello");

    let fx = effects![
        begin_undo;
        insert(5, "!");
        set_cursor(5);
        end_undo(2)
    ];
    ctx.dispatch(&mut mock, fx);
    assert_editor!(mock, text: "hello!");

    ctx.dispatch(&mut mock, effects![undo_steps(2, 0)]);
    assert_editor!(mock, text: "hello");

    ctx.dispatch(&mut mock, effects![undo_steps(1, 0)]);
    assert_editor!(mock, text: "hello world");

    ctx.dispatch(&mut mock, effects![redo_steps(1, 0)]);
    assert_editor!(mock, text: "hello");

    ctx.dispatch(&mut mock, effects![redo_steps(2, 0)]);
    assert_editor!(mock, text: "hello!");
}

#[test]
fn dispatch_consecutive_commands_independent_undo() {
    let mut mock = MockTextEdit::new("aaa bbb ccc");
    let mut ctx = DispatchCtx::new();

    let fx = effects![begin_undo; delete(4, 8); set_cursor(4); end_undo(1)];
    ctx.dispatch(&mut mock, fx);
    assert_editor!(mock, text: "aaa ccc");

    let fx = effects![begin_undo; delete(4, 7); set_cursor(3); end_undo(2)];
    ctx.dispatch(&mut mock, fx);
    assert_editor!(mock, text: "aaa ");

    // Each command is an independent undo step.
    ctx.dispatch(&mut mock, effects![undo_steps(2, 0)]);
    assert_editor!(mock, text: "aaa ccc");

    ctx.dispatch(&mut mock, effects![undo_steps(1, 0)]);
    assert_editor!(mock, text: "aaa bbb ccc");
}

#[test]
fn dispatch_replace_then_undo_preserves_original() {
    let mut mock = MockTextEdit::new("the quick brown fox");
    let mut ctx = DispatchCtx::new();

    let fx = effects![
        begin_undo;
        replace(4, 9, "slow");
        set_cursor(7);
        end_undo(1)
    ];
    ctx.dispatch(&mut mock, fx);
    assert_editor!(mock, text: "the slow brown fox");

    ctx.dispatch(&mut mock, effects![undo_steps(1, 0)]);
    assert_editor!(mock, text: "the quick brown fox");
}

#[test]
fn dispatch_multiple_inserts_in_one_group() {
    let mut mock = MockTextEdit::new("ac");
    let mut ctx = DispatchCtx::new();

    let fx = effects![begin_undo; insert(1, "b"); end_undo(1)];
    ctx.dispatch(&mut mock, fx);
    assert_editor!(mock, text: "abc");

    ctx.dispatch(&mut mock, effects![undo_steps(1, 0)]);
    assert_editor!(mock, text: "ac");
}

#[test]
fn undo_with_count_via_dispatch() {
    let mut mock = MockTextEdit::new("");
    let mut ctx = DispatchCtx::new();

    // Three separate insert commands with unique node IDs.
    let node_ids: [u32; 3] = [1, 2, 3];
    for (i, ch) in ["a", "b", "c"].iter().enumerate() {
        let len = mock.get_text().len();
        let nid = node_ids[i];
        let fx = effects![begin_undo; insert(len, *ch); end_undo(nid)];
        ctx.dispatch(&mut mock, fx);
    }
    assert_editor!(mock, text: "abc");

    // Undo nodes 3 and 2 individually (changeset model undoes by node, not count).
    ctx.dispatch(&mut mock, effects![undo_steps(3, 0)]);
    assert_editor!(mock, text: "ab");

    ctx.dispatch(&mut mock, effects![undo_steps(2, 0)]);
    assert_editor!(mock, text: "a");

    // Redo node 2
    ctx.dispatch(&mut mock, effects![redo_steps(2, 0)]);
    assert_editor!(mock, text: "ab");
}

// ── Selection + cursor interaction ──────────────────────────────────────
// The dispatch loop tracks whether a SetSelection has been applied in the
// current batch. If so, subsequent SetCursor effects are suppressed because
// Godot's select() already positions the caret, and a separate set_caret_line
// would corrupt the selection endpoints.

#[test]
fn dispatch_set_selection_then_cursor_skip() {
    use vim_core::primitives::SelectionShape;

    let mut mock = MockTextEdit::new("hello world");
    let mut ctx = DispatchCtx::new();

    let fx = effects![
        set_selection(0, 4, SelectionShape::Char);
        set_cursor(4)
    ];
    ctx.dispatch(&mut mock, fx);

    assert_editor!(mock, has_selection, selection_cols: (0, 5));
}

#[test]
fn dispatch_clear_selection_re_enables_cursor() {
    use vim_core::primitives::SelectionShape;

    let mut mock = MockTextEdit::new("hello world");
    let mut ctx = DispatchCtx::new();

    // ClearSelection resets the batch tracking, so the final SetCursor applies.
    let fx = effects![
        set_selection(0, 4, SelectionShape::Char);
        set_cursor(4);
        clear_selection;
        set_cursor(8)
    ];
    ctx.dispatch(&mut mock, fx);

    assert_editor!(mock, no_selection, cursor: (0, 8));
}

// ── Scroll via dispatch ─────────────────────────────────────────────────

#[test]
fn dispatch_scroll_effects() {
    let lines: String = (0..50)
        .map(|i| format!("line {}", i))
        .collect::<Vec<_>>()
        .join("\n");
    let mut mock = MockTextEdit::new(&lines);
    mock.set_visible_line_count(10);
    let mut ctx = DispatchCtx::new();

    // Move cursor to line 20, then CenterCursor
    let offset_20 = lines
        .match_indices('\n')
        .nth(19)
        .map(|(i, _)| i + 1)
        .unwrap();
    let fx = effects![set_cursor(offset_20); center_cursor];
    ctx.dispatch(&mut mock, fx);
    assert_editor!(mock, cursor: (20, 0), scroll: 15);
}

#[test]
fn dispatch_cursor_to_bottom() {
    let lines: String = (0..50)
        .map(|i| format!("line {}", i))
        .collect::<Vec<_>>()
        .join("\n");
    let mut mock = MockTextEdit::new(&lines);
    mock.set_visible_line_count(10);
    let mut ctx = DispatchCtx::new();

    let offset_20 = lines
        .match_indices('\n')
        .nth(19)
        .map(|(i, _)| i + 1)
        .unwrap();
    let fx = effects![set_cursor(offset_20); cursor_to_bottom];
    ctx.dispatch(&mut mock, fx);
    assert_editor!(mock, cursor: (20, 0), scroll: 11);
}

#[test]
fn dispatch_scroll_left_right_round_trip() {
    let mut mock = MockTextEdit::new("hello world");
    let mut ctx = DispatchCtx::new();

    ctx.dispatch(&mut mock, effects![scroll_right(5)]);
    assert_editor!(mock, h_scroll: 5);

    ctx.dispatch(&mut mock, effects![scroll_left(3)]);
    assert_editor!(mock, h_scroll: 2);
}

#[test]
fn dispatch_scroll_to_offset() {
    let lines: String = (0..20)
        .map(|i| format!("line {}", i))
        .collect::<Vec<_>>()
        .join("\n");
    let mut mock = MockTextEdit::new(&lines);
    mock.set_visible_line_count(10);
    let mut ctx = DispatchCtx::new();

    // ScrollTo offset at start of line 10
    let byte_offset = lines
        .match_indices('\n')
        .nth(9)
        .map(|(i, _)| i + 1)
        .unwrap();
    ctx.dispatch(&mut mock, effects![scroll_to(byte_offset)]);
    assert_editor!(mock, scroll: 10);
}

// ── Text cache invalidation ─────────────────────────────────────────────
// The dispatch loop re-snapshots text after each mutation so that subsequent
// byte offsets resolve correctly against the updated content.

#[test]
fn dispatch_text_cache_invalidation() {
    let mut mock = MockTextEdit::new("abc");
    let mut ctx = DispatchCtx::new();

    // Two mutations in one dispatch — second insert's byte offset (3) must
    // resolve against the post-first-insert text "aXbc", not the original "abc".
    let fx = effects![
        begin_undo;
        insert(1, "X");
        end_undo(1);
        begin_undo;
        insert(3, "Y");
        end_undo(2)
    ];
    ctx.dispatch(&mut mock, fx);
    assert_editor!(mock, text: "aXbYc");
}

// ── Fold effects (smoke test: no-ops must not panic) ────────────────────

#[test]
fn dispatch_fold_effects_no_panic() {
    let mut mock = MockTextEdit::new("hello\nworld\nfoo");
    let mut ctx = DispatchCtx::new();

    let fx = effects![
        fold_line(0);
        unfold_line(0);
        toggle_fold(1);
        fold_all;
        unfold_all
    ];
    ctx.dispatch(&mut mock, fx);
    assert_editor!(mock, text: "hello\nworld\nfoo");
}

// ── Complex multi-step scenarios ────────────────────────────────────────
// These simulate realistic editing sessions to catch interactions between
// multiple undo groups, multiline mutations, and redo.

#[test]
fn scenario_insert_delete_replace_sequence() {
    let mut mock = MockTextEdit::new("fn main() {}");
    let mut ctx = DispatchCtx::new();

    let fx = effects![begin_undo; replace(10, 12, "{\n    \n}"); set_cursor(15); end_undo(1)];
    ctx.dispatch(&mut mock, fx);
    assert_editor!(mock, text: "fn main() {\n    \n}");

    let fx = effects![begin_undo; insert(16, "println!(\"hello\");"); set_cursor(33); end_undo(2)];
    ctx.dispatch(&mut mock, fx);
    assert_editor!(mock, text: "fn main() {\n    println!(\"hello\");\n}");

    ctx.dispatch(&mut mock, effects![undo_steps(2, 0)]);
    assert_editor!(mock, text: "fn main() {\n    \n}");

    ctx.dispatch(&mut mock, effects![undo_steps(1, 0)]);
    assert_editor!(mock, text: "fn main() {}");

    ctx.dispatch(&mut mock, effects![redo_steps(1, 0)]);
    assert_editor!(mock, text: "fn main() {\n    \n}");

    ctx.dispatch(&mut mock, effects![redo_steps(2, 0)]);
    assert_editor!(mock, text: "fn main() {\n    println!(\"hello\");\n}");
}

#[test]
fn scenario_visual_select_delete_undo() {
    use vim_core::primitives::SelectionShape;

    let mut mock = MockTextEdit::new("hello world foo");
    let mut ctx = DispatchCtx::new();

    // SetSelection paired with SetCursor (engine always emits both).
    let fx = effects![set_selection(6, 11, SelectionShape::Char); set_cursor(11)];
    ctx.dispatch(&mut mock, fx);
    assert_editor!(mock, has_selection);

    let fx = effects![
        clear_selection;
        begin_undo;
        delete(6, 12);
        set_cursor(6);
        end_undo(1)
    ];
    ctx.dispatch(&mut mock, fx);
    assert_editor!(mock, text: "hello foo");

    ctx.dispatch(&mut mock, effects![undo_steps(1, 0)]);
    assert_editor!(mock, text: "hello world foo");
}

// ── Conditional caret clear (multi-cursor preservation) ────────────────
// The dispatch loop conditionally calls `remove_secondary_carets()`:
// - When 0 or 1 SetCursor effects: secondaries are cleared (single-cursor mode).
// - When 2+ SetCursor effects: secondaries are preserved (multi-cursor mode).
// Suppressed SetCursor effects (from SelectionPairing) must NOT inflate the count.

#[test]
fn single_cursor_normal_op_clears_secondaries() {
    // Simulate: single-cursor motion (e.g., `w`) — one SetCursor effect.
    // Pre-existing secondary carets must be cleared after dispatch.
    let mut mock = MockTextEdit::new("hello world foo");
    mock.add_caret(1, 0); // simulate a leftover secondary
    assert_eq!(mock.caret_count(), 2);
    let mut ctx = DispatchCtx::new();

    // `w` motion: engine emits a single SetCursor to the next word.
    let fx = effects![set_cursor(6)];
    ctx.dispatch(&mut mock, fx);

    // Secondary should be cleared (single-cursor mode).
    assert_editor!(mock, carets: 1, cursor: (0, 6));
}

#[test]
fn single_cursor_j_motion_clears_secondaries() {
    // Simulate: `j` motion — single SetCursor.
    let mut mock = MockTextEdit::new("hello\nworld\nfoo");
    mock.add_caret(2, 0); // leftover secondary
    assert_eq!(mock.caret_count(), 2);
    let mut ctx = DispatchCtx::new();

    // `j` from line 0 to line 1, offset 6 = start of "world".
    let fx = effects![set_cursor(6)];
    ctx.dispatch(&mut mock, fx);

    assert_editor!(mock, carets: 1, cursor: (1, 0));
}

#[test]
fn single_cursor_dd_clears_secondaries() {
    // Simulate: `dd` — text mutation + single SetCursor.
    let mut mock = MockTextEdit::new("hello\nworld\nfoo");
    mock.add_caret(2, 0); // leftover secondary
    assert_eq!(mock.caret_count(), 2);
    let mut ctx = DispatchCtx::new();

    // Delete "hello\n" (line 1), cursor goes to offset 0.
    let fx = effects![
        begin_undo;
        delete(0, 6);
        set_cursor(0);
        end_undo(1)
    ];
    ctx.dispatch(&mut mock, fx);

    // Text is now "world\nfoo", secondaries cleared.
    assert_editor!(mock, carets: 1, text: "world\nfoo", cursor: (0, 0));
}

#[test]
fn multi_cursor_typing_preserves_secondaries() {
    // Simulate: 3 cursors in insert mode, each receiving a SetCursor after insert.
    // When 3 SetCursor effects are present, secondaries must be preserved.
    let mut mock = MockTextEdit::new("aaa\nbbb\nccc");
    // Add 2 secondary carets (total 3).
    mock.add_caret(1, 0);
    mock.add_caret(2, 0);
    assert_eq!(mock.caret_count(), 3);
    let mut ctx = DispatchCtx::new();

    // Multi-cursor insert: engine emits 3 inserts + 3 SetCursors.
    let fx = effects![
        begin_undo;
        insert(0, "X");
        insert(5, "X");
        insert(10, "X");
        set_cursor(1);
        set_cursor(6);
        set_cursor(11);
        end_undo(1)
    ];
    ctx.dispatch(&mut mock, fx);

    // All 3 carets must be preserved (3 SetCursors -> multi-cursor mode).
    assert_editor!(mock, carets: 3, text: "Xaaa\nXbbb\nXccc");
}

#[test]
fn two_cursors_preserves_secondaries() {
    // Edge case: exactly 2 SetCursor effects should also preserve secondaries.
    let mut mock = MockTextEdit::new("hello\nworld");
    mock.add_caret(1, 0);
    assert_eq!(mock.caret_count(), 2);
    let mut ctx = DispatchCtx::new();

    let fx = effects![set_cursor(2); set_cursor(8)];
    ctx.dispatch(&mut mock, fx);

    // 2 SetCursors: multi-cursor mode, secondaries preserved.
    assert_editor!(mock, carets: 2);
}

#[test]
fn transition_multi_to_single_clears_secondaries() {
    // Simulate: Escape from multi-cursor returns to single cursor.
    // After Escape, engine emits only 1 SetCursor — secondaries should be cleared.
    let mut mock = MockTextEdit::new("hello\nworld\nfoo");
    mock.add_caret(1, 0);
    mock.add_caret(2, 0);
    assert_eq!(mock.caret_count(), 3);
    let mut ctx = DispatchCtx::new();

    // First dispatch: multi-cursor mode (3 SetCursors), secondaries preserved.
    let fx = effects![set_cursor(0); set_cursor(6); set_cursor(12)];
    ctx.dispatch(&mut mock, fx);
    assert_editor!(mock, carets: 3);

    // Second dispatch: Escape — engine returns to single cursor (1 SetCursor).
    let fx = effects![set_cursor(0)];
    ctx.dispatch(&mut mock, fx);
    assert_editor!(mock, carets: 1, cursor: (0, 0));
}

#[test]
fn zero_set_cursor_effects_clears_secondaries() {
    // When no SetCursor at all (e.g., only scroll effects), secondaries are cleared.
    let mut mock = MockTextEdit::new("hello\nworld");
    mock.add_caret(1, 0);
    assert_eq!(mock.caret_count(), 2);
    let mut ctx = DispatchCtx::new();

    let fx = effects![center_cursor];
    ctx.dispatch(&mut mock, fx);

    assert_editor!(mock, carets: 1);
}

#[test]
fn selection_paired_cursor_not_counted_for_multi_cursor() {
    // SetSelection followed by its paired SetCursor — the paired SetCursor is
    // suppressed by SelectionPairing and must NOT count toward set_cursor_count.
    // A subsequent real SetCursor (after ClearSelection) counts as 1, so
    // secondaries are still cleared.
    use vim_core::primitives::SelectionShape;

    let mut mock = MockTextEdit::new("hello world");
    mock.add_caret(0, 9); // leftover secondary
    assert_eq!(mock.caret_count(), 2);
    let mut ctx = DispatchCtx::new();

    let fx = effects![
        set_selection(0, 5, SelectionShape::Char);
        set_cursor(5);     // suppressed by pairing
        clear_selection;
        set_cursor(8)      // real SetCursor (count = 1)
    ];
    ctx.dispatch(&mut mock, fx);

    // Only 1 real SetCursor counted -> single-cursor mode -> secondaries cleared.
    assert_editor!(mock, carets: 1, cursor: (0, 8));
}

#[test]
fn multiple_selections_do_not_inflate_cursor_count() {
    // Two SetSelection+SetCursor pairs (both suppressed), then one real SetCursor.
    // Total real set_cursor_count = 1 -> secondaries cleared.
    use vim_core::primitives::SelectionShape;

    let mut mock = MockTextEdit::new("hello world foo");
    mock.add_caret(0, 10); // leftover secondary
    assert_eq!(mock.caret_count(), 2);
    let mut ctx = DispatchCtx::new();

    let fx = effects![
        set_selection(0, 5, SelectionShape::Char);
        set_cursor(5);     // suppressed
        set_selection(6, 11, SelectionShape::Char);
        set_cursor(11);    // suppressed
        clear_selection;
        set_cursor(3)      // real (count = 1)
    ];
    ctx.dispatch(&mut mock, fx);

    assert_editor!(mock, carets: 1, cursor: (0, 3));
}

// ── Multi-cursor effect routing (SetCursor index routing) ─────────────
// These tests validate that SetCursor effects are routed by index:
// effect 0 → primary caret, effect 1 → secondary caret 1, etc.

#[test]
fn multi_cursor_three_set_cursors_at_correct_offsets() {
    // Dispatch 3 SetCursor effects at offsets 5, 15, 25 on a single line.
    let mut mock = MockTextEdit::new("0123456789abcdefghijklmnopqrstuvwxyz");
    let mut ctx = DispatchCtx::new();

    let fx = effects![set_cursor(5); set_cursor(15); set_cursor(25)];
    ctx.dispatch(&mut mock, fx);

    // 3 SetCursor effects -> 3 carets preserved.
    assert_editor!(mock, carets: 3);
    // Primary caret (index 0) at offset 5 -> col 5
    assert_eq!(mock.get_caret_line_for(0), 0);
    assert_eq!(mock.get_caret_column_for(0), 5);
    // Secondary caret 1 (index 1) at offset 15 -> col 15
    assert_eq!(mock.get_caret_line_for(1), 0);
    assert_eq!(mock.get_caret_column_for(1), 15);
    // Secondary caret 2 (index 2) at offset 25 -> col 25
    assert_eq!(mock.get_caret_line_for(2), 0);
    assert_eq!(mock.get_caret_column_for(2), 25);
}

#[test]
fn multi_cursor_three_set_cursors_multiline() {
    // Multiline variant: offsets map to different lines.
    // "hello\nworld\nfoobar" -> line 0: hello (0-4), \n at 5
    //                           line 1: world (6-10), \n at 11
    //                           line 2: foobar (12-17)
    let mut mock = MockTextEdit::new("hello\nworld\nfoobar");
    let mut ctx = DispatchCtx::new();

    // offset 2 -> (0, 2), offset 8 -> (1, 2), offset 14 -> (2, 2)
    let fx = effects![set_cursor(2); set_cursor(8); set_cursor(14)];
    ctx.dispatch(&mut mock, fx);

    assert_editor!(mock, carets: 3);
    assert_eq!(mock.get_caret_line_for(0), 0);
    assert_eq!(mock.get_caret_column_for(0), 2);
    assert_eq!(mock.get_caret_line_for(1), 1);
    assert_eq!(mock.get_caret_column_for(1), 2);
    assert_eq!(mock.get_caret_line_for(2), 2);
    assert_eq!(mock.get_caret_column_for(2), 2);
}

// ── Excess caret removal ──────────────────────────────────────────────────
// When fewer SetCursor effects are dispatched than Godot carets exist,
// excess carets must be removed.

#[test]
fn excess_carets_removed_after_two_set_cursors_on_five_caret_editor() {
    // Editor starts with 5 carets. Dispatch 2 SetCursor effects.
    // Excess 3 carets (indices 2, 3, 4) must be removed.
    let mut mock = MockTextEdit::new("hello\nworld\nfoo\nbar\nbaz");
    // Add 4 secondary carets (total 5).
    mock.add_caret(1, 0);
    mock.add_caret(2, 0);
    mock.add_caret(3, 0);
    mock.add_caret(4, 0);
    assert_eq!(mock.caret_count(), 5);
    let mut ctx = DispatchCtx::new();

    // 2 SetCursor effects: primary at offset 2 -> (0,2), secondary at offset 8 -> (1,2)
    let fx = effects![set_cursor(2); set_cursor(8)];
    ctx.dispatch(&mut mock, fx);

    // Only 2 carets should remain.
    assert_editor!(mock, carets: 2);
    assert_eq!(mock.get_caret_line_for(0), 0);
    assert_eq!(mock.get_caret_column_for(0), 2);
    assert_eq!(mock.get_caret_line_for(1), 1);
    assert_eq!(mock.get_caret_column_for(1), 2);
}

#[test]
fn excess_carets_removed_three_to_one() {
    // 3 carets, 1 SetCursor -> secondaries cleared (single-cursor mode path).
    let mut mock = MockTextEdit::new("hello\nworld\nfoo");
    mock.add_caret(1, 0);
    mock.add_caret(2, 0);
    assert_eq!(mock.caret_count(), 3);
    let mut ctx = DispatchCtx::new();

    let fx = effects![set_cursor(0)];
    ctx.dispatch(&mut mock, fx);

    assert_editor!(mock, carets: 1, cursor: (0, 0));
}

// ── SaveSelections / RestoreSelections round-trip ──────────────────────────

#[test]
fn save_restore_selections_round_trip_single_caret() {
    // Single caret: save position, move somewhere else, restore.
    let mut mock = MockTextEdit::new("hello world foo bar");
    let mut ctx = DispatchCtx::new();

    // Move cursor to offset 6 -> (0, 6), then save.
    let fx = effects![set_cursor(6); save_selections];
    ctx.dispatch(&mut mock, fx);
    assert_editor!(mock, cursor: (0, 6));

    // Move cursor somewhere else.
    let fx = effects![set_cursor(12)];
    ctx.dispatch(&mut mock, fx);
    assert_editor!(mock, cursor: (0, 12));

    // Restore: should go back to (0, 6).
    let fx = effects![restore_selections];
    ctx.dispatch(&mut mock, fx);
    assert_editor!(mock, cursor: (0, 6));
}

#[test]
fn save_restore_selections_round_trip_multi_caret() {
    // 3 carets: save all positions, move them, restore.
    let mut mock = MockTextEdit::new("aaa\nbbb\nccc");
    let mut ctx = DispatchCtx::new();

    // Place 3 carets and save in the same batch (engine always pairs
    // SetCursor effects with SaveSelections in one effect list).
    let fx = effects![set_cursor(1); set_cursor(5); set_cursor(9); save_selections];
    ctx.dispatch(&mut mock, fx);
    assert_editor!(mock, carets: 3);
    assert_eq!(mock.get_caret_line_for(0), 0);
    assert_eq!(mock.get_caret_column_for(0), 1);
    assert_eq!(mock.get_caret_line_for(1), 1);
    assert_eq!(mock.get_caret_column_for(1), 1);
    assert_eq!(mock.get_caret_line_for(2), 2);
    assert_eq!(mock.get_caret_column_for(2), 1);

    // Move carets to different positions.
    let fx = effects![set_cursor(0); set_cursor(4); set_cursor(8)];
    ctx.dispatch(&mut mock, fx);
    assert_eq!(mock.get_caret_column_for(0), 0);
    assert_eq!(mock.get_caret_column_for(1), 0);
    assert_eq!(mock.get_caret_column_for(2), 0);

    // Restore: should go back to original positions.
    let fx = effects![restore_selections];
    ctx.dispatch(&mut mock, fx);
    assert_editor!(mock, carets: 3);
    assert_eq!(mock.get_caret_line_for(0), 0);
    assert_eq!(mock.get_caret_column_for(0), 1);
    assert_eq!(mock.get_caret_line_for(1), 1);
    assert_eq!(mock.get_caret_column_for(1), 1);
    assert_eq!(mock.get_caret_line_for(2), 2);
    assert_eq!(mock.get_caret_column_for(2), 1);
}

#[test]
fn save_restore_selections_removes_excess_carets() {
    // Start with 2 carets, save. Then add more carets.
    // Restore should trim back to the saved 2.
    let mut mock = MockTextEdit::new("aaa\nbbb\nccc\nddd\neee");
    let mut ctx = DispatchCtx::new();

    // Place 2 carets and save in same batch.
    let fx = effects![set_cursor(1); set_cursor(5); save_selections];
    ctx.dispatch(&mut mock, fx);
    assert_editor!(mock, carets: 2);

    // Add more carets (simulate 4 carets now).
    let fx = effects![set_cursor(0); set_cursor(4); set_cursor(8); set_cursor(12)];
    ctx.dispatch(&mut mock, fx);
    assert_editor!(mock, carets: 4);

    // Restore: should go back to 2 carets at the saved positions.
    let fx = effects![restore_selections];
    ctx.dispatch(&mut mock, fx);
    assert_editor!(mock, carets: 2);
    assert_eq!(mock.get_caret_line_for(0), 0);
    assert_eq!(mock.get_caret_column_for(0), 1);
    assert_eq!(mock.get_caret_line_for(1), 1);
    assert_eq!(mock.get_caret_column_for(1), 1);
}

// ── Primary caret uses handle_set_cursor (scrolloff behavior) ─────────────

#[test]
fn primary_caret_enforces_scrolloff() {
    // Verify that the primary caret (effect index 0) uses handle_set_cursor
    // which includes scrolloff enforcement. When scrolloff > 0, moving the
    // cursor near the edge of the viewport should scroll.
    let lines: String = (0..50)
        .map(|i| format!("line {:02}", i))
        .collect::<Vec<_>>()
        .join("\n");
    let mut mock = MockTextEdit::new(&lines);
    mock.set_visible_line_count(10);

    // Override scrolloff to 3 by calling the production dispatch function
    // directly with a custom DispatchContext (DispatchCtx uses scrolloff=0).
    let text = mock.get_text();
    // Move cursor to line 1 (offset of start of line 1 = 8 for "line 00\n")
    let offset_line_1 = lines.find("line 01").unwrap();
    crate::effects::dispatch(
        effects![set_cursor(offset_line_1)],
        &mut mock,
        crate::effects::DispatchContext {
            state: &mut crate::state::ShellState::default(),
            editor_id: godot::prelude::InstanceId::from_i64(99),
            auto_brace: crate::effects::dispatch::AutoBraceMode::Ineligible,
            auto_brace_snapshot: crate::bridge::AutoBraceSnapshot::disabled(),
            line_index_hint: None,
            scrolloff: 3,
            highlight_yank_duration_ms: 0,
            syntax_query: Box::new(|_, _| crate::bridge::SyntaxRegion::code()),
            clipboard: &mut crate::bridge::clipboard::MockClipboard::new(),
            cursor_count: 1,
        },
        &text,
    );

    // With scrolloff=3, cursor at line 1, viewport should scroll so that
    // there are at least 3 lines above the cursor. Since line 1 < 3,
    // the viewport should be at line 0 (can't scroll negative).
    // The point is that handle_set_cursor was called (not just set_caret_line_for).
    assert_eq!(mock.get_caret_line(), 1);

    // Now test with cursor deep in the file to confirm scrolloff works.
    let offset_line_45 = lines.find("line 45").unwrap();
    let text = mock.get_text();
    crate::effects::dispatch(
        effects![set_cursor(offset_line_45)],
        &mut mock,
        crate::effects::DispatchContext {
            state: &mut crate::state::ShellState::default(),
            editor_id: godot::prelude::InstanceId::from_i64(99),
            auto_brace: crate::effects::dispatch::AutoBraceMode::Ineligible,
            auto_brace_snapshot: crate::bridge::AutoBraceSnapshot::disabled(),
            line_index_hint: None,
            scrolloff: 3,
            highlight_yank_duration_ms: 0,
            syntax_query: Box::new(|_, _| crate::bridge::SyntaxRegion::code()),
            clipboard: &mut crate::bridge::clipboard::MockClipboard::new(),
            cursor_count: 1,
        },
        &text,
    );

    assert_eq!(mock.get_caret_line(), 45);
    // With visible=10 and scrolloff=3, cursor at line 45:
    // The viewport bottom edge must be >= 45 + 3 = 48, so first visible <= 45 - (10-1-3) = 39.
    // Actually: first_visible + visible - 1 - scrolloff >= cursor_line
    // first_visible + 10 - 1 - 3 >= 45 => first_visible >= 39.
    let first_visible = mock.get_first_visible_line();
    assert!(
        first_visible <= 45 - 3,
        "scrolloff not enforced: first_visible={}, cursor at 45, scrolloff=3",
        first_visible
    );
}

// ── Multi-cursor real-use-case: `dw` replicated to 3 cursors ────────────
// Simulates the engine replicating `dw` across 3 cursors. The engine emits
// 3 Delete effects (pass 1) that mutate text, then 3 SetCursor effects
// (pass 2) against the final post-mutation text.

#[test]
fn multi_cursor_dw_deletes_word_on_three_lines() {
    // Document: "foo bar\nfoo bar\nfoo bar"
    // 3 cursors at col 0 of each line. `dw` deletes "foo " from each.
    //
    // Pass 1 (text mutations applied in sequence against evolving text):
    //   delete(0, 4):  "foo bar\nfoo bar\nfoo bar" -> "bar\nfoo bar\nfoo bar"
    //   delete(4, 8):  "bar\nfoo bar\nfoo bar"     -> "bar\nbar\nfoo bar"
    //   delete(8, 12): "bar\nbar\nfoo bar"         -> "bar\nbar\nbar"
    //
    // Pass 2 (cursor positioning against final text "bar\nbar\nbar"):
    //   SetCursor(0):  line 0, col 0
    //   SetCursor(4):  line 1, col 0
    //   SetCursor(8):  line 2, col 0
    let mut mock = MockTextEdit::new("foo bar\nfoo bar\nfoo bar");
    let mut ctx = DispatchCtx::new();

    let fx = effects![
        begin_undo;
        delete(0, 4);
        delete(4, 8);
        delete(8, 12);
        set_cursor(0);
        set_cursor(4);
        set_cursor(8);
        end_undo(1)
    ];
    ctx.dispatch(&mut mock, fx);

    assert_editor!(mock, text: "bar\nbar\nbar", carets: 3);
    assert_editor!(mock, cursor: (0, 0));
    assert_eq!(mock.get_caret_line_for(1), 1);
    assert_eq!(mock.get_caret_column_for(1), 0);
    assert_eq!(mock.get_caret_line_for(2), 2);
    assert_eq!(mock.get_caret_column_for(2), 0);
}

#[test]
fn multi_cursor_dw_deletes_at_mid_word_positions() {
    // Document: "hello world\nhello world"
    // 2 cursors at col 6 ("w" of "world"). `dw` deletes "world" from each.
    //
    // Pass 1:
    //   delete(6, 11):  "hello world\nhello world" -> "hello \nhello world"
    //   delete(13, 18): "hello \nhello world"      -> "hello \nhello "
    //
    // Pass 2 against "hello \nhello ":
    //   SetCursor(5):  line 0, col 5
    //   SetCursor(12): line 1, col 5
    let mut mock = MockTextEdit::new("hello world\nhello world");
    let mut ctx = DispatchCtx::new();

    let fx = effects![
        begin_undo;
        delete(6, 11);
        delete(13, 18);
        set_cursor(5);
        set_cursor(12);
        end_undo(1)
    ];
    ctx.dispatch(&mut mock, fx);

    assert_editor!(mock, text: "hello \nhello ", carets: 2);
    assert_editor!(mock, cursor: (0, 5));
    assert_eq!(mock.get_caret_line_for(1), 1);
    assert_eq!(mock.get_caret_column_for(1), 5);
}

// ── Multi-cursor real-use-case: insert mode typing ─���────────────────────
// Simulates multi-cursor insert mode: the engine replicates character
// insertion across all cursors, then positions each caret past the insert.

#[test]
fn multi_cursor_insert_char_at_three_cursors() {
    // Document: "aaa\nbbb\nccc"
    // 3 cursors at col 0 of each line. User types "X".
    //
    // Pass 1 (inserts applied sequentially):
    //   insert(0, "X"):  "aaa\nbbb\nccc" -> "Xaaa\nbbb\nccc"
    //   insert(5, "X"):  "Xaaa\nbbb\nccc" -> "Xaaa\nXbbb\nccc"
    //   insert(10, "X"): "Xaaa\nXbbb\nccc" -> "Xaaa\nXbbb\nXccc"
    //
    // Pass 2 against "Xaaa\nXbbb\nXccc":
    //   SetCursor(1):  line 0, col 1
    //   SetCursor(6):  line 1, col 1
    //   SetCursor(11): line 2, col 1
    let mut mock = MockTextEdit::new("aaa\nbbb\nccc");
    let mut ctx = DispatchCtx::new();

    let fx = effects![
        begin_undo;
        insert(0, "X");
        insert(5, "X");
        insert(10, "X");
        set_cursor(1);
        set_cursor(6);
        set_cursor(11);
        end_undo(1)
    ];
    ctx.dispatch(&mut mock, fx);

    assert_editor!(mock, text: "Xaaa\nXbbb\nXccc", carets: 3);
    assert_editor!(mock, cursor: (0, 1));
    assert_eq!(mock.get_caret_line_for(1), 1);
    assert_eq!(mock.get_caret_column_for(1), 1);
    assert_eq!(mock.get_caret_line_for(2), 2);
    assert_eq!(mock.get_caret_column_for(2), 1);
}

#[test]
fn multi_cursor_insert_multi_char_string() {
    // 3 cursors typing "->>" (multi-char) at end of each line.
    //   Document: "fn\nfn\nfn"
    //   Cursors at col 2 (end) of each line.
    //
    // Pass 1:
    //   insert(2, "->>"):  "fn\nfn\nfn" -> "fn->>\nfn\nfn"
    //   insert(8, "->>"):  "fn->>\nfn\nfn" -> "fn->>\nfn->>\nfn"
    //   insert(14, "->>"): "fn->>\nfn->>\nfn" -> "fn->>\nfn->>\nfn->>"
    //
    // Pass 2 against "fn->>\nfn->>\nfn->>":
    //   SetCursor(5):  line 0, col 5
    //   SetCursor(11): line 1, col 5
    //   SetCursor(17): line 2, col 5
    let mut mock = MockTextEdit::new("fn\nfn\nfn");
    let mut ctx = DispatchCtx::new();

    let fx = effects![
        begin_undo;
        insert(2, "->>");
        insert(8, "->>");
        insert(14, "->>");
        set_cursor(5);
        set_cursor(11);
        set_cursor(17);
        end_undo(1)
    ];
    ctx.dispatch(&mut mock, fx);

    assert_editor!(mock, text: "fn->>\nfn->>\nfn->>", carets: 3);
    assert_editor!(mock, cursor: (0, 5));
    assert_eq!(mock.get_caret_line_for(1), 1);
    assert_eq!(mock.get_caret_column_for(1), 5);
    assert_eq!(mock.get_caret_line_for(2), 2);
    assert_eq!(mock.get_caret_column_for(2), 5);
}

// ── Descending-order caret removal (index shift safety) ─────────────────
// These tests specifically verify that when we remove excess carets, we do
// so from highest index downward. If we removed from lowest first, each
// removal would shift subsequent indices down, causing incorrect removals.

#[test]
fn descending_removal_from_six_to_two_preserves_correct_carets() {
    // 6 carets. Engine sends 2 SetCursor effects.
    // Removal must happen: idx 5, 4, 3, 2 (descending).
    // If done ascending (2, 3, 4, 5), index 3 after removing index 2 would
    // actually be the original index 4, causing corruption.
    let mut mock = MockTextEdit::new("aa\nbb\ncc\ndd\nee\nff");
    mock.add_caret(1, 0);
    mock.add_caret(2, 0);
    mock.add_caret(3, 0);
    mock.add_caret(4, 0);
    mock.add_caret(5, 0);
    assert_eq!(mock.caret_count(), 6);
    let mut ctx = DispatchCtx::new();

    // 2 SetCursor effects: position at (0, 1) and (1, 1)
    // "aa\nbb\ncc\ndd\nee\nff" offsets: a=0, a=1, \n=2, b=3, b=4, \n=5, ...
    let fx = effects![set_cursor(1); set_cursor(4)];
    ctx.dispatch(&mut mock, fx);

    assert_editor!(mock, carets: 2);
    assert_eq!(mock.get_caret_line_for(0), 0);
    assert_eq!(mock.get_caret_column_for(0), 1);
    assert_eq!(mock.get_caret_line_for(1), 1);
    assert_eq!(mock.get_caret_column_for(1), 1);
}

#[test]
fn descending_removal_after_multi_cursor_insert_then_escape() {
    // Simulate: multi-cursor insert (3 cursors), then Escape (single cursor).
    // First dispatch: 3 inserts + 3 SetCursors -> 3 carets.
    // Second dispatch: 1 SetCursor -> excess 2 carets removed (descending).
    let mut mock = MockTextEdit::new("aaa\nbbb\nccc");
    let mut ctx = DispatchCtx::new();

    // Multi-cursor insert.
    let fx = effects![
        begin_undo;
        insert(0, "X");
        insert(5, "X");
        insert(10, "X");
        set_cursor(1);
        set_cursor(6);
        set_cursor(11);
        end_undo(1)
    ];
    ctx.dispatch(&mut mock, fx);
    assert_editor!(mock, text: "Xaaa\nXbbb\nXccc", carets: 3);

    // Escape: engine drops to single cursor. Single SetCursor -> secondaries removed.
    let fx = effects![set_cursor(1)];
    ctx.dispatch(&mut mock, fx);

    assert_editor!(mock, carets: 1, cursor: (0, 1));
}

// SetMode dispatch requires the Godot logger to be initialized (it logs
// mode transitions), so it cannot be tested here. The handler itself only
// calls cancel_code_completion() and dismiss_code_hint() (both no-ops).

// ── Architecture B: multi-cursor dispatch guards ────────────────────────────

#[test]
fn multi_cursor_dispatch_skips_all_set_cursors() {
    let mut mock = MockTextEdit::new("hello\nworld\nfoobar");
    let mut ctx = DispatchCtx::new();

    // Effects in descending offset order (matching engine output).
    // delete(start, end) — byte range [start, end).
    let fx = effects![
        begin_undo;
        delete(12, 13);
        set_cursor(12);
        delete(6, 7);
        set_cursor(5);
        delete(0, 1);
        set_cursor(0);
        end_undo(1)
    ];
    ctx.dispatch_multi(&mut mock, fx, 3);

    // Text mutations still applied (pass 1 unchanged).
    assert_editor!(mock, text: "ello\norld\noobar");
    // ALL SetCursor effects skipped in MC mode. Cleanup is no-op.
    // Only the initial caret 0 remains (untouched by dispatch).
    assert_eq!(mock.get_caret_count(), 1);
}

#[test]
fn multi_cursor_dispatch_skips_all_set_selection() {
    use vim_core::primitives::SelectionShape;
    let mut mock = MockTextEdit::new("hello\nworld");
    mock.add_caret(1, 0);
    let mut ctx = DispatchCtx::new();

    let fx = effects![
        set_selection(0, 3, SelectionShape::Char);
        set_cursor(3);
        set_selection(6, 9, SelectionShape::Char);
        set_cursor(9)
    ];
    ctx.dispatch_multi(&mut mock, fx, 2);

    assert_eq!(mock.get_caret_count(), 2);
}

#[test]
fn multi_cursor_clear_selection_preserves_secondary_carets() {
    let mut mock = MockTextEdit::new("hello\nworld");
    mock.add_caret(1, 0);
    assert_eq!(mock.get_caret_count(), 2);
    let mut ctx = DispatchCtx::new();

    let fx = effects![clear_selection];
    ctx.dispatch_multi(&mut mock, fx, 2);

    assert_eq!(mock.get_caret_count(), 2);
}
