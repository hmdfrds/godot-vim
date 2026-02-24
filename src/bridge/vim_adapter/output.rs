//! VimOutput — The adapter's output type.
//!
//! Returned by VimEngine after processing a key or action.
//! Contains all the information the shell needs to update the UI.

use vim_core::inputs::VimKey;
use vim_core::protocol::messages::TransactionPatch;

use crate::bridge::types::command::EditorCommand;

/// Output from the VimEngine after processing a key event or action.
///
/// The shell iterates `commands` to dispatch side effects, applies the
/// `transaction` to the document, and replays `pending_keys` for macros.
#[derive(Debug)]
pub struct VimOutput {
    /// Editor commands the shell must execute (mode changes, file ops, etc.)
    pub commands: Vec<EditorCommand>,
    /// Raw transaction patch to apply (text edits). `None` if no text was changed.
    /// Uses vim-core's TransactionPatch type since it's within the adapter boundary.
    pub transaction: Option<TransactionPatch>,
    /// Pending keys for macro replay / repeat. Empty if none.
    pub pending_keys: Vec<VimKey>,
}

impl VimOutput {
    /// Creates a no-op output (nothing changed).
    #[must_use]
    pub fn noop() -> Self {
        Self {
            commands: Vec::new(),
            transaction: None,
            pending_keys: Vec::new(),
        }
    }

    /// Returns `true` if this output contains a text transaction.
    #[inline]
    #[must_use]
    pub fn has_transaction(&self) -> bool {
        self.transaction.is_some()
    }
}

impl Default for VimOutput {
    fn default() -> Self {
        Self::noop()
    }
}
