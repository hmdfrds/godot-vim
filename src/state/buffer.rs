//! Per-buffer persistent state: canonical visual selection, buffer-local
//! mappings, sticky scroll count, and undo tree.

use vim_core::keymap::BufferMappings;
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

    buffer_mappings: BufferMappings,

    /// Sticky half-page scroll count: once the user supplies an explicit count
    /// to `Ctrl-D`/`Ctrl-U`, that count persists for subsequent scrolls in
    /// this buffer (Vim `:help scroll` semantics).
    scroll_half_count: Option<u32>,

    undo_tree: Option<super::undo_tree::UndoTree>,
}

impl BufferState {
    // ── Accessors ────────────────────────────────────────────────────

    #[must_use]
    pub(crate) fn visual(&self) -> Option<&VisualSelectionState> {
        self.visual.as_ref()
    }

    #[must_use]
    pub(crate) fn buffer_mappings(&self) -> &BufferMappings {
        &self.buffer_mappings
    }

    pub(crate) fn buffer_mappings_mut(&mut self) -> &mut BufferMappings {
        &mut self.buffer_mappings
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

    #[must_use]
    #[allow(dead_code)] // callers pending scroll-half wiring
    pub(crate) fn scroll_half_count(&self) -> Option<u32> {
        self.scroll_half_count
    }

    pub(crate) fn set_scroll_half_count(&mut self, count: u32) {
        self.scroll_half_count = Some(count);
    }

    /// No-op if `init_undo_tree` was never called. `text` is the full document
    /// for periodic snapshot storage.
    pub(crate) fn record_undo_edit(&mut self, text: &str) {
        if let Some(ref mut tree) = self.undo_tree {
            tree.record_edit("edit", text);
        }
    }
}
