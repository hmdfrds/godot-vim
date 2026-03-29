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
mod eval;
mod external;
pub(crate) mod file;

pub(crate) use dispatch::execute;
pub(crate) use dispatch::SecurityPolicy;
pub(crate) use editor_host::GodotEditorHost;

use compact_str::CompactString;
use vim_core::execution::{HostRequestId, HostResult};

pub(crate) fn host_failure(id: HostRequestId, msg: impl Into<CompactString>) -> HostResult {
    HostResult::Failure { id, error: msg.into() }
}

pub(crate) fn host_success(id: HostRequestId) -> HostResult {
    HostResult::Success { id, message: None }
}
