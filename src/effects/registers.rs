//! Mirrors clipboard register (`*`, `+`) writes to the OS clipboard via
//! the [`ClipboardPort`] abstraction.

use crate::bridge::clipboard::ClipboardPort;
use vim_core::primitives::RegisterName;

/// Mirror `*`/`+` register writes to the OS clipboard. vim-core owns
/// register storage — this is a one-way sync for clipboard integration.
pub(super) fn sync_register_to_clipboard(
    name: RegisterName,
    content: &str,
    clipboard: &mut dyn ClipboardPort,
) {
    let ch = name.char();
    if ch == '*' || ch == '+' {
        log::trace!("sync_clipboard: register='{}' len={}", ch, content.len());
        clipboard.write(content);
    }
}

/// `:CopyToClipboard` — direct clipboard write, bypassing register storage.
pub(super) fn handle_copy_to_clipboard(content: &str, clipboard: &mut dyn ClipboardPort) {
    log::trace!("copy_to_clipboard: len={}", content.len());
    clipboard.write(content);
}
