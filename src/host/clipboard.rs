//! Reads the system clipboard via the [`ClipboardPort`] abstraction to
//! fulfill `HostRequest::ReadClipboard`.

use compact_str::CompactString;
use vim_core::execution::{HostRequestId, HostResult};

use crate::bridge::clipboard::ClipboardPort;

/// Fulfills `HostRequest::ReadClipboard` for `"*p` / `"+p` paste operations.
///
/// The [`ClipboardPort`] abstraction provides a unified cross-platform
/// clipboard API, avoiding the need for platform-specific clipboard access
/// (X11 selections, Wayland data offers, Win32 clipboard, etc.).
pub(super) fn handle_read_clipboard(id: HostRequestId, clipboard: &dyn ClipboardPort) -> HostResult {
    let text = clipboard.read();
    log::trace!("clipboard::read: len={}", text.len());
    HostResult::ClipboardText {
        id,
        text: CompactString::from(text),
    }
}
