//! Multi-cursor support: bidirectional sync between vim-core's
//! MultiCursorState and Godot's multi-caret TextEdit API.

pub(crate) mod keybindings;
pub(crate) mod sync;
