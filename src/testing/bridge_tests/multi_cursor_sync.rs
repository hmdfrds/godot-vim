//! Integration tests for `sync_cursors_to_editor` — the Engine→Godot cursor
//! sync function from `src/multi_cursor/sync.rs`.
//!
//! Validates the delta-sync algorithm:
//! - Adding carets when the engine has more than Godot
//! - Removing excess carets (highest index first to avoid invalidation)
//! - Repositioning all carets to match engine positions
//! - Updating `buffer_state.last_caret_count`
//! - Graceful handling of empty cursor positions

use crate::bridge::port::TextEditorPort;
use crate::multi_cursor::sync::sync_cursors_to_editor;
use crate::state::buffer::BufferState;
use crate::testing::MockTextEdit;

// ── Adding carets ──────────────────────────────────────────────────────────

#[test]
fn sync_adds_carets_when_engine_has_more() {
    // Engine has 3 cursors, Godot has 1 (the default primary caret).
    // Should add 2 new carets.
    let mut mock = MockTextEdit::new("aaa\nbbb\nccc");
    let mut buffer_state = BufferState::default();

    let positions: Vec<(usize, usize, usize)> = vec![
        (0, 0, 0),  // primary caret stays at (0,0)
        (1, 1, 5),  // new caret at line 1, col 1
        (2, 2, 10), // new caret at line 2, col 2
    ];

    assert_eq!(mock.get_caret_count(), 1);
    sync_cursors_to_editor(&positions, &mut mock, &mut buffer_state);

    assert_eq!(mock.get_caret_count(), 3);
}

#[test]
fn sync_adds_carets_positions_are_correct() {
    // Verify that newly added carets end up at the correct positions.
    let mut mock = MockTextEdit::new("hello\nworld\nfoo");
    let mut buffer_state = BufferState::default();

    let positions: Vec<(usize, usize, usize)> = vec![
        (0, 2, 2),  // primary at (0, 2)
        (1, 3, 9),  // new caret at (1, 3)
        (2, 1, 13), // new caret at (2, 1)
    ];

    sync_cursors_to_editor(&positions, &mut mock, &mut buffer_state);

    assert_eq!(mock.get_caret_count(), 3);
    // All positions should be set by the final reposition loop.
    assert_eq!(mock.get_caret_line(), 0);
    assert_eq!(mock.get_caret_column(), 2);
}

// ── Removing carets ────────────────────────────────────────────────────────

#[test]
fn sync_removes_excess_carets_from_high_to_low() {
    // Engine has 1 cursor, Godot has 3 carets.
    // Should remove carets at indices 2, then 1 (highest first).
    let mut mock = MockTextEdit::new("aaa\nbbb\nccc");
    let mut buffer_state = BufferState::default();

    // Manually set up 3 carets in Godot.
    mock.add_caret(1, 0);
    mock.add_caret(2, 0);
    assert_eq!(mock.get_caret_count(), 3);

    let positions: Vec<(usize, usize, usize)> = vec![
        (0, 1, 1), // only the primary cursor remains
    ];

    sync_cursors_to_editor(&positions, &mut mock, &mut buffer_state);

    assert_eq!(mock.get_caret_count(), 1);
    // Primary caret should be repositioned to (0, 1).
    assert_eq!(mock.get_caret_line(), 0);
    assert_eq!(mock.get_caret_column(), 1);
}

#[test]
fn sync_removes_from_five_to_two() {
    // Engine has 2 cursors, Godot has 5.
    // Should remove indices 4, 3, 2 (highest first).
    let mut mock = MockTextEdit::new("aaaa\nbbbb\ncccc\ndddd\neeee");
    let mut buffer_state = BufferState::default();

    mock.add_caret(1, 0);
    mock.add_caret(2, 0);
    mock.add_caret(3, 0);
    mock.add_caret(4, 0);
    assert_eq!(mock.get_caret_count(), 5);

    let positions: Vec<(usize, usize, usize)> = vec![(0, 2, 2), (1, 3, 7)];

    sync_cursors_to_editor(&positions, &mut mock, &mut buffer_state);

    assert_eq!(mock.get_caret_count(), 2);
}

// ── Repositioning ──────────────────────────────────────────────────────────

#[test]
fn sync_repositions_all_carets_when_count_matches() {
    // Engine and Godot both have 3 carets, but positions differ.
    // Should reposition without adding or removing.
    let mut mock = MockTextEdit::new("hello\nworld\nfoo");
    let mut buffer_state = BufferState::default();

    // Set up 3 carets at arbitrary initial positions.
    mock.add_caret(0, 1);
    mock.add_caret(0, 2);
    assert_eq!(mock.get_caret_count(), 3);

    let positions: Vec<(usize, usize, usize)> = vec![
        (1, 4, 10), // move primary to (1, 4)
        (2, 2, 14), // move secondary 1 to (2, 2)
        (0, 3, 3),  // move secondary 2 to (0, 3)
    ];

    sync_cursors_to_editor(&positions, &mut mock, &mut buffer_state);

    // Count unchanged.
    assert_eq!(mock.get_caret_count(), 3);
    // Primary caret (index 0) should be at (1, 4).
    assert_eq!(mock.get_caret_line(), 1);
    assert_eq!(mock.get_caret_column(), 4);
}

#[test]
fn sync_repositions_after_adding() {
    // After adding carets, all positions (including pre-existing ones) get updated.
    let mut mock = MockTextEdit::new("aaaa\nbbbb\ncccc");
    let mut buffer_state = BufferState::default();

    // Start with 1 caret at (0, 0).
    assert_eq!(mock.get_caret_count(), 1);
    assert_eq!(mock.get_caret_line(), 0);
    assert_eq!(mock.get_caret_column(), 0);

    let positions: Vec<(usize, usize, usize)> = vec![
        (2, 3, 13), // primary moves to (2, 3)
        (1, 2, 7),  // new caret at (1, 2)
    ];

    sync_cursors_to_editor(&positions, &mut mock, &mut buffer_state);

    assert_eq!(mock.get_caret_count(), 2);
    // Primary (index 0) should be repositioned to (2, 3).
    assert_eq!(mock.get_caret_line(), 2);
    assert_eq!(mock.get_caret_column(), 3);
}

// ── Empty positions (edge case) ────────────────────────────────────────────

#[test]
fn sync_empty_positions_leaves_editor_unchanged() {
    // Empty cursor_positions should be a no-op (safety guard).
    let mut mock = MockTextEdit::new("hello\nworld");
    let mut buffer_state = BufferState::default();

    // Set caret to a known position.
    mock.set_caret_line(1);
    mock.set_caret_column(3);

    let positions: Vec<(usize, usize, usize)> = vec![];

    sync_cursors_to_editor(&positions, &mut mock, &mut buffer_state);

    // Nothing should change.
    assert_eq!(mock.get_caret_count(), 1);
    assert_eq!(mock.get_caret_line(), 1);
    assert_eq!(mock.get_caret_column(), 3);
}

#[test]
fn sync_empty_positions_does_not_update_last_caret_count() {
    // When positions are empty, buffer_state.last_caret_count must NOT be updated.
    let mut mock = MockTextEdit::new("hello");
    let mut buffer_state = BufferState::default();

    // Simulate a previous sync that set last_caret_count to 3.
    buffer_state.set_last_caret_count(3);

    let positions: Vec<(usize, usize, usize)> = vec![];
    sync_cursors_to_editor(&positions, &mut mock, &mut buffer_state);

    // Should remain at 3 (the early return skips the update).
    assert_eq!(buffer_state.last_caret_count(), 3);
}

// ── buffer_state.last_caret_count tracking ─────────────────────────────────

#[test]
fn sync_updates_last_caret_count_on_add() {
    let mut mock = MockTextEdit::new("aaa\nbbb\nccc");
    let mut buffer_state = BufferState::default();
    assert_eq!(buffer_state.last_caret_count(), 1);

    let positions: Vec<(usize, usize, usize)> = vec![(0, 0, 0), (1, 0, 4), (2, 0, 8)];

    sync_cursors_to_editor(&positions, &mut mock, &mut buffer_state);

    assert_eq!(buffer_state.last_caret_count(), 3);
}

#[test]
fn sync_updates_last_caret_count_on_remove() {
    let mut mock = MockTextEdit::new("aaa\nbbb\nccc");
    let mut buffer_state = BufferState::default();
    buffer_state.set_last_caret_count(3);

    // Godot has 3 carets.
    mock.add_caret(1, 0);
    mock.add_caret(2, 0);
    assert_eq!(mock.get_caret_count(), 3);

    // Engine says only 1 cursor.
    let positions: Vec<(usize, usize, usize)> = vec![(0, 0, 0)];

    sync_cursors_to_editor(&positions, &mut mock, &mut buffer_state);

    assert_eq!(buffer_state.last_caret_count(), 1);
}

#[test]
fn sync_updates_last_caret_count_on_reposition() {
    // Even when count doesn't change, last_caret_count should still be set
    // (idempotent update).
    let mut mock = MockTextEdit::new("aaa\nbbb");
    let mut buffer_state = BufferState::default();

    mock.add_caret(1, 0);
    assert_eq!(mock.get_caret_count(), 2);
    buffer_state.set_last_caret_count(2);

    let positions: Vec<(usize, usize, usize)> = vec![(0, 1, 1), (1, 2, 6)];

    sync_cursors_to_editor(&positions, &mut mock, &mut buffer_state);

    assert_eq!(buffer_state.last_caret_count(), 2);
}

// ── Column clamping ────────────────────────────────────────────────────────

#[test]
fn sync_clamps_positions_to_line_length() {
    // If engine reports a column beyond the line length, MockTextEdit (like Godot)
    // should clamp it.
    let mut mock = MockTextEdit::new("hi\nab");
    let mut buffer_state = BufferState::default();

    // Engine says cursor is at (0, 99) which is past "hi" (len=2).
    let positions: Vec<(usize, usize, usize)> = vec![(0, 99, 99)];

    sync_cursors_to_editor(&positions, &mut mock, &mut buffer_state);

    // Should clamp to line length (2 for "hi").
    assert_eq!(mock.get_caret_line(), 0);
    assert_eq!(mock.get_caret_column(), 2);
}
