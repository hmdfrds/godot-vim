//! Dot repeat and insert-exit repetition for VimController.
//!
//! Handles the `.` (dot repeat) command and insert-mode exit with count repetition.

use crate::bridge::vim_adapter::core::cast::{i32_to_usize, usize_to_i32};
use crate::bridge::vim_adapter::core::column_codec;
use crate::bridge::vim_adapter::handlers::mode::ModeHandler;
use crate::bridge::vim_wrapper::VimController;
use godot::classes::CodeEdit;
use godot::prelude::*;
use vim_core::runtime::pure::{compute_repeat_insert, RepeatInsertAction};
use vim_core::state::mode::{Mode, InsertMode};

impl VimController {
    /// Handles exiting Insert mode with repetition count.
    ///
    /// Performs repetition if count > 1, ensures cursor position is correct (move back 1),
    /// and triggers mode change to Normal.
    pub(crate) fn handle_exit_insert_mode(&mut self, text: String, count: usize) {
        if let Some(mut editor) = self.get_editor() {
            if count > 1 && !text.is_empty() {
                editor.begin_complex_operation();
                // Repeat insertion count-1 times (first time was handled during typing)
                let repeat_count = count - 1;
                for _ in 0..repeat_count {
                    editor.insert_text_at_caret(&text);
                }
                editor.end_complex_operation();
            }

            // Standard Vim behavior: move cursor back one char when exiting insert
            // Only if not at start of line
            let col = editor.get_caret_column();
            if col > 0 {
                editor.set_caret_column(col - 1);
            }

            // Keep vim_state in sync so sync_cursor_to_editor does not overwrite this position.
            self.engine.sync_cursor(Self::cursor_from_editor(&editor));
        }

        // Pass the previous Insert mode so handle_mode_change saves the last insert position.
        self.handle_mode_change(Mode::Normal, Some(Mode::Insert(InsertMode::Standard { count })));
    }

    /// Handles Dot Repeat (.) command via the canonical action pipeline.
    ///
    /// Insert repeats are applied directly via deterministic pure helper output.
    pub(crate) fn handle_repeat(&mut self, repeat_count: usize) {
        let Some(mut editor) = self.get_editor() else {
            return;
        };

        let Some(change) = self.engine.last_change().cloned() else {
            log::debug!("Repeat: No last change to repeat");
            return;
        };

        log::debug!("Repeat: executing change={:?} count={}", change, repeat_count);

        match &change {
            vim_core::state::global::repeat::RepeatableChange::Insert { text, count, entry } => {
                self.handle_repeat_insert(&mut editor, text, *count, repeat_count, entry);
            }
            vim_core::state::global::repeat::RepeatableChange::Operation {
                operator,
                motion,
                count,
                ..
            } => {
                drop(editor);
                let effective_count = if repeat_count > 1 {
                    repeat_count
                } else {
                    *count
                };

                for digit in effective_count.to_string().chars() {
                    self.engine.accumulate_digit(digit);
                }

                let op = *operator;
                let mot = *motion;
                crate::bridge::safety::guard(
                    || {
                        self.execute_action_with_visuals(
                            vim_core::inputs::commands::Action::Operator {
                                op,
                                motion: Some(mot),
                            },
                            vim_core::state::mode::Mode::Normal,
                        );
                    },
                    (),
                );
                if let Some(editor) = self.get_editor() {
                    self.engine.sync_cursor(Self::cursor_from_editor(&editor));
                }
                self.engine.set_mode(vim_core::state::mode::Mode::Normal);
            }
        }
    }

    /// Handles insert-type dot-repeat directly via Godot API.
    ///
    /// Thin shell adapter: delegates insertion-point computation to
    /// `vim_core::compute_repeat_insert`, then applies via Godot API.
    fn handle_repeat_insert(
        &mut self,
        editor: &mut Gd<CodeEdit>,
        text: &str,
        original_count: usize,
        repeat_count: usize,
        entry: &vim_core::state::global::repeat::InsertEntry,
    ) {
        let line = editor.get_caret_line();
        let cursor_col = column_codec::editor_col_to_byte_in_editor(
            editor,
            i32_to_usize(line),
            i32_to_usize(editor.get_caret_column()),
        );
        let line_text = editor.get_line(line).to_string();

        let action = compute_repeat_insert(
            entry,
            cursor_col,
            &line_text,
            text,
            original_count,
            repeat_count,
        );

        if matches!(action, RepeatInsertAction::Noop) {
            return;
        }

        editor.begin_complex_operation();

        match action {
            RepeatInsertAction::InsertAt { col, text } => {
                let editor_col =
                    column_codec::byte_to_editor_col_in_editor(editor, i32_to_usize(line), col);
                editor.set_caret_column(usize_to_i32(editor_col));
                editor.insert_text_at_caret(&text);
            }
            RepeatInsertAction::InsertEOL { text } => {
                let line_len = editor.get_line(line).len();
                editor.set_caret_column(usize_to_i32(line_len));
                editor.insert_text_at_caret(&text);
            }
            RepeatInsertAction::OpenBelow { text } => {
                let line_len = editor.get_line(line).len();
                editor.set_caret_column(usize_to_i32(line_len));
                let mut insert = String::with_capacity(1 + text.len());
                insert.push('\n');
                insert.push_str(&text);
                editor.insert_text_at_caret(&insert);
            }
            RepeatInsertAction::OpenAbove { text } => {
                editor.set_caret_line(line);
                editor.set_caret_column(0);
                let mut insert = String::with_capacity(text.len() + 1);
                insert.push_str(&text);
                insert.push('\n');
                editor.insert_text_at_caret(&insert);
            }
            RepeatInsertAction::Replace {
                overwrite_count,
                text,
            } => {
                let col = editor.get_caret_column();
                if overwrite_count > 0 {
                    editor.remove_text(line, col, line, col + usize_to_i32(overwrite_count));
                }
                editor.insert_text_at_caret(&text);
            }
            RepeatInsertAction::Noop => {} // Already handled above
        }

        // Move cursor back one char (standard Vim behavior on exiting insert)
        let col = editor.get_caret_column();
        if col > 0 {
            editor.set_caret_column(col - 1);
        }

        editor.end_complex_operation();

        self.engine.sync_cursor(Self::cursor_from_editor(editor));
        self.engine.set_mode(vim_core::state::mode::Mode::Normal);
    }
}
