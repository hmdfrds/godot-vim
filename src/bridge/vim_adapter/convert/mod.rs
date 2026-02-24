//! Type conversions between godot-vim types and vim-core types.
//!
//! Every function here converts between the two type systems.
//! This is the single location where both type systems coexist.
//!
//! # Zero-Cost Design
//!
//! All conversions are `#[inline]` and most are simple field copies.
//! The compiler eliminates them entirely in release builds.

mod command;
mod external;
mod key;
mod mode;
mod position;

use crate::bridge::types::command::EditorCommand;
use crate::bridge::types::cursor::CursorPos;
use crate::bridge::types::key::KeyEvent;
use crate::bridge::types::mode::EditorMode;
use vim_core::domain::position::Position;
use vim_core::inputs::keys::VimKey;
use vim_core::prelude::ShellRequest;
use vim_core::state::mode::Mode;

#[inline]
#[must_use]
pub fn mode_to_editor_mode(mode: &Mode) -> EditorMode {
    mode::mode_to_editor_mode(mode)
}

#[inline]
#[must_use]
pub fn key_event_to_vim_key(key: &KeyEvent) -> VimKey {
    key::key_event_to_vim_key(key)
}

#[inline]
#[must_use]
pub fn cursor_to_position(cursor: &CursorPos) -> Position {
    position::cursor_to_position(cursor)
}

#[must_use]
pub fn shell_request_to_command(req: &ShellRequest) -> EditorCommand {
    command::shell_request_to_command(req)
}
