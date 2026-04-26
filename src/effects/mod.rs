//! Effect dispatch — applies vim-core engine effects to Godot's CodeEdit.
//!
//! When the Vim engine processes a keystroke it produces a list of effects
//! (cursor moves, text edits, mode changes, scroll commands, etc.). This
//! module dispatches each effect to the appropriate CodeEdit operation,
//! handling text mutations, cursor positioning, selection updates, scroll
//! adjustments, undo grouping, register syncing, and search highlighting.

pub(crate) mod auto_brace;
mod compound;
pub(crate) mod cursor;
pub(crate) mod dispatch;
pub(crate) mod messages;
pub(crate) mod mode;
pub(crate) mod navigation;
mod registers;
pub(crate) mod scroll;
pub(crate) mod search;
pub(crate) mod text;
pub(crate) mod undo;

pub(crate) use compound::{CompoundAction, LineNumber, WindowNavAction};
pub(crate) use dispatch::{dispatch, DispatchContext};
pub(crate) use undo::UndoDepth;
