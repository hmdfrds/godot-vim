//! Bridge between Godot's CodeEdit and the vim-core engine.
//!
//! Translates between two coordinate systems: Godot uses (line, column) pairs
//! measured in Unicode scalar values, while vim-core uses UTF-8 byte offsets.
//! The `codec` submodule owns the `LineIndex` that makes this conversion O(log n).
//!
//! Submodule responsibilities:
//! - `codec` — UTF-8 byte offset <-> (line, char-column) conversion, `LineIndex`
//! - `context` — builds an `InputContext` snapshot from live CodeEdit state
//! - `document` — `GodotDocument`: vim-core `Document` trait over `&str`
//! - `input` — Godot `InputEventKey` -> vim-core `KeyEvent` translation
//! - `port` / `port_impl` — `TextEditorPort` trait and CodeEdit implementation
//! - `code_edit_ext` — fold-aware cursor movement extension trait

pub(crate) mod code_edit_ext;
pub(crate) mod codec;
pub(crate) mod context;
// TODO(task-2): remove `allow` once consumers are wired up.
#[allow(dead_code)]
pub(crate) mod godot_calls;
pub(crate) mod document;
pub(crate) mod input;
pub(crate) mod port;
pub(crate) mod port_impl;

pub(crate) use port_impl::{AutoBraceSnapshot, SyntaxRegion};
