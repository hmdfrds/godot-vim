//! Cross-buffer global state: status bar messages and hlsearch toggle.
//!
//! Global because these apply to the editor session, not individual buffers.
//! Status messages are shared across tabs (Vim shows one status line),
//! and `hlsearch` is a global setting in Vim.

use compact_str::CompactString;

use crate::types::StatusMessage;

#[derive(Debug)]
pub(crate) struct GlobalState {
    message: StatusMessage,
    /// Mirrors Vim's `hlsearch` state: `:noh` suppresses highlights without
    /// clearing the search pattern (so `n`/`N` still reuse it). Any new
    /// search command (e.g. `/foo`) re-enables highlighting.
    hlsearch_enabled: bool,
}

/// Manual `Default` because `hlsearch_enabled` must start as `true` (Vim's default).
impl Default for GlobalState {
    fn default() -> Self {
        Self {
            message: StatusMessage::None,
            hlsearch_enabled: true,
        }
    }
}

impl GlobalState {
    // ── Message ──────────────────────────────────────────────────────

    #[must_use]
    pub(crate) fn message_status(&self) -> &StatusMessage {
        &self.message
    }

    pub(crate) fn set_message(&mut self, text: impl Into<CompactString>) {
        self.message = StatusMessage::Info(text.into());
    }

    pub(crate) fn set_error(&mut self, text: impl Into<CompactString>) {
        self.message = StatusMessage::Error(text.into());
    }

    pub(crate) fn clear_message(&mut self) {
        self.message = StatusMessage::None;
    }

    // ── Search highlight ──────────────────────────────────────────────

    #[must_use]
    pub(crate) fn hlsearch_enabled(&self) -> bool {
        self.hlsearch_enabled
    }

    pub(crate) fn set_hlsearch_enabled(&mut self, enabled: bool) {
        self.hlsearch_enabled = enabled;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hlsearch_enabled_by_default() {
        assert!(GlobalState::default().hlsearch_enabled());
    }
}
