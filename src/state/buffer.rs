//! Per-buffer persistent state: canonical visual selection, engine-side
//! per-buffer state, and undo tree.

use vim_core::execution::BufferLocalState;
use vim_core::primitives::Offset;

use crate::types::CharLineCol;

/// Active Ctrl+D session. Locks the search word at session start and tracks
/// the Godot caret index of the last-added match. The caret position is read
/// live from Godot on each Ctrl+D, so text edits between presses are handled
/// correctly (Godot adjusts caret positions after mutations).
#[derive(Debug, Clone)]
pub(crate) struct MatchSession {
    pub(crate) word: String,
    pub(crate) last_caret_index: i32,
}

/// Shell-owned visual selection state, set and cleared atomically during
/// effect dispatch where `final_text` is already available.
#[derive(Debug, Clone, Copy)]
pub(crate) struct VisualSelectionState {
    pub(crate) anchor: Offset,
    pub(crate) head: Offset,
    /// Pre-computed during effect dispatch to avoid an extra `get_text()`
    /// FFI round-trip in `ui_snapshot()`.
    pub(crate) head_pos: CharLineCol,
    /// Pre-computed anchor position, used by the block visual overlay.
    pub(crate) anchor_pos: CharLineCol,
}

#[derive(Debug)]
pub(crate) struct BufferState {
    /// Shell-owned selection truth. We cannot read selection state back from
    /// Godot's `select()` API because it loses collapsed selections, corrupts
    /// line-mode selections when `SetCursor` fires, and cannot represent
    /// block-mode anchor/head spanning multiple lines.
    visual: Option<VisualSelectionState>,

    /// Engine-side per-buffer state (marks, changelist, last_visual, sticky_column,
    /// buffer_overrides, buffer_mappings, exchange). Saved by `on_buffer_leave`,
    /// restored by `on_buffer_enter`. `None` for buffers not yet visited.
    engine_state: Option<BufferLocalState>,

    undo_tree: Option<super::undo_tree::UndoTree>,

    /// How many Godot carets existed on the last frame. Used to detect
    /// mouse-added/removed carets.
    last_caret_count: usize,

    /// Cursor positions (line, col, byte_offset) saved by SaveSelections effect,
    /// restored by RestoreSelections.
    saved_selections: Option<Vec<(usize, usize, usize)>>,

    /// Active Ctrl+D (AddNextMatch) session. Tracks the locked search word
    /// and the Godot caret index of the last-added match. On each Ctrl+D,
    /// the search start is computed by reading the live caret position from
    /// Godot (immune to text edits), not a stored byte offset.
    /// Destroyed when cursor count drops to 1 or the word under cursor changes.
    match_session: Option<MatchSession>,
}

impl Default for BufferState {
    fn default() -> Self {
        Self {
            visual: None,
            engine_state: None,
            undo_tree: None,
            last_caret_count: 1,
            saved_selections: None,
            match_session: None,
        }
    }
}

impl BufferState {
    // ── Accessors ────────────────────────────────────────────────────

    #[must_use]
    pub(crate) fn visual(&self) -> Option<&VisualSelectionState> {
        self.visual.as_ref()
    }

    pub(crate) fn set_engine_state(&mut self, state: BufferLocalState) {
        self.engine_state = Some(state);
    }

    pub(crate) fn take_engine_state(&mut self) -> Option<BufferLocalState> {
        self.engine_state.take()
    }

    #[must_use]
    pub(crate) fn undo_tree(&self) -> Option<&super::undo_tree::UndoTree> {
        self.undo_tree.as_ref()
    }

    /// Captures the initial document text as the undo tree root snapshot.
    pub(crate) fn init_undo_tree(&mut self, text: &str) {
        self.undo_tree = Some(super::undo_tree::UndoTree::new(text));
    }

    // ── Multi-cursor tracking ─────────────────────────────────────────

    #[must_use]
    pub(crate) fn last_caret_count(&self) -> usize {
        self.last_caret_count
    }

    pub(crate) fn set_last_caret_count(&mut self, count: usize) {
        self.last_caret_count = count;
    }

    #[must_use]
    pub(crate) fn saved_selections(&self) -> Option<&[(usize, usize, usize)]> {
        self.saved_selections.as_deref()
    }

    pub(crate) fn save_selections(&mut self, positions: Vec<(usize, usize, usize)>) {
        self.saved_selections = Some(positions);
    }

    pub(crate) fn clear_saved_selections(&mut self) {
        self.saved_selections = None;
    }

    pub(crate) fn match_session(&self) -> Option<&MatchSession> {
        self.match_session.as_ref()
    }

    pub(crate) fn set_match_session(&mut self, word: String, caret_index: i32) {
        self.match_session = Some(MatchSession {
            word,
            last_caret_index: caret_index,
        });
    }

    pub(crate) fn clear_match_session(&mut self) {
        self.match_session = None;
    }

    // ── Mutation ─────────────────────────────────────────────────────

    pub(crate) fn update_visual_selection(
        &mut self,
        anchor: Offset,
        head: Offset,
        head_pos: CharLineCol,
        anchor_pos: CharLineCol,
    ) {
        self.visual = Some(VisualSelectionState {
            anchor,
            head,
            head_pos,
            anchor_pos,
        });
    }

    pub(crate) fn clear_visual_selection(&mut self) {
        self.visual = None;
    }

    /// No-op if `init_undo_tree` was never called. `text` is the full document
    /// for periodic snapshot storage.
    pub(crate) fn record_undo_edit(&mut self, text: &str) {
        if let Some(ref mut tree) = self.undo_tree {
            tree.record_edit("edit", text);
        }
    }
}
