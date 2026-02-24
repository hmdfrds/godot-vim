//! Marks handler trait for `VimController`.
//!
//! Handles setting marks and jumping to marks/positions.

use crate::bridge::vim_adapter::core::cast::{i32_to_usize, usize_to_i32};
use crate::bridge::vim_adapter::core::cursor::CursorMoveType;
use crate::bridge::vim_wrapper::VimController;
use vim_core::domain::position::Position;

/// Trait for handling marks and jump operations.
pub trait MarksHandler {
    /// Set a mark at the current cursor position.
    fn handle_set_mark(&mut self, c: char);

    /// Jump to a mark by name.
    fn handle_jump_to_mark(&mut self, name: char, exact: bool);

    /// Jump to a specific position.
    fn handle_jump_to(&mut self, pos: Position);
}

impl MarksHandler for VimController {
    fn handle_set_mark(&mut self, c: char) {
        if let Some(editor) = self.get_editor() {
            let pos = Position::new(
                i32_to_usize(editor.get_caret_line()),
                i32_to_usize(editor.get_caret_column()),
            );
            self.engine.set_mark(c, pos);
            log::debug!("Mark set name='{c}' line={} col={}", pos.line, pos.col);
        }
    }

    fn handle_jump_to_mark(&mut self, name: char, exact: bool) {
        if let Some(mut editor) = self.get_editor() {
            // Resolve special marks first, then fall back to named marks
            let pos = match name {
                // `` - position before last jump
                '`' => self.engine.last_jump_pos(),
                // `. - position of last change
                '.' => self.engine.last_change_pos(),
                // `^ - position of last insert
                '^' => self.engine.last_insert_pos(),
                // `< - start of visual area
                '<' => self.engine.last_visual_selection().map(|vs| vs.start),
                // `> - end of visual area
                '>' => self.engine.last_visual_selection().map(|vs| vs.end),
                // Named marks (a-z, A-Z)
                _ => self.engine.get_mark(name),
            };

            if let Some(mut pos) = pos {
                if !exact {
                    // For ' (apostrophe) - jump to first non-blank of line
                    let first_nonblank =
                        editor.get_first_non_whitespace_column(usize_to_i32(pos.line));
                    pos.col = i32_to_usize(first_nonblank);
                }

                self.engine.move_cursor_tracked(pos, CursorMoveType::Jump);
                editor
                    .set_caret_line_ex(usize_to_i32(pos.line))
                    .can_be_hidden(false)
                    .done();
                editor.set_caret_column(usize_to_i32(pos.col));
                log::debug!("Jumped to mark '{name}'");
            } else {
                log::debug!("Mark '{name}' not set");
            }
        }
    }

    fn handle_jump_to(&mut self, pos: Position) {
        if let Some(mut editor) = self.get_editor() {
            // JumpRestoration: callers (jump_back/jump_forward) manage the jumplist stack
            // themselves, so the cursor moves without pushing a new entry.
            self.engine
                .move_cursor_tracked(pos, CursorMoveType::JumpRestoration);
            editor
                .set_caret_line_ex(usize_to_i32(pos.line))
                .can_be_hidden(false)
                .done();
            editor.set_caret_column(usize_to_i32(pos.col));
        }
    }
}
