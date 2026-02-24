//! Transaction handling methods for VimController.
//!
//! Applies transactions to the editor with Vim register semantics.

use crate::bridge::vim_adapter::core::transaction;
use crate::bridge::vim_wrapper::VimController;

impl VimController {
    /// Applies a transaction patch to the editor.
    ///
    /// This applies the atomic edits from the core.
    pub(crate) fn apply_transaction(&mut self, tx: vim_core::protocol::messages::TransactionPatch) {
        let Some(mut editor) = self.get_editor() else {
            return;
        };

        // Update last change position for `. mark, using the pre-edit cursor position.
        let pos = Self::cursor_from_editor(&editor);
        self.engine.set_last_change(pos);

        // Remove secondary carets to prevent multi-caret corruption
        editor.remove_secondary_carets();
        editor.begin_complex_operation();
        transaction::apply_transaction_patch(&mut editor, &tx);
        editor.end_complex_operation();
    }
}
