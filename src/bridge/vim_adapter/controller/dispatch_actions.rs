//! Output application and visual synchronization for VimController.

use crate::bridge::vim_adapter::controller::signals::SignalHandlersTrait;
use crate::bridge::vim_adapter::core::column_codec;
use crate::bridge::vim_adapter::managers::visual_tracker::{DirtyFlags, VisualSnapshot};
use crate::bridge::vim_wrapper::VimController;
use vim_core::state::mode::Mode;

impl VimController {
    pub(crate) fn apply_output_with_visuals(
        &mut self,
        prev_mode: Mode,
        snap: &VisualSnapshot,
        output: crate::bridge::vim_adapter::output::VimOutput,
    ) {
        let has_transaction = output.has_transaction();
        self.handle_vim_output(output);

        let dirty =
            self.engine
                .visual_diff(snap, &mut self.visuals.visual_tracker, has_transaction);

        if dirty.contains(DirtyFlags::CURSOR) {
            self.sync_cursor_to_editor();
        }

        if dirty.contains(DirtyFlags::MODE) {
            self.update_mode_visuals_if_changed(prev_mode);
        }

        if dirty.contains(DirtyFlags::SELECTION) {
            self.update_visual_selection();
        }

        if dirty.contains(DirtyFlags::CURSOR) {
            self.update_cursor_visual();
        }

        if dirty.contains(DirtyFlags::SEARCH) {
            if let Some(mut editor) = self.get_editor() {
                self.sync_search_highlight(&mut editor);
            }
        }
    }

    /// Sync vim cursor state to editor caret.
    pub(crate) fn sync_cursor_to_editor(&mut self) {
        if let Some(mut editor) = self.get_editor() {
            let target = self.engine.cursor_pos();
            column_codec::apply_core_position_to_editor(&mut editor, target);
        }
    }
}
