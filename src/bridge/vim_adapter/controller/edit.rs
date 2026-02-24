//! Edit helper methods for VimController.
//!
//! Handles character typing, motion execution, undo/redo, and paste operations.

use crate::bridge::vim_adapter::core::cast::{i32_to_usize, usize_to_i32};
use crate::bridge::vim_adapter::core::cursor::CursorMoveType;
use crate::bridge::vim_adapter::handlers::edit::perform_paste;
use crate::bridge::vim_adapter::handlers::motion;
use crate::bridge::vim_wrapper::VimController;
use godot::classes::CodeEdit;
use godot::prelude::*;
use vim_core::domain::position::Position;
use vim_core::prelude::EditOp;
use vim_core::runtime::pure::{calculate_replace_edits, calculate_virtual_replace_edits};
use vim_core::inputs::commands::motions::Motion;
use vim_core::state::mode::{Mode, ReplaceMode};

impl VimController {
    /// Handles typing a single character in Insert or Replace mode.
    pub(crate) fn handle_type_char(&mut self, c: char) {
        let Some(mut editor) = self.get_editor() else {
            return;
        };

        let mode = self.engine.mode();
        let is_replace = matches!(mode, Mode::Replace(ReplaceMode::Overwrite) | Mode::Replace(ReplaceMode::Virtual));

        if is_replace {
            editor.begin_complex_operation();
        }

        // Update last change position for `. mark (once per character typed)
        let pos = Self::cursor_from_editor(&editor);
        self.engine.set_last_change(pos);
        let line = usize_to_i32(pos.line);
        let col = usize_to_i32(pos.col);

        // Handle newline separately (special indentation logic)
        if c == '\n' {
            let line_text = editor.get_line(line).to_string();
            let indent: String = line_text
                .chars()
                .take_while(|ch| ch.is_whitespace())
                .collect();

            if indent.is_empty() {
                editor.insert_text_at_caret("\n");
            } else {
                editor.insert_text_at_caret(&format!("\n{indent}"));
            }

            if is_replace {
                editor.end_complex_operation();
            }
            return;
        }

        if is_replace {
            let tab_size = self.editor_config.indent_size;
            // Fetch only the prefix of the line needed for the calculation.
            // VirtualReplace needs at least char_col + 1 to check the replaced character.
            // Adding tab_size provides enough context to handle tab-splitting without
            // allocating the entire line.
            let line_prefix = editor
                .get_line(line)
                .left(i64::from(col + usize_to_i32(tab_size)));
            let line_text = line_prefix.to_string();

            let edits = if matches!(mode, Mode::Replace(ReplaceMode::Virtual)) {
                calculate_virtual_replace_edits(
                    &line_text,
                    i32_to_usize(line),
                    i32_to_usize(col),
                    c,
                    tab_size,
                )
            } else {
                calculate_replace_edits(&line_text, i32_to_usize(line), i32_to_usize(col), c)
            };

            for op in edits {
                if let EditOp::Replace { start, end, text } = op {
                    editor.remove_text(
                        usize_to_i32(start.line),
                        usize_to_i32(start.col),
                        usize_to_i32(end.line),
                        usize_to_i32(end.col),
                    );
                    editor.insert_text_at_caret(&*text);
                }
            }
            editor.end_complex_operation();
        } else {
            // Standard Insert mode: insert the character directly.
            let mut buf = [0u8; 4];
            let s = c.encode_utf8(&mut buf);
            editor.insert_text_at_caret(&*s);
        }
    }

    /// Handles motion with count.
    pub(crate) fn handle_motion_with_count(&mut self, motion: Motion, count: usize) {
        self.handle_motion_dispatch(motion, count);
    }

    /// Dispatches motion to either pure logic (preferred) or imperative fallback.
    fn handle_motion_dispatch(&mut self, motion: Motion, count: usize) {
        // Motions that require viewport/UI (scroll effects)
        match motion {
            Motion::HalfPageDown
            | Motion::HalfPageUp
            | Motion::FullPageDown
            | Motion::FullPageUp
            | Motion::ScrollLineUp
            | Motion::ScrollLineDown
            | Motion::ScrollLeft
            | Motion::ScrollRight
            | Motion::ScrollHalfScreenLeft
            | Motion::ScrollHalfScreenRight
            | Motion::CenterCursor
            | Motion::TopCursor
            | Motion::BottomCursor => {
                // Scroll motions need viewport - inline handler
                if let Some(mut editor) = self.get_editor() {
                    motion::execute_scroll_motion(&mut editor, motion, count);
                    self.engine.sync_cursor(Self::cursor_from_editor(&editor));
                }
            }
            // Window positioning (H, M, L) - handled separately to consume count directly
            Motion::WindowTop | Motion::WindowMiddle | Motion::WindowBottom => {
                if let Some(mut editor) = self.get_editor() {
                    motion::execute_window_motion(&mut editor, motion, count);
                    self.engine.sync_cursor(Self::cursor_from_editor(&editor));
                }
            }
            // Screen-line motions (g0/g^/g$/gm/gk/gj) - require wrap info from Godot
            Motion::ScreenLineStart
            | Motion::ScreenLineFirstNonBlank
            | Motion::ScreenLineEnd
            | Motion::ScreenLineMiddle
            | Motion::ScreenLineUp
            | Motion::ScreenLineDown => {
                if let Some(mut editor) = self.get_editor() {
                    for _ in 0..count {
                        motion::execute_screen_line_motion(&mut editor, motion);
                    }
                    self.engine.sync_cursor(Self::cursor_from_editor(&editor));
                }
            }
            _ => {
                // Pure logic (Visual Mode safe)
                self.apply_pure_motion(motion, count);
            }
        }

        // Apply scroll offset (scrolloff) after cursor movement
        if let Some(mut editor) = self.get_editor() {
            Self::apply_scroll_offset(&mut editor);
        }

        self.update_visual_selection();
    }

    /// Applies a pure motion to the editor.
    fn apply_pure_motion(&mut self, motion: Motion, count: usize) {
        if let Some(mut editor) = self.get_editor() {
            // Use cached config instead of reconstructing config per motion.
            self.engine.apply_motion(&mut editor, motion, count);
        }
    }

    /// Handles append command (a/A).
    #[allow(
        clippy::unused_self,
        reason = "Logically belongs to VimController, called as self.handle_append()"
    )]
    pub(crate) fn handle_append(&self, editor: &mut Gd<CodeEdit>, at_eol: bool) {
        if at_eol {
            let line = editor.get_caret_line();
            // GString.len() returns character count directly - no String allocation needed
            let line_len = editor.get_line(line).len();
            editor.set_caret_column(usize_to_i32(line_len));
        } else {
            let col = editor.get_caret_column();
            editor.set_caret_column(col + 1);
        }
    }

    /// Handles insert at first non-blank (I command).
    #[allow(
        clippy::unused_self,
        reason = "Logically belongs to VimController, called as self.handle_insert_first_nonblank()"
    )]
    pub(crate) fn handle_insert_first_nonblank(&mut self, editor: &mut Gd<CodeEdit>) {
        let line = editor.get_caret_line(); // i32
        let line_text = editor.get_line(line).to_string();
        let first_nonblank = line_text.find(|c: char| !c.is_whitespace()).unwrap_or(0);

        let target = Position::new(i32_to_usize(line), first_nonblank);
        self.engine
            .move_cursor_tracked(target, CursorMoveType::Step);
        editor
            .set_caret_line_ex(usize_to_i32(target.line))
            .can_be_hidden(false)
            .done();
        editor.set_caret_column(usize_to_i32(target.col));
    }

    /// Handles insert at last position (gi command).
    pub(crate) fn handle_insert_at_last_position(&mut self, editor: &mut Gd<CodeEdit>) {
        match self.engine.last_insert_pos() {
            Some(pos) => {
                // Clamp to valid buffer bounds before moving.
                let line_count = i32_to_usize(editor.get_line_count());
                let line = pos.line.min(line_count.saturating_sub(1));
                let line_len = editor.get_line(usize_to_i32(line)).len();
                let col = pos.col.min(line_len);

                let target = Position::new(line, col);
                self.engine
                    .move_cursor_tracked(target, CursorMoveType::Jump);
                editor
                    .set_caret_line_ex(usize_to_i32(target.line))
                    .can_be_hidden(false)
                    .done();
                editor.set_caret_column(usize_to_i32(target.col));
                log::debug!("gi: jumped to last insert position line={} col={}", line, col);
            }
            None => {
                log::debug!("gi: No last insert position recorded");
            }
        }
    }

    /// Handles undo command.
    pub(crate) fn handle_undo(&mut self, count: usize) {
        if let Some(mut editor) = self.get_editor() {
            for _ in 0..count {
                editor.undo();
            }
            // Godot undo can restore selection state; clear it for Vim consistency
            editor.remove_secondary_carets();
            editor.deselect();
            log::debug!("Undo count={}", count);
        }
    }

    /// Handles redo command.
    pub(crate) fn handle_redo(&mut self, count: usize) {
        if let Some(mut editor) = self.get_editor() {
            for _ in 0..count {
                editor.redo();
            }
            log::debug!("Redo count={}", count);
        }
    }

    /// Handles paste command.
    pub(crate) fn handle_paste(
        &mut self,
        after: bool,
        register: Option<char>,
        count: usize,
        adjust_indent: bool,
        move_cursor_to_end: bool,
    ) {
        if let Some(mut editor) = self.get_editor() {
            perform_paste(
                &mut editor,
                after,
                register,
                count,
                adjust_indent,
                move_cursor_to_end,
                &self.engine,
            );
        }
    }
}
