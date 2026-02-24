use crate::bridge::types::cursor::CursorPos;
use vim_core::domain::position::Position;

/// Convert vim-core `Position` to shell `CursorPos`.
#[inline]
#[must_use]
pub fn position_to_cursor(pos: &Position) -> CursorPos {
    CursorPos::new(pos.line, pos.col)
}

/// Convert shell `CursorPos` to vim-core `Position`.
#[inline]
#[must_use]
pub fn cursor_to_position(cursor: &CursorPos) -> Position {
    Position::new(cursor.line, cursor.col)
}
