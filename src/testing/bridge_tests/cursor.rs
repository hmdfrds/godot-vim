//! Tests for cursor positioning, selection (char/line/block modes), and
//! multi-caret management. These verify that the bridge correctly translates
//! vim-core's byte-offset selections into Godot's line/col selection API.

use super::macros::apply_set_selection;
use crate::bridge::port::TextEditorPort;
use crate::testing::MockTextEdit;
use crate::types::CharLineCol;
use vim_core::primitives::SelectionShape;

// ── Cursor clamping (validates MockTextEdit matches Godot's clamping) ───

#[test]
fn set_caret_line_clamps() {
    let mut mock = MockTextEdit::new("a\nb\nc");
    mock.set_caret_line(100);
    assert_eq!(mock.get_caret_line(), 2);
    mock.set_caret_line(-5);
    assert_eq!(mock.get_caret_line(), 0);
}

#[test]
fn set_caret_column_clamps() {
    let mut mock = MockTextEdit::new("hello");
    mock.set_caret_column(100);
    assert_eq!(mock.get_caret_column(), 5);
    mock.set_caret_column(-3);
    assert_eq!(mock.get_caret_column(), 0);
}

#[test]
fn set_caret_line_clamps_column() {
    let mut mock = MockTextEdit::new("hello\nhi");
    mock.set_caret_column(5);
    mock.set_caret_line(1);
    assert_editor!(mock, cursor: (1, 2));
}

// ── Selection via trait API (origin-based model) ────────────────────────

#[test]
fn select_forward() {
    let mut mock = MockTextEdit::new("hello world");
    mock.select(CharLineCol::new(0, 0), CharLineCol::new(0, 5));
    assert_editor!(mock, has_selection, selection: (0, 0) => (0, 5));
}

#[test]
fn select_backward() {
    let mut mock = MockTextEdit::new("hello world");
    mock.select(CharLineCol::new(0, 5), CharLineCol::new(0, 0));
    assert_editor!(mock, has_selection, selection: (0, 0) => (0, 5));
}

#[test]
fn select_degenerate_is_inactive() {
    let mut mock = MockTextEdit::new("hello");
    mock.select(CharLineCol::new(0, 3), CharLineCol::new(0, 3));
    assert_editor!(mock, no_selection);
}

#[test]
fn deselect_clears_all() {
    let mut mock = MockTextEdit::new("hello");
    mock.select(CharLineCol::new(0, 0), CharLineCol::new(0, 5));
    assert_editor!(mock, has_selection);
    mock.deselect();
    assert_editor!(mock, no_selection);
}

// ── Multi-caret ─────────────────────────────────────────────────────────

#[test]
fn add_and_remove_secondary_carets() {
    let mut mock = MockTextEdit::new("aaa\nbbb\nccc");
    let idx = mock.add_caret(1, 1);
    assert_eq!(idx, 1);
    assert_editor!(mock, carets: 2);
    mock.remove_secondary_carets();
    assert_editor!(mock, carets: 1);
}

#[test]
fn select_for_caret_out_of_bounds() {
    let mut mock = MockTextEdit::new("hello");
    mock.select_for_caret(CharLineCol::new(0, 0), CharLineCol::new(0, 3), 5);
    assert_editor!(mock, no_selection);
}

// ── Effect handler: set_cursor (byte offset -> line/col) ────────────────

#[test]
fn effect_handle_set_cursor() {
    let mut mock = MockTextEdit::new("hello\nworld");
    super::macros::apply_set_cursor(&mut mock, 8);
    assert_editor!(mock, cursor: (1, 2));
}

// ── Effect handler: set_selection — Char mode ───────────────────────────
// Vim's char selection is inclusive at both ends; Godot's is exclusive at
// the end. The bridge adds +1 to the head column to compensate.

#[test]
fn effect_handle_set_selection_char_forward() {
    let mut mock = MockTextEdit::new("hello world");
    apply_set_selection(&mut mock, 0, 4, SelectionShape::Char);
    assert_editor!(mock, has_selection, selection_cols: (0, 5));
}

#[test]
fn effect_handle_set_selection_char_backward() {
    let mut mock = MockTextEdit::new("hello world");
    apply_set_selection(&mut mock, 4, 0, SelectionShape::Char);
    assert_editor!(mock, has_selection, selection_cols: (0, 5));
}

#[test]
fn char_selection_across_lines() {
    let mut mock = MockTextEdit::new("hello\nworld");
    apply_set_selection(&mut mock, 2, 8, SelectionShape::Char);
    assert_editor!(mock,
        has_selection,
        selection: (0, 2) => (1, 3), // +1 exclusive end adjustment
    );
}

#[test]
fn char_selection_single_char() {
    let mut mock = MockTextEdit::new("hello");
    // Same anchor and head — Vim still selects the character under the cursor.
    apply_set_selection(&mut mock, 2, 2, SelectionShape::Char);
    assert_editor!(mock, has_selection, selection_cols: (2, 3));
}

// ── Effect handler: clear_selection ─────────────────────────────────────

#[test]
fn effect_handle_clear_selection() {
    let mut mock = MockTextEdit::new("hello");
    mock.select(CharLineCol::new(0, 0), CharLineCol::new(0, 5));
    crate::effects::cursor::handle_clear_selection(&mut mock);
    assert_editor!(mock, no_selection);
}

// ── Effect handler: set_selection — Line mode ───────────────────────────

#[test]
fn effect_handle_set_selection_line() {
    let mut mock = MockTextEdit::new("hello\nworld\nfoo");
    apply_set_selection(&mut mock, 0, 8, SelectionShape::Line);
    assert_editor!(mock, has_selection, selection: (0, 0) => (1, 5));
}

#[test]
fn line_selection_backward() {
    let mut mock = MockTextEdit::new("hello\nworld\nfoo");
    // Backward direction: line selection always expands to full lines regardless
    // of anchor/head direction, so result matches the forward case.
    apply_set_selection(&mut mock, 8, 0, SelectionShape::Line);
    assert_editor!(mock, has_selection, selection: (0, 0) => (1, 5));
}

#[test]
fn line_selection_single_line() {
    let mut mock = MockTextEdit::new("hello\nworld");
    apply_set_selection(&mut mock, 1, 3, SelectionShape::Line);
    assert_editor!(mock,
        has_selection,
        selection: (0, 0) => (0, 5),
    );
}

// ── Effect handler: set_selection — Block mode ──────────────────────────
// Block selection uses multi-caret: one caret per line in the block range,
// each with a selection spanning the block columns. The bridge must create
// secondary carets and clamp columns for short lines.

#[test]
fn effect_handle_set_selection_block() {
    let mut mock = MockTextEdit::new("abcd\nefgh\nijkl");
    apply_set_selection(&mut mock, 1, 12, SelectionShape::Block);
    assert_editor!(mock, carets: 3);
}

#[test]
fn block_selection_backward_creates_correct_carets() {
    let mut mock = MockTextEdit::new("abcd\nefgh\nijkl");
    apply_set_selection(&mut mock, 12, 1, SelectionShape::Block);
    assert_editor!(mock, carets: 3);
}

#[test]
fn block_selection_single_line_one_caret() {
    let mut mock = MockTextEdit::new("hello world");
    apply_set_selection(&mut mock, 0, 4, SelectionShape::Block);
    assert_editor!(mock, carets: 1, has_selection);
}

#[test]
fn block_selection_columns_are_correct() {
    let mut mock = MockTextEdit::new("abcd\nefgh\nijkl");
    // anchor=(0,1), head=(2,2) → block cols 1..=2 → render cols 1..3 (exclusive end)
    apply_set_selection(&mut mock, 1, 12, SelectionShape::Block);
    assert_editor!(mock, selection_cols: (1, 3));
}

#[test]
fn block_selection_backward_columns() {
    let mut mock = MockTextEdit::new("abcd\nefgh\nijkl");
    // Same block columns regardless of anchor/head direction.
    apply_set_selection(&mut mock, 12, 1, SelectionShape::Block);
    assert_editor!(mock, selection_cols: (1, 3));
}

#[test]
fn clear_selection_removes_secondary_carets() {
    let mut mock = MockTextEdit::new("abcd\nefgh\nijkl");
    apply_set_selection(&mut mock, 1, 12, SelectionShape::Block);
    assert_editor!(mock, carets: 3);

    crate::effects::cursor::handle_clear_selection(&mut mock);
    assert_editor!(mock, carets: 1, no_selection);
}

#[test]
fn block_selection_replaces_previous_carets() {
    let mut mock = MockTextEdit::new("abcd\nefgh\nijkl");
    apply_set_selection(&mut mock, 1, 12, SelectionShape::Block);
    assert_editor!(mock, carets: 3);

    // New block on 2 lines — must replace previous 3-caret block, not accumulate.
    apply_set_selection(&mut mock, 0, 6, SelectionShape::Block);
    assert_editor!(mock, carets: 2);
}

#[test]
fn block_selection_short_line_clamps() {
    // Line 1 ("ab") is shorter than the block columns — must clamp, not panic.
    let mut mock = MockTextEdit::new("0123456789\nab\n0123456789");
    apply_set_selection(&mut mock, 5, 21, SelectionShape::Block);
    assert_editor!(mock, carets: 3);
}

// ── Edge case: char selection crossing line boundary ─────────────────

#[test]
fn char_selection_forward_anchor_at_eol() {
    let mut mock = MockTextEdit::new("hello\nworld");
    // Selection spans the \n: from 'o' (byte 4) to 'w' (byte 6).
    apply_set_selection(&mut mock, 4, 6, SelectionShape::Char);
    assert_editor!(mock, has_selection, selection: (0, 4) => (1, 1));
}

#[test]
fn char_selection_backward_head_at_eol() {
    let mut mock = MockTextEdit::new("hello\nworld");
    // Same range in reverse direction — result should be identical.
    apply_set_selection(&mut mock, 6, 4, SelectionShape::Char);
    assert_editor!(mock, has_selection, selection: (0, 4) => (1, 1));
}

// ── Edge case: cursor at EOF ─────────────────────────────────────────

#[test]
fn set_cursor_at_eof() {
    let mut mock = MockTextEdit::new("hello\nworld");
    super::macros::apply_set_cursor(&mut mock, 11);
    assert_editor!(mock, cursor: (1, 5));
}

// ── Edge case: zero-width block selection (same column) ──────────────

#[test]
fn block_selection_zero_width() {
    // Same column on anchor and head — block is 1 char wide (min_col == max_col).
    let mut mock = MockTextEdit::new("abcd\nefgh\nijkl");
    apply_set_selection(&mut mock, 1, 11, SelectionShape::Block);
    assert_editor!(mock, carets: 3, has_selection);
}

// ── Edge case: block selection where ALL lines are shorter than block ─

#[test]
fn block_selection_all_lines_short() {
    // All lines are only 2 chars — block columns extend past every line's end.
    // Must clamp without panicking.
    let mut mock = MockTextEdit::new("ab\ncd\nef");
    apply_set_selection(&mut mock, 1, 8, SelectionShape::Block);
    assert_editor!(mock, carets: 3);
}
