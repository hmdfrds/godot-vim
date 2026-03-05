//! Visual change tracking with dirty flags.
//!
//! ## Purpose
//! Tracks which visual elements need updating after an action,
//! Only the changed elements are redrawn.
//!
//! ## How It Works
//! 1. Call `snapshot()` before executing an action to capture current state
//! 2. Call `diff()` after execution to compute what changed
//! 3. Use the returned `DirtyFlags` to conditionally call visual updates
//!
//! ## Performance Impact
//! Without tracking: every action triggers 4 visual updates unconditionally.
//! With tracking: only changed visuals are updated. For simple motions (hjkl),
//! this skips mode, selection, and search updates entirely.

use vim_core::domain::position::Position;
use vim_core::state::mode::Mode;

/// Bitfield tracking which visual elements need updating.
///
/// Each flag corresponds to one visual subsystem. Only subsystems
/// with their flag set need to be refreshed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct DirtyFlags(u8);

impl DirtyFlags {
    /// Nothing changed - skip all visual updates.
    pub const EMPTY: Self = Self(0);
    /// Mode indicator needs update (cmdline text, cursor shape).
    pub const MODE: Self = Self(1 << 0);
    /// Cursor position changed (sync caret + overlay).
    pub const CURSOR: Self = Self(1 << 1);
    /// Selection changed (Visual mode highlighting).
    pub const SELECTION: Self = Self(1 << 2);
    /// Search highlights changed.
    pub const SEARCH: Self = Self(1 << 3);
    /// Everything changed - update all visuals.
    pub const ALL: Self = Self(0b0000_1111);

    /// Returns true if no flags are set.
    #[inline]
    #[cfg(test)]
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// Returns true if `other` flags are all set in `self`.
    #[inline]
    pub const fn contains(self, other: Self) -> bool {
        (self.0 & other.0) == other.0
    }
}

impl std::ops::BitOr for DirtyFlags {
    type Output = Self;

    #[inline]
    fn bitor(self, rhs: Self) -> Self::Output {
        Self(self.0 | rhs.0)
    }
}

impl std::ops::BitOrAssign for DirtyFlags {
    #[inline]
    fn bitor_assign(&mut self, rhs: Self) {
        self.0 |= rhs.0;
    }
}

/// Snapshot of visual-relevant state taken before action execution.
///
/// Compared against post-action state to determine what changed.
#[derive(Debug, Clone)]
pub struct VisualSnapshot {
    pub mode: Mode,
    pub cursor: Position,
    pub search: Option<String>,
}

/// Tracks visual state changes to enable conditional updates.
///
/// ## Usage
/// ```ignore
/// let snap = tracker.snapshot(&vim_state);
/// // ... execute action ...
/// let dirty = tracker.diff(&snap, &vim_state, &effects);
///
/// if dirty.contains(DirtyFlags::CURSOR) {
///     sync_cursor_to_editor();
/// }
/// if dirty.contains(DirtyFlags::MODE) {
///     update_mode_visuals();
/// }
/// ```
#[derive(Debug, Default)]
pub struct VisualTracker {
    /// Force all visuals to update on next diff (used after attach, etc.)
    force_all: bool,
}

impl VisualTracker {
    /// Creates a new tracker.
    #[must_use]
    pub fn new() -> Self {
        Self { force_all: false }
    }

    /// Take a snapshot of the current visual-relevant state.
    ///
    /// Call this **before** executing an action.
    pub fn snapshot(state: &vim_core::state::VimState) -> VisualSnapshot {
        VisualSnapshot {
            mode: state.mode(),
            cursor: state.cursor_state.position,
            search: state.search.last_search().map(String::from),
        }
    }

    /// Compute what changed by comparing pre-action snapshot to current state.
    ///
    /// Call this **after** executing an action.
    ///
    /// Returns `DirtyFlags` indicating which visual subsystems need updating.
    #[must_use]
    pub fn diff(
        &mut self,
        before: &VisualSnapshot,
        state: &vim_core::state::VimState,
        has_transaction: bool,
    ) -> DirtyFlags {
        // Force-all mode: clear flag and return ALL
        if self.force_all {
            self.force_all = false;
            return DirtyFlags::ALL;
        }

        let mut dirty = DirtyFlags::EMPTY;

        let current_mode = state.mode();

        // Mode changed.
        if before.mode != current_mode {
            dirty |= DirtyFlags::MODE;
            // Mode change also implies cursor shape change.
            dirty |= DirtyFlags::CURSOR;
            // Entering or leaving Visual mode means the selection changed.
            if before.mode.is_visual() || current_mode.is_visual() {
                dirty |= DirtyFlags::SELECTION;
            }
        }

        // Cursor moved.
        if before.cursor != state.cursor_state.position {
            dirty |= DirtyFlags::CURSOR;
        }

        // Search pattern changed.
        if before.search.as_deref() != state.search.last_search() {
            dirty |= DirtyFlags::SEARCH;
        }

        // Transaction means text was edited - cursor, selection, and search
        // highlights may all be affected by text changes
        if has_transaction {
            dirty |= DirtyFlags::CURSOR | DirtyFlags::SELECTION | DirtyFlags::SEARCH;
        }

        // Visual mode: cursor movement always means selection changed
        // because the selection endpoint tracks the cursor
        if current_mode.is_visual() && dirty.contains(DirtyFlags::CURSOR) {
            dirty |= DirtyFlags::SELECTION;
        }

        dirty
    }

    /// Force all visuals to update on the next `diff()` call.
    ///
    /// Use after editor attach, settings reload, or other full-invalidation events.
    pub fn invalidate_all(&mut self) {
        self.force_all = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vim_core::state::mode::{InsertMode, VisualKind};
    use vim_core::state::VimState;

    #[test]
    fn test_no_change_produces_empty_flags() {
        let mut tracker = VisualTracker::new();
        let state = VimState::new();
        let snap = VisualTracker::snapshot(&state);

        let dirty = tracker.diff(&snap, &state, false);

        assert!(dirty.is_empty());
    }

    #[test]
    fn test_cursor_move_sets_cursor_flag() {
        let mut tracker = VisualTracker::new();
        let mut state = VimState::new();
        let snap = VisualTracker::snapshot(&state);

        state.set_cursor_pos(Position::from_byte(5, 0));

        let dirty = tracker.diff(&snap, &state, false);

        assert!(dirty.contains(DirtyFlags::CURSOR));
        assert!(!dirty.contains(DirtyFlags::MODE));
        assert!(!dirty.contains(DirtyFlags::SELECTION));
        assert!(!dirty.contains(DirtyFlags::SEARCH));
    }

    #[test]
    fn test_mode_change_sets_mode_and_cursor_flags() {
        let mut tracker = VisualTracker::new();
        let mut state = VimState::new();
        let snap = VisualTracker::snapshot(&state);

        state.set_mode(Mode::Insert(InsertMode::Standard { count: 1 }));

        let dirty = tracker.diff(&snap, &state, false);

        assert!(dirty.contains(DirtyFlags::MODE));
        assert!(dirty.contains(DirtyFlags::CURSOR)); // cursor shape changes with mode
    }

    #[test]
    fn test_entering_visual_mode_sets_selection_flag() {
        let mut tracker = VisualTracker::new();
        let mut state = VimState::new();
        let snap = VisualTracker::snapshot(&state);

        state.set_mode(Mode::Visual(VisualKind::Char {
            start: Position::from_byte(0, 0),
        }));

        let dirty = tracker.diff(&snap, &state, false);

        assert!(dirty.contains(DirtyFlags::MODE));
        assert!(dirty.contains(DirtyFlags::SELECTION));
    }

    #[test]
    fn test_cursor_move_in_visual_mode_sets_selection() {
        let mut tracker = VisualTracker::new();
        let mut state = VimState::new();
        state.set_mode(Mode::Visual(VisualKind::Char {
            start: Position::from_byte(0, 0),
        }));
        let snap = VisualTracker::snapshot(&state);

        state.set_cursor_pos(Position::from_byte(2, 0));

        let dirty = tracker.diff(&snap, &state, false);

        assert!(dirty.contains(DirtyFlags::CURSOR));
        assert!(dirty.contains(DirtyFlags::SELECTION)); // visual mode cursor move = selection change
    }

    #[test]
    fn test_search_change_sets_search_flag() {
        let mut tracker = VisualTracker::new();
        let mut state = VimState::new();
        let snap = VisualTracker::snapshot(&state);

        state.search.set_search("hello".to_string(), true);

        let dirty = tracker.diff(&snap, &state, false);

        assert!(dirty.contains(DirtyFlags::SEARCH));
        assert!(!dirty.contains(DirtyFlags::MODE));
    }

    #[test]
    fn test_transaction_sets_cursor_selection_search() {
        let mut tracker = VisualTracker::new();
        let state = VimState::new();
        let snap = VisualTracker::snapshot(&state);

        let dirty = tracker.diff(&snap, &state, true);

        assert!(dirty.contains(DirtyFlags::CURSOR));
        assert!(dirty.contains(DirtyFlags::SELECTION));
        assert!(dirty.contains(DirtyFlags::SEARCH));
    }

    #[test]
    fn test_invalidate_all_forces_full_update() {
        let mut tracker = VisualTracker::new();
        tracker.invalidate_all();

        let state = VimState::new();
        let snap = VisualTracker::snapshot(&state);

        let dirty = tracker.diff(&snap, &state, false);

        assert_eq!(dirty, DirtyFlags::ALL);
    }

    #[test]
    fn test_invalidate_clears_after_one_diff() {
        let mut tracker = VisualTracker::new();
        tracker.invalidate_all();

        let state = VimState::new();
        let snap = VisualTracker::snapshot(&state);

        // First diff: forced ALL
        let dirty1 = tracker.diff(&snap, &state, false);
        assert_eq!(dirty1, DirtyFlags::ALL);

        // Second diff: back to normal
        let dirty2 = tracker.diff(&snap, &state, false);
        assert!(dirty2.is_empty());
    }

    #[test]
    fn test_flag_combination() {
        let combined = DirtyFlags::MODE | DirtyFlags::CURSOR;
        assert!(combined.contains(DirtyFlags::MODE));
        assert!(combined.contains(DirtyFlags::CURSOR));
        assert!(!combined.contains(DirtyFlags::SEARCH));
    }
}
