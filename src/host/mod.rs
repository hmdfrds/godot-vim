//! Host request fulfillment — the Godot side of the host request/response protocol.
//!
//! The vim-core engine emits [`HostRequest`]s for side effects it cannot perform
//! (file I/O, clipboard, shell commands, buffer navigation, Godot-specific
//! editor operations). Each sub-module handles one domain. Every request carries
//! a [`HostRequestId`] and resolves to a [`HostResult`] fed back via
//! `complete_host_request()`, closing the async request lifecycle.

mod buffer;
mod clipboard;
mod custom_commands;
mod dispatch;
pub(crate) mod editor_host;
pub(crate) mod error;
mod eval;
mod external;
pub(crate) mod file;

pub(crate) use dispatch::execute;
pub(crate) use dispatch::SecurityPolicy;
pub(crate) use editor_host::GodotEditorHost;

use vim_core::execution::{HostRequestId, HostResult};
use vim_core::VimString;

pub(crate) fn host_failure(id: HostRequestId, msg: impl Into<VimString>) -> HostResult {
    HostResult::Failure {
        id,
        error: msg.into(),
    }
}

pub(crate) fn host_success(id: HostRequestId) -> HostResult {
    HostResult::Success { id, message: None }
}

/// Whether the host runs one of godot-vim's own commands for the command
/// line `command`, as `GodotHost::handle_request_inner` and
/// `handle_custom_ex_command` match it.
pub(crate) fn runs_own_command(command: &str) -> bool {
    let command = command.trim();
    custom_commands::list_all_commands().contains(&command)
        || command == "perf reset"
        || command.starts_with("vimdebug")
        || command.starts_with("panelmap ")
}

/// Whether `command` names one of godot-vim's own commands, with or
/// without arguments, so an argument form the host adds later is covered.
pub(crate) fn names_own_command(command: &str) -> bool {
    let name = command
        .trim_start()
        .split(|c: char| !c.is_ascii_alphanumeric())
        .next()
        .unwrap_or_default();
    runs_own_command(command) || custom_commands::list_all_commands().contains(&name)
}
