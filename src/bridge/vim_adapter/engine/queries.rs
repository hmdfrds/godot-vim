use vim_core::domain::position::Position;
use vim_core::state::global::repeat::RepeatableChange;
use vim_core::state::mode::{Mode, ReplaceMode, VisualKind};
#[cfg(test)]
use vim_core::state::registry::CommandRegistry;
use vim_core::state::VimState;

use crate::bridge::vim_adapter::managers::visual_tracker::{
    DirtyFlags, VisualSnapshot, VisualTracker,
};

use super::VimEngine;

impl VimEngine {
    #[inline]
    #[must_use]
    pub(crate) fn mode(&self) -> Mode {
        self.state.mode()
    }

    #[inline]
    #[must_use]
    pub(crate) fn is_visual(&self) -> bool {
        self.state.mode().is_visual()
    }

    #[inline]
    #[must_use]
    pub(crate) fn is_insert(&self) -> bool {
        matches!(self.state.mode(), Mode::Insert { .. })
    }

    #[inline]
    #[must_use]
    #[allow(dead_code)]
    pub(crate) fn is_replace(&self) -> bool {
        matches!(
            self.state.mode(),
            Mode::Replace(ReplaceMode::Overwrite) | Mode::Replace(ReplaceMode::Virtual)
        )
    }

    #[inline]
    #[must_use]
    #[allow(dead_code)]
    pub(crate) fn is_normal(&self) -> bool {
        matches!(self.state.mode(), Mode::Normal)
    }

    #[inline]
    #[must_use]
    pub(crate) fn is_cmdline(&self) -> bool {
        matches!(self.state.mode(), Mode::CmdLine(_))
    }

    #[inline]
    #[must_use]
    #[allow(dead_code)]
    pub(crate) fn is_visual_block(&self) -> bool {
        matches!(self.state.mode(), Mode::Visual(VisualKind::Block { .. }))
    }

    #[inline]
    #[must_use]
    pub(crate) fn cursor_pos(&self) -> Position {
        self.state.cursor_state.position
    }

    #[inline]
    #[must_use]
    #[allow(dead_code)]
    pub(crate) fn preferred_column(&self) -> Option<vim_core::domain::column::ByteCol> {
        self.state.cursor_state.preferred_column
    }

    #[inline]
    #[must_use]
    #[allow(dead_code)]
    pub(crate) fn is_recording(&self) -> bool {
        self.state.macros.recording.is_some()
    }

    #[inline]
    #[must_use]
    pub(crate) fn recording_register(&self) -> Option<char> {
        self.state.macros.recording
    }

    #[inline]
    #[must_use]
    pub(crate) fn visual_snapshot(&self) -> VisualSnapshot {
        VisualTracker::snapshot(&self.state)
    }

    #[inline]
    pub(crate) fn visual_diff(
        &self,
        before: &VisualSnapshot,
        tracker: &mut VisualTracker,
        has_transaction: bool,
    ) -> DirtyFlags {
        tracker.diff(before, &self.state, has_transaction)
    }

    pub(crate) fn register_entries(
        &self,
    ) -> impl Iterator<Item = (char, &vim_core::domain::shared_str::SharedStr)> {
        self.state.regs.storage.iter()
    }

    #[inline]
    #[must_use]
    pub(crate) fn register_get(
        &self,
        name: char,
    ) -> Option<&(
        vim_core::domain::shared_str::SharedStr,
        vim_core::domain::selection::SelectionMode,
    )> {
        self.state.regs.storage.get(name)
    }

    #[inline]
    #[must_use]
    pub(crate) fn last_search(&self) -> Option<&str> {
        self.state.search.last_search()
    }

    #[inline]
    #[must_use]
    pub(crate) fn last_search_forward(&self) -> bool {
        self.state.search.last_search_forward()
    }

    #[must_use]
    pub(crate) fn last_substitute(&self) -> (Option<&str>, Option<&str>, Option<&str>) {
        (
            self.state.search.last_substitute_pattern(),
            self.state.search.last_substitute_replacement(),
            self.state.search.last_substitute_flags(),
        )
    }

    #[inline]
    #[must_use]
    pub(crate) fn is_completion_visible(&self) -> bool {
        self.state.completion_visible
    }

    #[inline]
    #[must_use]
    #[allow(dead_code)]
    pub(crate) fn quantum_cursor(&self) -> Position {
        self.state.insert.quantum.cursor()
    }

    #[inline]
    #[must_use]
    pub(crate) fn last_insert_pos(&self) -> Option<Position> {
        self.state.visual.last_insert_pos
    }

    #[inline]
    #[must_use]
    pub(crate) fn last_change_pos(&self) -> Option<Position> {
        self.state.visual.last_change_pos
    }

    #[inline]
    #[must_use]
    pub(crate) fn last_jump_pos(&self) -> Option<Position> {
        self.state.visual.last_jump_pos
    }

    #[inline]
    #[must_use]
    pub(crate) fn last_visual_selection(&self) -> Option<vim_core::state::VisualSelection> {
        self.state.visual.last_selection
    }

    #[inline]
    #[must_use]
    pub(crate) fn get_mark(&self, name: char) -> Option<Position> {
        self.state.history.marks.get(name)
    }

    #[inline]
    #[must_use]
    #[cfg(test)]
    pub(crate) fn registry(&self) -> &CommandRegistry {
        &self.state.registry
    }

    #[inline]
    #[must_use]
    pub(crate) fn last_change(&self) -> Option<&RepeatableChange> {
        self.state.history.last_change.as_ref()
    }

    #[inline]
    #[must_use]
    pub(crate) fn current_line_snapshot(&self) -> &Option<(usize, String)> {
        &self.state.insert.line_snapshot
    }

    /// Read-only pass-through for key pipeline context assembly.
    #[inline]
    pub(crate) fn vim_state_ref(&self) -> &VimState {
        &self.state
    }

    /// Pop the next key from the macro call stack.
    ///
    /// Returns `None` when the stack is empty (playback complete).
    #[inline]
    pub(crate) fn pop_macro_key(&mut self) -> Option<vim_core::inputs::VimKey> {
        self.state.macros.call_stack.next_key()
    }

    /// Return `true` if the macro call stack has pending keys.
    #[inline]
    #[must_use]
    pub(crate) fn has_pending_macro_keys(&self) -> bool {
        self.state.macros.call_stack.is_active()
    }
}
