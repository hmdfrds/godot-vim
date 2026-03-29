//! Shell-side persistent state: per-buffer state, global state, and undo tree.
//!
//! The engine is stateless with respect to the host environment — it does not
//! know about Godot `InstanceId`s, editor tabs, or the OS clipboard. This module
//! bridges that gap by keying per-buffer state on `InstanceId` and maintaining
//! global state (messages, search highlight toggle) across all buffers.
//!
//! - [`buffer`] — per-editor visual selection, scroll count, buffer-local mappings
//! - [`globals`] — cross-buffer status messages, hlsearch flag, substitute preview
//! - [`shell`] — `ShellState` container with `HashMap<InstanceId, BufferState>`
//! - [`undo_tree`] — per-buffer branching undo tree with snapshot/pruning

pub(crate) mod buffer;
pub(crate) mod globals;
mod shell;
pub(crate) mod undo_tree;

pub(crate) use globals::GlobalState;
pub(crate) use shell::ShellState;
