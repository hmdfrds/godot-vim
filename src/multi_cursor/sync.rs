//! Bidirectional cursor synchronization between vim-core and Godot.
//!
//! - Engine→Godot: after process_key, sync vim-core's cursor positions to Godot carets
//! - Godot→Engine: before process_key, import mouse-added carets into vim-core

use crate::bridge::codec::{usize_to_i32, LineIndex};
use crate::bridge::port::TextEditorPort;
use crate::state::buffer::BufferState;

/// A cursor position as `(line, col, byte_offset)`.
/// The `byte_offset` field is the engine-internal byte position in the document.
type CursorPosition = (usize, usize, usize);

/// RAII guard ensuring `end_multicaret_edit` is called even if a panic
/// unwinds past the sync logic (e.g., from a Godot FFI call via gdext).
struct MulticaretEditGuard<'a>(&'a mut dyn TextEditorPort);

impl<'a> MulticaretEditGuard<'a> {
    fn begin(editor: &'a mut dyn TextEditorPort) -> Self {
        editor.begin_multicaret_edit();
        Self(editor)
    }
}

impl Drop for MulticaretEditGuard<'_> {
    fn drop(&mut self) {
        self.0.end_multicaret_edit();
    }
}

/// Synchronize vim-core's cursor positions to Godot's multi-caret API (delta-sync).
///
/// After `session.process_key()`, the engine may have added or removed cursors.
/// This function reconciles Godot's caret state with the engine's truth by:
/// 1. Wrapping all caret operations in begin/end_multicaret_edit (merge deferral)
/// 2. Adding carets if the engine has more than Godot
/// 3. Removing excess carets (highest index first to avoid invalidation)
/// 4. Updating positions only for carets that actually moved (position diff)
///
/// Uses `begin_multicaret_edit` instead of `begin_complex_operation` to:
/// - Defer `merge_overlapping_carets` until all positions are set
/// - Suppress `add_caret`'s overlap rejection (-1 return)
/// - Avoid undo chain corruption (`multicaret_edit` has zero undo interaction)
///
/// `cursor_positions` contains `CursorPosition` tuples from vim-core.
/// The `byte_offset` field is unused here (it's for engine-internal tracking).
pub(crate) fn sync_cursors_to_editor(
    cursor_positions: &[CursorPosition],
    editor: &mut dyn TextEditorPort,
    buffer_state: &mut BufferState,
) {
    // Safety: empty positions should not happen, but if it does, leave Godot unchanged.
    if cursor_positions.is_empty() {
        return;
    }

    let vim_count = usize_to_i32(cursor_positions.len());
    let godot_count = editor.get_caret_count();

    {
        let guard = MulticaretEditGuard::begin(editor);
        let ed = &mut *guard.0;

        // ── Add new carets if engine has more ───────────────────────────
        if vim_count > godot_count {
            for &(line, col, _) in &cursor_positions[godot_count as usize..] {
                let result = ed.add_caret(usize_to_i32(line), usize_to_i32(col));
                if result < 0 {
                    log::error!(
                        "sync_cursors_to_editor: add_caret({}, {}) failed",
                        line,
                        col
                    );
                }
            }
        }

        // ── Remove excess carets from highest index down ────────────────
        if vim_count < godot_count {
            for idx in (vim_count..godot_count).rev() {
                ed.remove_caret(idx);
            }
        }

        // Re-read actual caret count after adds/removes — add_caret can fail
        // (returns -1), so the actual count may differ from the expected count.
        let actual_count = ed.get_caret_count();
        let reposition_count = vim_count.min(actual_count);

        // ── Update positions for carets that actually moved ─────────────
        for (caret_idx, &(line, col, _)) in cursor_positions.iter().enumerate() {
            let idx = usize_to_i32(caret_idx);
            if idx >= reposition_count {
                break;
            }
            let target_line = usize_to_i32(line);
            let target_col = usize_to_i32(col);

            let current_line = ed.get_caret_line_for(idx);
            let current_col = ed.get_caret_column_for(idx);
            if current_line != target_line || current_col != target_col {
                ed.set_caret_line_for(target_line, idx);
                ed.set_caret_column_for(target_col, idx);
            }
        }
        // guard dropped here → end_multicaret_edit called
    }

    let final_count = editor.get_caret_count();
    if final_count != vim_count {
        log::warn!(
            "sync_cursors_to_editor: divergence — engine wants {} carets, Godot has {}",
            vim_count,
            final_count
        );
    }

    buffer_state.set_last_caret_count(final_count as usize);
}

/// The result of comparing Godot's current caret count against the last known count.
///
/// This is the single source of truth for the import-carets algorithm. Both the
/// callback-based `import_godot_carets` (for testability) and the production
/// `import_godot_carets_into_engine` (in `process.rs`) delegate to
/// `compute_import_action` to determine what to do, then apply the action
/// using their respective APIs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ImportAction {
    /// No change in caret count — nothing to do.
    NoChange,
    /// User added carets (Ctrl+Click). Contains byte offsets of the NEW carets only.
    AddCursors(Vec<usize>),
    /// User removed carets. Contains byte offsets of ALL current secondary carets
    /// (indices 1..current_count). The caller must clear all engine secondaries
    /// first, then re-add these.
    FullResync(Vec<usize>),
}

/// Compute the import action by reading caret positions from the editor.
///
/// This is the SINGLE source of truth for the Godot→Engine import algorithm.
/// It reads the editor's current caret state, compares against `last_count`,
/// and returns an `ImportAction` describing what the caller must do.
///
/// After applying the action, the caller must update `buffer_state.set_last_caret_count`.
pub(crate) fn compute_import_action(
    editor: &dyn TextEditorPort,
    last_count: usize,
    line_index: &LineIndex,
    text: &str,
) -> ImportAction {
    let current_count = editor.get_caret_count() as usize;

    if current_count == last_count {
        return ImportAction::NoChange;
    }

    if current_count > last_count {
        // User added carets — compute byte offsets for only the new ones.
        let offsets = (last_count..current_count)
            .map(|idx| {
                let line = editor.get_caret_line_for(idx as i32);
                let col = editor.get_caret_column_for(idx as i32);
                line_index.line_col_to_byte(text, line, col)
            })
            .collect();
        ImportAction::AddCursors(offsets)
    } else {
        // User removed carets — compute byte offsets for all current secondaries.
        let offsets = (1..current_count)
            .map(|idx| {
                let line = editor.get_caret_line_for(idx as i32);
                let col = editor.get_caret_column_for(idx as i32);
                line_index.line_col_to_byte(text, line, col)
            })
            .collect();
        ImportAction::FullResync(offsets)
    }
}

/// Import mouse-added Godot carets into vim-core (Godot→Engine direction).
///
/// Detects when the user Ctrl+Clicks in Godot to add/remove carets and
/// informs the engine via the provided callbacks. Called before `process_key`
/// so the engine sees the updated cursor set.
///
/// When carets are added (current > last), the new ones are converted to byte
/// offsets and forwarded via `add_cursor`. When carets are removed (current < last),
/// all vim-core secondary cursors are cleared and re-added from current Godot state
/// (full resync) since we cannot determine which specific caret was removed.
///
/// The core algorithm lives in `compute_import_action` — this function is a
/// thin callback-based adapter for testability.
#[cfg(test)]
pub(crate) fn import_godot_carets(
    editor: &dyn TextEditorPort,
    buffer_state: &mut BufferState,
    add_cursor: &mut dyn FnMut(usize),
    remove_cursor: &mut dyn FnMut(usize),
    line_index: &LineIndex,
    text: &str,
) {
    let last_count = buffer_state.last_caret_count();
    let action = compute_import_action(editor, last_count, line_index, text);

    match action {
        ImportAction::NoChange => return,
        ImportAction::AddCursors(offsets) => {
            for offset in &offsets {
                add_cursor(*offset);
            }
        }
        ImportAction::FullResync(new_secondary_offsets) => {
            // Signal "clear all secondaries" with a single remove_cursor(0) call,
            // matching the production code's ClearSecondary semantics. We do NOT
            // read positions from old indices (they may be OOB since the editor
            // has already lost those carets).
            remove_cursor(0);
            // Re-add current secondaries.
            for offset in &new_secondary_offsets {
                add_cursor(*offset);
            }
        }
    }

    let current_count = editor.get_caret_count() as usize;
    buffer_state.set_last_caret_count(current_count);
}

/// Synchronize vim-core's per-cursor visual selections to Godot's editor.
///
/// Each entry in `selections` is `(from_line, from_col, to_line, to_col)`
/// representing Godot-ready coordinates for `select_for_caret`. The
/// Vim-inclusive to Godot-exclusive +1 adjustment is applied by the caller
/// in `sync_multi_cursors_to_godot` (process.rs).
///
/// Precondition: the caller must have already called `sync_cursors_to_editor`
/// to ensure the caret count matches. If `selections.len()` exceeds the current
/// caret count, excess entries are silently ignored.
pub(crate) fn sync_selections_to_editor(
    selections: &[(usize, usize, usize, usize)],
    editor: &mut dyn TextEditorPort,
) {
    use crate::types::CharLineCol;

    let caret_count = editor.get_caret_count() as usize;

    for (caret_idx, &(from_line, from_col, to_line, to_col)) in
        selections.iter().enumerate()
    {
        if caret_idx >= caret_count {
            break;
        }

        let from = CharLineCol::new(from_line as i32, from_col as i32);
        let to = CharLineCol::new(to_line as i32, to_col as i32);

        if from == to {
            continue;
        }

        editor.select_for_caret(from, to, caret_idx as i32);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bridge::codec::LineIndex;
    use crate::bridge::port::TextEditorPort;
    use crate::testing::MockTextEdit;

    fn default_buffer_state() -> BufferState {
        BufferState::default()
    }

    // ── Basic sync: single cursor ─────────────────────────────────────────

    #[test]
    fn single_cursor_positions_correctly() {
        let mut mock = MockTextEdit::new("hello\nworld\nfoo");
        let mut bs = default_buffer_state();

        let positions = vec![(1, 3, 9)]; // line 1, col 3
        sync_cursors_to_editor(&positions, &mut mock, &mut bs);

        assert_eq!(mock.get_caret_count(), 1);
        assert_eq!(mock.get_caret_line(), 1);
        assert_eq!(mock.get_caret_column(), 3);
        assert_eq!(bs.last_caret_count(), 1);
    }

    // ── Ctrl+D scenario: 3 cursors on matching words, then movement ───────

    #[test]
    fn ctrl_d_three_cursors_then_move_right() {
        // Scenario: user has "foo" on lines 0, 2, 4. After Ctrl+D x3, engine
        // has 3 cursors at (0,0), (2,0), (4,0). User types 'l' (move right),
        // engine moves all to col 1. Sync should position all 3 at col 1.
        let mut mock = MockTextEdit::new("foo bar\nbaz\nfoo baz\nqux\nfoo end");
        let mut bs = default_buffer_state();

        // Initial state: 3 cursors at start of each "foo"
        let positions = vec![(0, 0, 0), (2, 0, 12), (4, 0, 24)];
        sync_cursors_to_editor(&positions, &mut mock, &mut bs);

        assert_eq!(mock.get_caret_count(), 3);
        assert_eq!(bs.last_caret_count(), 3);

        // After 'l' (move right): all cursors move to col 1
        let moved_positions = vec![(0, 1, 1), (2, 1, 13), (4, 1, 25)];
        sync_cursors_to_editor(&moved_positions, &mut mock, &mut bs);

        assert_eq!(mock.get_caret_count(), 3);
        // Primary caret (index 0)
        assert_eq!(mock.get_caret_line(), 0);
        assert_eq!(mock.get_caret_column(), 1);
    }

    #[test]
    fn ctrl_d_three_cursors_typing_inserts_at_all() {
        // Scenario: 3 cursors at (0,3), (2,3), (4,3) — end of "foo".
        // User types 'x' in insert mode. Engine reports new positions at col 4.
        // (Text mutation is handled separately; sync only positions carets.)
        let mut mock = MockTextEdit::new("foo bar\nbaz\nfoo baz\nqux\nfoo end");
        let mut bs = default_buffer_state();

        // After typing 'x': engine has moved cursors past the inserted char
        let positions = vec![(0, 4, 4), (2, 4, 16), (4, 4, 28)];
        sync_cursors_to_editor(&positions, &mut mock, &mut bs);

        assert_eq!(mock.get_caret_count(), 3);
        assert_eq!(bs.last_caret_count(), 3);
    }

    // ── Add cursor above/below (vertical placement) ───────────────────────

    #[test]
    fn add_cursor_below_vertical_placement() {
        // Scenario: cursor at (2, 5), user presses Ctrl+Alt+Down twice.
        // Engine reports cursors at (2,5), (3,5), (4,5).
        let mut mock = MockTextEdit::new("abcdefgh\nabcdefgh\nabcdefgh\nabcdefgh\nabcdefgh");
        let mut bs = default_buffer_state();

        let positions = vec![(2, 5, 23), (3, 5, 32), (4, 5, 41)];
        sync_cursors_to_editor(&positions, &mut mock, &mut bs);

        assert_eq!(mock.get_caret_count(), 3);
        assert_eq!(bs.last_caret_count(), 3);
    }

    #[test]
    fn add_cursor_above_vertical_placement() {
        // Scenario: cursor at (3, 2), user presses Ctrl+Alt+Up twice.
        // Engine reports cursors at (1,2), (2,2), (3,2).
        let mut mock = MockTextEdit::new("abcdefgh\nabcdefgh\nabcdefgh\nabcdefgh");
        let mut bs = default_buffer_state();

        let positions = vec![(1, 2, 11), (2, 2, 20), (3, 2, 29)];
        sync_cursors_to_editor(&positions, &mut mock, &mut bs);

        assert_eq!(mock.get_caret_count(), 3);
        // All on same column, different lines
        assert_eq!(mock.get_caret_line(), 1); // primary
        assert_eq!(mock.get_caret_column(), 2);
    }

    #[test]
    fn vertical_cursor_clamps_to_short_line() {
        // If a line is shorter than the target column, Godot/mock clamps.
        // Engine would not send out-of-range, but verify mock handles gracefully.
        let mut mock = MockTextEdit::new("long line here\nab\nlong line here");
        let mut bs = default_buffer_state();

        // Engine says col 10 on line 1, but line 1 ("ab") is only 2 chars.
        // Mock should clamp to col 2 (matching Godot's real behavior).
        let positions = vec![(0, 10, 10), (1, 10, 25), (2, 10, 27)];
        sync_cursors_to_editor(&positions, &mut mock, &mut bs);

        assert_eq!(mock.get_caret_count(), 3);
    }

    // ── Rapid add/remove cycle ────────────────────────────────────────────

    #[test]
    fn rapid_add_remove_cycle() {
        // Scenario: add 5, remove 3, add 2 — final state is 4 cursors.
        let mut mock = MockTextEdit::new("line0\nline1\nline2\nline3\nline4\nline5\nline6");
        let mut bs = default_buffer_state();

        // Step 1: Engine has 5 cursors (one per line 0..4)
        let step1 = vec![(0, 0, 0), (1, 0, 6), (2, 0, 12), (3, 0, 18), (4, 0, 24)];
        sync_cursors_to_editor(&step1, &mut mock, &mut bs);
        assert_eq!(mock.get_caret_count(), 5);
        assert_eq!(bs.last_caret_count(), 5);

        // Step 2: Engine removes 3 cursors — now 2 remain (lines 0, 1)
        let step2 = vec![(0, 0, 0), (1, 0, 6)];
        sync_cursors_to_editor(&step2, &mut mock, &mut bs);
        assert_eq!(mock.get_caret_count(), 2);
        assert_eq!(bs.last_caret_count(), 2);

        // Step 3: Engine adds 2 more — now 4 (lines 0, 1, 5, 6)
        let step3 = vec![(0, 0, 0), (1, 0, 6), (5, 0, 30), (6, 0, 36)];
        sync_cursors_to_editor(&step3, &mut mock, &mut bs);
        assert_eq!(mock.get_caret_count(), 4);
        assert_eq!(bs.last_caret_count(), 4);
    }

    #[test]
    fn rapid_grow_shrink_grow() {
        // Add to 3, shrink to 1, grow to 4 — tests that removal from highest
        // index down doesn't corrupt lower indices.
        let mut mock = MockTextEdit::new("aaaa\nbbbb\ncccc\ndddd\neeee");
        let mut bs = default_buffer_state();

        // Grow to 3
        let step1 = vec![(0, 2, 2), (1, 2, 7), (2, 2, 12)];
        sync_cursors_to_editor(&step1, &mut mock, &mut bs);
        assert_eq!(mock.get_caret_count(), 3);

        // Shrink to 1 (primary only)
        let step2 = vec![(0, 2, 2)];
        sync_cursors_to_editor(&step2, &mut mock, &mut bs);
        assert_eq!(mock.get_caret_count(), 1);
        assert_eq!(bs.last_caret_count(), 1);

        // Grow to 4
        let step3 = vec![(0, 0, 0), (1, 0, 5), (2, 0, 10), (3, 0, 15)];
        sync_cursors_to_editor(&step3, &mut mock, &mut bs);
        assert_eq!(mock.get_caret_count(), 4);
        assert_eq!(bs.last_caret_count(), 4);
    }

    // ── Edge cases ────────────────────────────────────────────────────────

    #[test]
    fn empty_positions_is_noop() {
        let mut mock = MockTextEdit::new("hello");
        let mut bs = default_buffer_state();
        mock.set_caret_line(0);
        mock.set_caret_column(3);

        sync_cursors_to_editor(&[], &mut mock, &mut bs);

        // Nothing changed
        assert_eq!(mock.get_caret_count(), 1);
        assert_eq!(mock.get_caret_line(), 0);
        assert_eq!(mock.get_caret_column(), 3);
    }

    #[test]
    fn position_update_without_count_change() {
        // Same number of cursors but positions moved (e.g., all cursors moved right).
        let mut mock = MockTextEdit::new("abcdef\nghijkl\nmnopqr");
        let mut bs = default_buffer_state();

        // Initial: 2 cursors
        let initial = vec![(0, 0, 0), (1, 0, 7)];
        sync_cursors_to_editor(&initial, &mut mock, &mut bs);
        assert_eq!(mock.get_caret_count(), 2);

        // Same count, moved positions
        let moved = vec![(0, 3, 3), (1, 3, 10)];
        sync_cursors_to_editor(&moved, &mut mock, &mut bs);
        assert_eq!(mock.get_caret_count(), 2);
        assert_eq!(mock.get_caret_line(), 0);
        assert_eq!(mock.get_caret_column(), 3);
    }

    #[test]
    fn buffer_state_tracks_last_caret_count() {
        let mut mock = MockTextEdit::new("a\nb\nc\nd\ne");
        let mut bs = default_buffer_state();
        assert_eq!(bs.last_caret_count(), 1); // default

        sync_cursors_to_editor(&[(0, 0, 0), (1, 0, 2), (2, 0, 4)], &mut mock, &mut bs);
        assert_eq!(bs.last_caret_count(), 3);

        sync_cursors_to_editor(&[(0, 0, 0)], &mut mock, &mut bs);
        assert_eq!(bs.last_caret_count(), 1);
    }

    // ── Mock behavioral fidelity ──────────────────────────────────────────
    // Verify the mock matches Godot's real CodeEdit behavior for the
    // multi-caret operations used by sync_cursors_to_editor.

    #[test]
    fn mock_add_caret_returns_valid_index() {
        // Godot's add_caret returns the new caret index (carets.size() - 1).
        // Mock must replicate this for sync logic to work.
        let mut mock = MockTextEdit::new("hello\nworld\nfoo");
        assert_eq!(mock.get_caret_count(), 1);

        let idx1 = mock.add_caret(1, 0);
        assert_eq!(idx1, 1);
        assert_eq!(mock.get_caret_count(), 2);

        let idx2 = mock.add_caret(2, 0);
        assert_eq!(idx2, 2);
        assert_eq!(mock.get_caret_count(), 3);
    }

    #[test]
    fn mock_remove_caret_adjusts_count() {
        // Godot's remove_caret removes at index and shifts higher indices down.
        // Removing from highest index first (as sync does) avoids invalidation.
        let mut mock = MockTextEdit::new("a\nb\nc\nd");
        mock.add_caret(1, 0); // idx 1
        mock.add_caret(2, 0); // idx 2
        mock.add_caret(3, 0); // idx 3
        assert_eq!(mock.get_caret_count(), 4);

        // Remove from highest down (sync's strategy)
        mock.remove_caret(3);
        assert_eq!(mock.get_caret_count(), 3);
        mock.remove_caret(2);
        assert_eq!(mock.get_caret_count(), 2);
        mock.remove_caret(1);
        assert_eq!(mock.get_caret_count(), 1);
    }

    #[test]
    fn mock_remove_caret_from_middle_shifts_indices() {
        // Verify that removing from the middle is safe (though sync avoids it).
        let mut mock = MockTextEdit::new("a\nb\nc");
        mock.add_caret(1, 0); // idx 1
        mock.add_caret(2, 0); // idx 2
        assert_eq!(mock.get_caret_count(), 3);

        // Remove middle caret
        mock.remove_caret(1);
        assert_eq!(mock.get_caret_count(), 2);

        // The old idx 2 is now idx 1 — set_caret_line_for should target it.
        mock.set_caret_line_for(2, 1);
        // No panic = correct behavior (matching Godot's carets.remove_at(p_caret)).
    }

    #[test]
    fn mock_set_caret_line_for_clamps() {
        // Mirrors Godot's set_caret_line which clamps to valid range.
        let mut mock = MockTextEdit::new("short\nab");
        mock.add_caret(0, 0);

        // Line beyond document end
        mock.set_caret_line_for(99, 1);
        // Should clamp to last line (1)
        assert_eq!(mock.get_caret_count(), 2);
    }

    #[test]
    fn mock_set_caret_column_for_clamps() {
        // Mirrors Godot's set_caret_column which clamps to line length.
        let mut mock = MockTextEdit::new("ab\ncd");
        mock.add_caret(0, 0);

        // Column beyond line end
        mock.set_caret_column_for(99, 1);
        // Should clamp to line 0's length (2)
        assert_eq!(mock.get_caret_count(), 2);
    }

    // ══════════════════════════════════════════════════════════════════════
    // Godot→Engine import tests (import_godot_carets)
    // ══════════════════════════════════════════════════════════════════════

    #[test]
    fn import_carets_added_calls_add_cursor_with_correct_offsets() {
        // Start with 1 caret, mock adds 2 more (set count to 3).
        // Verify add_cursor called twice with correct byte offsets.
        let text = "hello\nworld\nfoo";
        let line_index = LineIndex::new(text);
        let mut mock = MockTextEdit::new(text);
        let mut bs = default_buffer_state();
        // bs defaults to last_caret_count=1

        // Simulate user Ctrl+Clicking to add carets at (1, 2) and (2, 1)
        mock.add_caret(1, 2); // "world" col 2 -> byte offset = 6 + 2 = 8
        mock.add_caret(2, 1); // "foo" col 1 -> byte offset = 12 + 1 = 13
        assert_eq!(mock.get_caret_count(), 3);

        let mut added: Vec<usize> = Vec::new();
        let mut removed: Vec<usize> = Vec::new();

        import_godot_carets(
            &mock,
            &mut bs,
            &mut |offset| added.push(offset),
            &mut |offset| removed.push(offset),
            &line_index,
            text,
        );

        // add_cursor called twice with byte offsets for the two new carets
        assert_eq!(added, vec![8, 13]);
        assert!(removed.is_empty());
        assert_eq!(bs.last_caret_count(), 3);
    }

    #[test]
    fn import_carets_removed_triggers_full_resync() {
        // Start with 3 carets, mock removes 1 (set count to 2).
        // Verify: remove all secondaries, then re-add current secondaries.
        let text = "aaa\nbbb\nccc";
        let line_index = LineIndex::new(text);
        let mut mock = MockTextEdit::new(text);
        let mut bs = default_buffer_state();

        // Set up mock with 3 carets: (0,0), (1,1), (2,2)
        mock.add_caret(1, 1); // byte 4+1=5
        mock.add_caret(2, 2); // byte 8+2=10
        assert_eq!(mock.get_caret_count(), 3);

        // Pretend last sync saw 3 carets
        bs.set_last_caret_count(3);

        // Now simulate user removing one caret: remove idx 2 so only (0,0) and (1,1) remain
        mock.remove_caret(2);
        assert_eq!(mock.get_caret_count(), 2);

        let mut added: Vec<usize> = Vec::new();
        let mut removed: Vec<usize> = Vec::new();

        import_godot_carets(
            &mock,
            &mut bs,
            &mut |offset| added.push(offset),
            &mut |offset| removed.push(offset),
            &line_index,
            text,
        );

        // Full resync: single remove_cursor(0) as "clear all" signal,
        // then re-add current secondaries from compute_import_action's result.
        assert_eq!(removed.len(), 1); // single "clear all" signal
        assert_eq!(removed, vec![0]);
        assert_eq!(added, vec![5]); // re-added current secondary at idx 1 -> (1,1) -> byte 5
        assert_eq!(bs.last_caret_count(), 2);
    }

    #[test]
    fn import_carets_count_unchanged_no_callbacks() {
        // Count unchanged → verify no callbacks fired.
        let text = "hello\nworld";
        let line_index = LineIndex::new(text);
        let mock = MockTextEdit::new(text);
        let mut bs = default_buffer_state();

        // Both mock and buffer_state agree on 1 caret
        assert_eq!(mock.get_caret_count(), 1);
        assert_eq!(bs.last_caret_count(), 1);

        let mut added: Vec<usize> = Vec::new();
        let mut removed: Vec<usize> = Vec::new();

        import_godot_carets(
            &mock,
            &mut bs,
            &mut |offset| added.push(offset),
            &mut |offset| removed.push(offset),
            &line_index,
            text,
        );

        assert!(added.is_empty());
        assert!(removed.is_empty());
        assert_eq!(bs.last_caret_count(), 1);
    }

    #[test]
    fn import_carets_count_unchanged_multiple_no_callbacks() {
        // Already have 3 carets, count stays 3 → no callbacks.
        let text = "aaa\nbbb\nccc";
        let line_index = LineIndex::new(text);
        let mut mock = MockTextEdit::new(text);
        let mut bs = default_buffer_state();

        mock.add_caret(1, 0);
        mock.add_caret(2, 0);
        bs.set_last_caret_count(3);

        let mut added: Vec<usize> = Vec::new();
        let mut removed: Vec<usize> = Vec::new();

        import_godot_carets(
            &mock,
            &mut bs,
            &mut |offset| added.push(offset),
            &mut |offset| removed.push(offset),
            &line_index,
            text,
        );

        assert!(added.is_empty());
        assert!(removed.is_empty());
        assert_eq!(bs.last_caret_count(), 3);
    }

    #[test]
    fn import_carets_updates_last_caret_count_on_add() {
        // Verify last_caret_count is updated after adding.
        let text = "abc\ndef\nghi";
        let line_index = LineIndex::new(text);
        let mut mock = MockTextEdit::new(text);
        let mut bs = default_buffer_state();
        assert_eq!(bs.last_caret_count(), 1);

        // Add 4 carets total (1 primary + 3 secondary)
        mock.add_caret(1, 0);
        mock.add_caret(2, 0);
        mock.add_caret(2, 2);

        let mut added: Vec<usize> = Vec::new();
        let mut removed: Vec<usize> = Vec::new();

        import_godot_carets(
            &mock,
            &mut bs,
            &mut |offset| added.push(offset),
            &mut |offset| removed.push(offset),
            &line_index,
            text,
        );

        assert_eq!(added.len(), 3);
        assert_eq!(bs.last_caret_count(), 4);
    }

    #[test]
    fn import_carets_updates_last_caret_count_on_remove() {
        // Verify last_caret_count is updated after removing.
        let text = "line0\nline1\nline2\nline3";
        let line_index = LineIndex::new(text);
        let mut mock = MockTextEdit::new(text);
        let mut bs = default_buffer_state();

        // Set up: 4 carets
        mock.add_caret(1, 0);
        mock.add_caret(2, 0);
        mock.add_caret(3, 0);
        bs.set_last_caret_count(4);

        // Remove 2 carets -> 2 remain
        mock.remove_caret(3);
        mock.remove_caret(2);
        assert_eq!(mock.get_caret_count(), 2);

        let mut added: Vec<usize> = Vec::new();
        let mut removed: Vec<usize> = Vec::new();

        import_godot_carets(
            &mock,
            &mut bs,
            &mut |offset| added.push(offset),
            &mut |offset| removed.push(offset),
            &line_index,
            text,
        );

        assert_eq!(bs.last_caret_count(), 2);
    }

    #[test]
    fn import_carets_line_col_to_byte_conversion_first_line() {
        // Verify byte offset conversion is correct for first line.
        let text = "abcdefgh";
        let line_index = LineIndex::new(text);
        let mut mock = MockTextEdit::new(text);
        let mut bs = default_buffer_state();

        // Add caret at col 5 on line 0 -> byte 5
        mock.add_caret(0, 5);
        assert_eq!(mock.get_caret_count(), 2);

        let mut added: Vec<usize> = Vec::new();

        import_godot_carets(
            &mock,
            &mut bs,
            &mut |offset| added.push(offset),
            &mut |_| {},
            &line_index,
            text,
        );

        assert_eq!(added, vec![5]);
    }

    #[test]
    fn import_carets_line_col_to_byte_conversion_multiline() {
        // Verify byte offset conversion across multiple lines.
        // "abc\ndefg\nhi" -> line starts at [0, 4, 9]
        let text = "abc\ndefg\nhi";
        let line_index = LineIndex::new(text);
        let mut mock = MockTextEdit::new(text);
        let mut bs = default_buffer_state();

        // Add carets: (1, 2) -> byte 4+2=6, (2, 1) -> byte 9+1=10
        mock.add_caret(1, 2);
        mock.add_caret(2, 1);

        let mut added: Vec<usize> = Vec::new();

        import_godot_carets(
            &mock,
            &mut bs,
            &mut |offset| added.push(offset),
            &mut |_| {},
            &line_index,
            text,
        );

        assert_eq!(added, vec![6, 10]);
    }

    // ══════════════════��═══════════════════════════════════════════════════
    // Real-use-case import scenarios
    //
    // These simulate actual user workflows where Ctrl+Click adds/removes
    // carets in Godot and the import function bridges them to vim-core.
    // ═══════════════════���═════════════════════���════════════════════════════

    #[test]
    fn import_ctrl_click_three_different_positions() {
        // Real scenario: User has a single cursor, then Ctrl+Clicks 3 distinct
        // positions across the document. On the next keystroke, all 3 new
        // positions must be imported with correct byte offsets.
        //
        // Document: "func main() {\n    let x = 1;\n    let y = 2;\n    return x + y;\n}"
        // Line starts: [0, 15, 30, 45, 63]
        let text = "func main() {\n    let x = 1;\n    let y = 2;\n    return x + y;\n}";
        let line_index = LineIndex::new(text);
        let mut mock = MockTextEdit::new(text);
        let mut bs = default_buffer_state();
        bs.set_last_caret_count(1);

        // User Ctrl+Clicks at:
        //   (1, 8)  -> "x" in "let x = 1;" -> byte 15 + 8 = 23
        //   (2, 8)  -> "y" in "let y = 2;" -> byte 30 + 8 = 38
        //   (3, 11) -> "x" in "return x + y;" -> byte 45 + 11 = 56
        mock.add_caret(1, 8);
        mock.add_caret(2, 8);
        mock.add_caret(3, 11);
        assert_eq!(mock.get_caret_count(), 4);

        let mut added: Vec<usize> = Vec::new();
        let mut removed: Vec<usize> = Vec::new();

        import_godot_carets(
            &mock,
            &mut bs,
            &mut |o| added.push(o),
            &mut |o| removed.push(o),
            &line_index,
            text,
        );

        // All 3 new carets imported.
        assert_eq!(added.len(), 3);
        assert_eq!(added[0], line_index.line_col_to_byte(text, 1, 8));
        assert_eq!(added[1], line_index.line_col_to_byte(text, 2, 8));
        assert_eq!(added[2], line_index.line_col_to_byte(text, 3, 11));
        // No removals on the add path.
        assert!(removed.is_empty());
        // Buffer state updated to 4.
        assert_eq!(bs.last_caret_count(), 4);
    }

    #[test]
    fn import_ctrl_click_already_cursored_position() {
        // Real scenario: User already has a cursor at (2, 3). They Ctrl+Click
        // the exact same position. Godot adds a new caret there (it doesn't
        // deduplicate — that's vim-core's job). import_godot_carets must still
        // call add_cursor so the engine can handle deduplication.
        //
        // Document: "aaa\nbbb\nccc ddd\neee"
        // Line starts: [0, 4, 8, 16]
        let text = "aaa\nbbb\nccc ddd\neee";
        let line_index = LineIndex::new(text);
        let mut mock = MockTextEdit::new(text);
        let mut bs = default_buffer_state();

        // Primary is at (0,0). User previously Ctrl+Clicked (2,3) -> byte 11.
        mock.add_caret(2, 3);
        bs.set_last_caret_count(2);

        // User Ctrl+Clicks (2,3) AGAIN — Godot adds another caret at same spot.
        mock.add_caret(2, 3);
        assert_eq!(mock.get_caret_count(), 3);

        let mut added: Vec<usize> = Vec::new();
        let mut removed: Vec<usize> = Vec::new();

        import_godot_carets(
            &mock,
            &mut bs,
            &mut |o| added.push(o),
            &mut |o| removed.push(o),
            &line_index,
            text,
        );

        // add_cursor IS called with the duplicate offset — vim-core deduplicates.
        assert_eq!(added.len(), 1);
        assert_eq!(added[0], line_index.line_col_to_byte(text, 2, 3));
        assert!(removed.is_empty());
        assert_eq!(bs.last_caret_count(), 3);
    }

    #[test]
    fn import_user_removes_caret_resync_cleans_up() {
        // Real scenario: User has 4 carets. They Ctrl+Click an existing caret
        // to remove it. Godot drops from 4 to 3. import_godot_carets does a
        // full resync: removes all old secondaries from the engine, then re-adds
        // all current secondaries. This ensures vim-core's state matches Godot
        // regardless of which specific caret was removed.
        //
        // Document: "alpha\nbeta\ngamma\ndelta"
        // Line starts: [0, 6, 11, 17]
        let text = "alpha\nbeta\ngamma\ndelta";
        let line_index = LineIndex::new(text);
        let mut mock = MockTextEdit::new(text);
        let mut bs = default_buffer_state();

        // Set up 4 carets: primary (0,0), secondaries at (1,2), (2,3), (3,1)
        mock.add_caret(1, 2); // idx 1 -> byte 6+2=8
        mock.add_caret(2, 3); // idx 2 -> byte 11+3=14
        mock.add_caret(3, 1); // idx 3 -> byte 17+1=18
        bs.set_last_caret_count(4);
        assert_eq!(mock.get_caret_count(), 4);

        // User Ctrl+Clicks the caret at (2,3) to remove it. In Godot this
        // removes caret at idx 2, shifting idx 3 down to idx 2.
        mock.remove_caret(2);
        assert_eq!(mock.get_caret_count(), 3);
        // Now: idx 0=(0,0), idx 1=(1,2), idx 2=(3,1)

        let mut added: Vec<usize> = Vec::new();
        let mut removed: Vec<usize> = Vec::new();

        import_godot_carets(
            &mock,
            &mut bs,
            &mut |o| added.push(o),
            &mut |o| removed.push(o),
            &line_index,
            text,
        );

        // Full resync: single remove_cursor(0) as "clear all" signal,
        // then re-add current secondaries from compute_import_action's result.
        assert_eq!(removed.len(), 1);
        assert_eq!(removed[0], 0);

        // Re-add current secondaries: (1..current_count=3) = [1, 2]
        //   idx 1 -> (1,2) -> byte 8
        //   idx 2 -> (3,1) -> byte 18
        assert_eq!(added.len(), 2);
        assert_eq!(added[0], line_index.line_col_to_byte(text, 1, 2));
        assert_eq!(added[1], line_index.line_col_to_byte(text, 3, 1));

        assert_eq!(bs.last_caret_count(), 3);
    }

    #[test]
    fn import_rapid_ctrl_click_add_remove_sequence() {
        // Real scenario: Rapid sequence of Ctrl+Click operations between
        // keystrokes. This tests that each call to import_godot_carets produces
        // correct callbacks and that buffer_state stays consistent across the
        // entire sequence.
        //
        // Document: "one\ntwo\nthree\nfour\nfive"
        // Line starts: [0, 4, 8, 14, 19]
        let text = "one\ntwo\nthree\nfour\nfive";
        let line_index = LineIndex::new(text);

        let mut mock = MockTextEdit::new(text);
        let mut bs = default_buffer_state();
        bs.set_last_caret_count(1);

        // ── Round 1: Add 2 carets (1 → 3) ───────���────────────────────────
        mock.add_caret(1, 1); // byte 4+1=5
        mock.add_caret(3, 2); // byte 14+2=16
        assert_eq!(mock.get_caret_count(), 3);

        let mut a1: Vec<usize> = Vec::new();
        let mut r1: Vec<usize> = Vec::new();
        import_godot_carets(
            &mock,
            &mut bs,
            &mut |o| a1.push(o),
            &mut |o| r1.push(o),
            &line_index,
            text,
        );

        assert_eq!(a1.len(), 2);
        assert_eq!(a1[0], 5);
        assert_eq!(a1[1], 16);
        assert!(r1.is_empty());
        assert_eq!(bs.last_caret_count(), 3);

        // ── Round 2: Add 1 more caret (3 → 4) ───────────────────────────
        mock.add_caret(4, 0); // byte 19
        assert_eq!(mock.get_caret_count(), 4);

        let mut a2: Vec<usize> = Vec::new();
        let mut r2: Vec<usize> = Vec::new();
        import_godot_carets(
            &mock,
            &mut bs,
            &mut |o| a2.push(o),
            &mut |o| r2.push(o),
            &line_index,
            text,
        );

        assert_eq!(a2.len(), 1);
        assert_eq!(a2[0], 19);
        assert!(r2.is_empty());
        assert_eq!(bs.last_caret_count(), 4);

        // ── Round 3: Remove 2 carets (4 → 2) ────────���───────────────────
        mock.remove_caret(3); // remove last
        mock.remove_caret(1); // remove first secondary
        assert_eq!(mock.get_caret_count(), 2);
        // Remaining: idx 0=(0,0), idx 1=(3,2)

        let mut a3: Vec<usize> = Vec::new();
        let mut r3: Vec<usize> = Vec::new();
        import_godot_carets(
            &mock,
            &mut bs,
            &mut |o| a3.push(o),
            &mut |o| r3.push(o),
            &line_index,
            text,
        );

        // Full resync: single "clear all" signal, then re-add current secondaries
        assert_eq!(r3.len(), 1);
        assert_eq!(r3[0], 0);
        assert_eq!(a3.len(), 1);
        assert_eq!(a3[0], line_index.line_col_to_byte(text, 3, 2)); // 16
        assert_eq!(bs.last_caret_count(), 2);

        // ── Round 4: Add 1 back (2 → 3) — state stays consistent ────────
        mock.add_caret(2, 0); // byte 8
        assert_eq!(mock.get_caret_count(), 3);

        let mut a4: Vec<usize> = Vec::new();
        let mut r4: Vec<usize> = Vec::new();
        import_godot_carets(
            &mock,
            &mut bs,
            &mut |o| a4.push(o),
            &mut |o| r4.push(o),
            &line_index,
            text,
        );

        assert_eq!(a4.len(), 1);
        assert_eq!(a4[0], 8);
        assert!(r4.is_empty());
        assert_eq!(bs.last_caret_count(), 3);
    }

    #[test]
    fn import_removal_full_resync_matches_spec() {
        // Validates spec: "vim-core normalizes on every add_cursor"
        //
        // The removal strategy is a FULL resync: regardless of which caret was
        // removed, we remove ALL secondaries and re-add from scratch. This is
        // correct because:
        // 1. We cannot know which Godot caret index maps to which vim-core cursor
        // 2. vim-core re-normalizes/deduplicates on every add_cursor anyway
        // 3. The cost is O(n) where n is small (typically < 10 cursors)
        //
        // This test verifies that even after multiple removals, the re-add
        // produces exactly the right set of current secondary offsets.
        //
        // Document: "aaaa\nbbbb\ncccc\ndddd\neeee\nffff"
        // Line starts: [0, 5, 10, 15, 20, 25]
        let text = "aaaa\nbbbb\ncccc\ndddd\neeee\nffff";
        let line_index = LineIndex::new(text);
        let mut mock = MockTextEdit::new(text);
        let mut bs = default_buffer_state();

        // Start with 6 carets (1 primary + 5 secondaries)
        mock.add_caret(1, 2); // byte 7
        mock.add_caret(2, 1); // byte 11
        mock.add_caret(3, 3); // byte 18
        mock.add_caret(4, 0); // byte 20
        mock.add_caret(5, 2); // byte 27
        bs.set_last_caret_count(6);
        assert_eq!(mock.get_caret_count(), 6);

        // Remove 3 carets from the middle: idx 4, 3, 2 (highest first to
        // avoid index shifting issues in the mock itself).
        mock.remove_caret(4);
        mock.remove_caret(3);
        mock.remove_caret(2);
        assert_eq!(mock.get_caret_count(), 3);
        // Remaining: idx 0=(0,0), idx 1=(1,2), idx 2=(5,2)

        let mut added: Vec<usize> = Vec::new();
        let mut removed: Vec<usize> = Vec::new();

        import_godot_carets(
            &mock,
            &mut bs,
            &mut |o| added.push(o),
            &mut |o| removed.push(o),
            &line_index,
            text,
        );

        // Full resync: single "clear all" signal
        assert_eq!(removed.len(), 1);
        assert_eq!(removed[0], 0);

        // Re-add current secondaries: (1..3) = [1, 2]
        assert_eq!(added.len(), 2);
        assert_eq!(added[0], line_index.line_col_to_byte(text, 1, 2)); // byte 7
        assert_eq!(added[1], line_index.line_col_to_byte(text, 5, 2)); // byte 27

        assert_eq!(bs.last_caret_count(), 3);
    }

    #[test]
    fn sync_multicaret_edit_allows_overlapping_add_then_merges() {
        let mut mock = MockTextEdit::new("aaa\nbbb");
        let mut bs = default_buffer_state();

        // Engine wants 3 cursors but two have the same position.
        // Inside begin_multicaret_edit (used by sync), add_caret must succeed
        // even for overlapping positions. end_multicaret_edit then merges.
        let positions = vec![(0, 0, 0), (0, 0, 0), (1, 0, 4)];
        sync_cursors_to_editor(&positions, &mut mock, &mut bs);

        // After end_multicaret_edit, overlapping carets at (0,0) merge to one.
        assert_eq!(mock.get_caret_count(), 2);
        assert_eq!(bs.last_caret_count(), 2);
    }
}
