//! Reads the system clipboard via Godot's [`DisplayServer`] to fulfill
//! `HostRequest::ReadClipboard`.

use compact_str::CompactString;
use godot::classes::DisplayServer;
use godot::prelude::*;
use vim_core::execution::{HostRequestId, HostResult};

/// Fulfills `HostRequest::ReadClipboard` for `"*p` / `"+p` paste operations.
///
/// Godot's `DisplayServer::clipboard_get()` provides a unified cross-platform
/// clipboard API, avoiding the need for platform-specific clipboard access
/// (X11 selections, Wayland data offers, Win32 clipboard, etc.).
pub(super) fn handle_read_clipboard(id: HostRequestId) -> HostResult {
    let text = DisplayServer::singleton().clipboard_get().to_string();
    log::trace!("clipboard::read: len={}", text.len());
    HostResult::ClipboardText {
        id,
        text: CompactString::from(text),
    }
}
