//! Mirrors clipboard register (`*`, `+`) writes to the OS clipboard via
//! Godot's [`DisplayServer`].

use godot::classes::DisplayServer;
use godot::prelude::*;
use vim_core::primitives::RegisterName;

/// Mirror `*`/`+` register writes to the OS clipboard. vim-core owns
/// register storage — this is a one-way sync for clipboard integration.
pub(super) fn sync_register_to_clipboard(name: RegisterName, content: &str) {
    let ch = name.char();
    if ch == '*' || ch == '+' {
        log::trace!("sync_clipboard: register='{}' len={}", ch, content.len());
        set_system_clipboard(content);
    }
}

/// `:CopyToClipboard` — direct clipboard write, bypassing register storage.
pub(super) fn handle_copy_to_clipboard(content: &str) {
    log::trace!("copy_to_clipboard: len={}", content.len());
    set_system_clipboard(content);
}

fn set_system_clipboard(content: &str) {
    DisplayServer::singleton().clipboard_set(&GString::from(content));
}
