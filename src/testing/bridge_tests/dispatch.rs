//! Full dispatch round-trip tests. These exercise the complete bridge pipeline
//! (effect list -> `crate::effects::dispatch` -> MockTextEdit), verifying that
//! undo grouping, text cache invalidation, selection/cursor interplay, and scroll
//! all work correctly together. Individual handler tests live in sibling modules;
//! these tests focus on multi-effect interactions that only manifest through the
//! full dispatch path.

use crate::bridge::port::TextEditorPort;
use crate::testing::MockTextEdit;
use super::macros::DispatchCtx;

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
        end_undo
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
        end_undo
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
        end_undo
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
        end_undo
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
        end_undo
    ];
    ctx.dispatch(&mut mock, fx);
    assert_editor!(mock, text: "hello");

    let fx = effects![
        begin_undo;
        insert(5, "!");
        set_cursor(5);
        end_undo
    ];
    ctx.dispatch(&mut mock, fx);
    assert_editor!(mock, text: "hello!");

    ctx.dispatch(&mut mock, effects![undo(1)]);
    assert_editor!(mock, text: "hello");

    ctx.dispatch(&mut mock, effects![undo(1)]);
    assert_editor!(mock, text: "hello world");

    ctx.dispatch(&mut mock, effects![redo(2)]);
    assert_editor!(mock, text: "hello!");
}

#[test]
fn dispatch_consecutive_commands_independent_undo() {
    let mut mock = MockTextEdit::new("aaa bbb ccc");
    let mut ctx = DispatchCtx::new();

    let fx = effects![begin_undo; delete(4, 8); set_cursor(4); end_undo];
    ctx.dispatch(&mut mock, fx);
    assert_editor!(mock, text: "aaa ccc");

    let fx = effects![begin_undo; delete(4, 7); set_cursor(3); end_undo];
    ctx.dispatch(&mut mock, fx);
    assert_editor!(mock, text: "aaa ");

    // Each command is an independent undo step.
    ctx.dispatch(&mut mock, effects![undo(1)]);
    assert_editor!(mock, text: "aaa ccc");

    ctx.dispatch(&mut mock, effects![undo(1)]);
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
        end_undo
    ];
    ctx.dispatch(&mut mock, fx);
    assert_editor!(mock, text: "the slow brown fox");

    ctx.dispatch(&mut mock, effects![undo(1)]);
    assert_editor!(mock, text: "the quick brown fox");
}

#[test]
fn dispatch_multiple_inserts_in_one_group() {
    let mut mock = MockTextEdit::new("ac");
    let mut ctx = DispatchCtx::new();

    let fx = effects![begin_undo; insert(1, "b"); end_undo];
    ctx.dispatch(&mut mock, fx);
    assert_editor!(mock, text: "abc");

    ctx.dispatch(&mut mock, effects![undo(1)]);
    assert_editor!(mock, text: "ac");
}

#[test]
fn undo_with_count_via_dispatch() {
    let mut mock = MockTextEdit::new("");
    let mut ctx = DispatchCtx::new();

    // Three separate insert commands
    for ch in &["a", "b", "c"] {
        let len = mock.get_text().len();
        let fx = effects![begin_undo; insert(len, *ch); end_undo];
        ctx.dispatch(&mut mock, fx);
    }
    assert_editor!(mock, text: "abc");

    // Undo 2 at once
    ctx.dispatch(&mut mock, effects![undo(2)]);
    assert_editor!(mock, text: "a");

    // Redo 1
    ctx.dispatch(&mut mock, effects![redo(1)]);
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
    let lines: String = (0..50).map(|i| format!("line {}", i)).collect::<Vec<_>>().join("\n");
    let mut mock = MockTextEdit::new(&lines);
    mock.set_visible_line_count(10);
    let mut ctx = DispatchCtx::new();

    // Move cursor to line 20, then CenterCursor
    let offset_20 = lines.match_indices('\n').nth(19).map(|(i, _)| i + 1).unwrap();
    let fx = effects![set_cursor(offset_20); center_cursor];
    ctx.dispatch(&mut mock, fx);
    assert_editor!(mock, cursor: (20, 0), scroll: 15);
}

#[test]
fn dispatch_cursor_to_bottom() {
    let lines: String = (0..50).map(|i| format!("line {}", i)).collect::<Vec<_>>().join("\n");
    let mut mock = MockTextEdit::new(&lines);
    mock.set_visible_line_count(10);
    let mut ctx = DispatchCtx::new();

    let offset_20 = lines.match_indices('\n').nth(19).map(|(i, _)| i + 1).unwrap();
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
    let lines: String = (0..20).map(|i| format!("line {}", i)).collect::<Vec<_>>().join("\n");
    let mut mock = MockTextEdit::new(&lines);
    mock.set_visible_line_count(10);
    let mut ctx = DispatchCtx::new();

    // ScrollTo offset at start of line 10
    let byte_offset = lines.match_indices('\n').nth(9).map(|(i, _)| i + 1).unwrap();
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
        end_undo;
        begin_undo;
        insert(3, "Y");
        end_undo
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

    let fx = effects![begin_undo; replace(10, 12, "{\n    \n}"); set_cursor(15); end_undo];
    ctx.dispatch(&mut mock, fx);
    assert_editor!(mock, text: "fn main() {\n    \n}");

    let fx = effects![begin_undo; insert(16, "println!(\"hello\");"); set_cursor(33); end_undo];
    ctx.dispatch(&mut mock, fx);
    assert_editor!(mock, text: "fn main() {\n    println!(\"hello\");\n}");

    ctx.dispatch(&mut mock, effects![undo(1)]);
    assert_editor!(mock, text: "fn main() {\n    \n}");

    ctx.dispatch(&mut mock, effects![undo(1)]);
    assert_editor!(mock, text: "fn main() {}");

    ctx.dispatch(&mut mock, effects![redo(2)]);
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
        end_undo
    ];
    ctx.dispatch(&mut mock, fx);
    assert_editor!(mock, text: "hello foo");

    ctx.dispatch(&mut mock, effects![undo(1)]);
    assert_editor!(mock, text: "hello world foo");
}

// SetMode dispatch requires the Godot logger to be initialized (it logs
// mode transitions), so it cannot be tested here. The handler itself only
// calls cancel_code_completion() and dismiss_code_hint() (both no-ops).
