//! Per-buffer persistent state: canonical visual selection, engine-side
//! per-buffer state, and undo tree.

use vim_core::execution::BufferLocalState;
use vim_core::primitives::Offset;

use crate::types::CharLineCol;

/// Shell-owned visual selection state, set and cleared atomically during
/// effect dispatch where `final_text` is already available.
#[derive(Debug, Clone, Copy)]
pub(crate) struct VisualSelectionState {
    pub(crate) anchor: Offset,
    pub(crate) head: Offset,
    /// Pre-computed during effect dispatch to avoid an extra `get_text()`
    /// FFI round-trip in `ui_snapshot()`.
    pub(crate) head_pos: CharLineCol,
}

#[derive(Debug, Default)]
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

    // ── Mutation ─────────────────────────────────────────────────────

    pub(crate) fn update_visual_selection(
        &mut self,
        anchor: Offset,
        head: Offset,
        head_pos: CharLineCol,
    ) {
        self.visual = Some(VisualSelectionState {
            anchor,
            head,
            head_pos,
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
