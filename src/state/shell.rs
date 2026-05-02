//! Top-level shell state container: per-buffer state keyed by Godot
//! `InstanceId`, global state, and transient UI data (substitute preview,
//! highlight yank).

use std::collections::HashMap;

use godot::prelude::InstanceId;

use super::buffer::BufferState;
use super::globals::GlobalState;
use crate::types::{HighlightYank, MatchRange};

/// Top-level shell state. Keyed by Godot `InstanceId` rather than an abstract
/// buffer ID -- couples us to Godot's type system, but `InstanceId` is a
/// lightweight `Copy` type and avoids an extra ID-mapping indirection.
#[derive(Debug, Default)]
pub(crate) struct ShellState {
    buffers: HashMap<InstanceId, BufferState>,
    globals: GlobalState,
    /// Transient per-dispatch-cycle data consumed by the controller to update
    /// the UI overlay. `Some(vec![])` = "clear preview"; `None` = "no change".
    substitute_preview: Option<Vec<MatchRange>>,
    /// Transient per-dispatch-cycle yank highlight, consumed by the controller.
    highlight_yank: Option<HighlightYank>,
}

impl ShellState {
    pub(crate) fn buffer(&mut self, id: InstanceId) -> &mut BufferState {
        self.buffers.entry(id).or_default()
    }

    /// Unlike `buffer()`, does not insert a default entry for unknown IDs.
    #[must_use]
    pub(crate) fn buffer_ref(&self, id: InstanceId) -> Option<&BufferState> {
        self.buffers.get(&id)
    }

    /// Evict buffer entries whose `InstanceId` is no longer valid (e.g.
    /// editor tab closed). Returns removed IDs for downstream cleanup
    /// (global marks, etc.). Predicate-based so this module stays free
    /// of direct Godot API calls.
    pub(crate) fn sweep_invalid_buffers(
        &mut self,
        is_valid: impl Fn(InstanceId) -> bool,
    ) -> Vec<InstanceId> {
        let mut removed = Vec::new();
        self.buffers.retain(|&id, _| {
            if is_valid(id) {
                true
            } else {
                removed.push(id);
                false
            }
        });
        removed
    }

    #[must_use]
    pub(crate) fn globals(&self) -> &GlobalState {
        &self.globals
    }

    pub(crate) fn globals_mut(&mut self) -> &mut GlobalState {
        &mut self.globals
    }

    // ── Substitute preview ─────────────────────────────────────────────

    pub(crate) fn set_substitute_preview(&mut self, positions: Vec<MatchRange>) {
        self.substitute_preview = Some(positions);
    }

    /// `Some(vec![])` signals "clear existing highlights" to the controller.
    pub(crate) fn clear_substitute_preview(&mut self) {
        self.substitute_preview = Some(Vec::new());
    }

    pub(crate) fn take_substitute_preview(&mut self) -> Option<Vec<MatchRange>> {
        self.substitute_preview.take()
    }

    // ── Highlight yank ─────────────────────────────────────────────────

    #[allow(dead_code)] // Setter for yank highlight — currently unused after vim-core removed HighlightYank effect
    pub(crate) fn set_highlight_yank(&mut self, yank: HighlightYank) {
        self.highlight_yank = Some(yank);
    }

    pub(crate) fn take_highlight_yank(&mut self) -> Option<HighlightYank> {
        self.highlight_yank.take()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_id(n: i64) -> InstanceId {
        InstanceId::from_i64(n)
    }

    #[test]
    fn buffer_creates_on_first_access() {
        let mut state = ShellState::default();
        let id = test_id(1);
        let buf = state.buffer(id);
        assert!(buf.visual().is_none());
    }

    #[test]
    fn buffer_returns_same_instance() {
        let mut state = ShellState::default();
        let id = test_id(1);
        assert!(state.buffer(id).visual().is_none());
        assert!(state.buffer(id).visual().is_none());
    }

    #[test]
    fn globals_accessors() {
        let mut state = ShellState::default();
        state.globals_mut().set_message("hello");
        assert_eq!(state.globals().message_status().text(), Some("hello"));
    }

    #[test]
    fn default_is_empty() {
        let state = ShellState::default();
        assert!(state.globals().message_status().text().is_none());
    }

    #[test]
    fn sweep_all_valid_removes_nothing() {
        let mut state = ShellState::default();
        let _b1 = state.buffer(test_id(1));
        let _b2 = state.buffer(test_id(2));
        let removed = state.sweep_invalid_buffers(|_| true);
        assert!(removed.is_empty());
        assert!(state.buffer_ref(test_id(1)).is_some());
        assert!(state.buffer_ref(test_id(2)).is_some());
    }

    #[test]
    fn sweep_all_invalid_removes_everything() {
        let mut state = ShellState::default();
        state.buffer(test_id(1));
        state.buffer(test_id(2));
        state.buffer(test_id(3));
        let mut removed = state.sweep_invalid_buffers(|_| false);
        removed.sort_by_key(|id| id.to_i64());
        assert_eq!(removed.len(), 3);
        assert!(state.buffer_ref(test_id(1)).is_none());
        assert!(state.buffer_ref(test_id(2)).is_none());
        assert!(state.buffer_ref(test_id(3)).is_none());
    }

    #[test]
    fn sweep_mixed_removes_only_invalid() {
        let mut state = ShellState::default();
        let _b1 = state.buffer(test_id(1));
        let _b2 = state.buffer(test_id(2));
        let _b3 = state.buffer(test_id(3));
        // Only id=2 is "invalid"
        let removed = state.sweep_invalid_buffers(|id| id != test_id(2));
        assert_eq!(removed, vec![test_id(2)]);
        assert!(state.buffer_ref(test_id(1)).is_some());
        assert!(state.buffer_ref(test_id(2)).is_none());
        assert!(state.buffer_ref(test_id(3)).is_some());
    }

    #[test]
    fn sweep_empty_map_is_noop() {
        let mut state = ShellState::default();
        let removed = state.sweep_invalid_buffers(|_| false);
        assert!(removed.is_empty());
    }
}
