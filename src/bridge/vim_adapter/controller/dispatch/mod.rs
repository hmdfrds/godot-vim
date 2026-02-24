//! Dispatch methods for VimController.
//!
//! Handles `VimOutput` dispatch to concrete editor handlers.

mod editor_command;
mod pending_keys;
mod transaction;

use crate::bridge::vim_adapter::output::VimOutput;
use crate::bridge::vim_wrapper::VimController;

impl VimController {
    /// Applies a `VimOutput` from the adapter runtime.
    ///
    /// Processing order:
    /// 1. Transaction edits
    /// 2. Editor commands
    /// 3. Pending keys replay
    pub(crate) fn handle_vim_output(&mut self, output: VimOutput) -> bool {
        let has_transaction = output.has_transaction();

        if let Some(tx) = output.transaction {
            self.apply_transaction(tx);
        }

        for cmd in output.commands {
            self.dispatch_editor_command(cmd);
        }

        if !output.pending_keys.is_empty() {
            log::debug!(
                "Processing {} pending keys from macro/repeat",
                output.pending_keys.len()
            );
            for key in &output.pending_keys {
                self.process_vim_key_internal(key, false, false);
            }

            if let Some(editor) = self.get_editor() {
                let mut control = editor.clone().upcast::<godot::classes::Control>();
                control.grab_focus();
                log::debug!("Restored focus to editor after pending_keys playback");
            }
        }

        has_transaction
    }
}
