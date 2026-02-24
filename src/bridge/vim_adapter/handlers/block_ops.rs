//! Block operations handler trait for `VimController`.
//!
//! Handles visual block insert/append operations.
//! Block insert/append use pure functions from `edits::block`.
//! Only transaction application and multi-caret cleanup are handled here.

use crate::bridge::vim_adapter::core::cast::usize_to_i32;
use crate::bridge::vim_wrapper::VimController;
use godot::classes::CodeEdit;
use godot::prelude::*;
use vim_core::domain::position::Position;
use vim_core::state::mode::{InsertMode, Mode};

/// Trait for handling visual block insert/append operations.
pub trait BlockOpsHandler {
    /// Handle 'I' in visual block mode - insert at block selection start.
    fn handle_begin_block_insert(&mut self, lines: (usize, usize), col: usize, origin: Position);

    /// Handle 'A' in visual block mode - append after block selection end.
    fn handle_begin_block_append(
        &mut self,
        lines: (usize, usize),
        end_col: usize,
        origin: Position,
    );

    /// Finish block insert by applying text to all lines.
    fn handle_finish_block_insert(
        &self,
        editor: &mut Gd<CodeEdit>,
        lines: (usize, usize),
        col: usize,
        text: &str,
        origin: Position,
    );

    /// Finish block append by applying text at specific column on all lines.
    fn handle_finish_block_append(
        &self,
        editor: &mut Gd<CodeEdit>,
        lines: (usize, usize),
        col: usize,
        text: &str,
        origin: Position,
    );

    /// Update realtime preview during block insert/append.
    fn handle_block_insert_preview(
        &self,
        editor: &mut Gd<CodeEdit>,
        lines: (usize, usize),
        col: usize,
        text: &str,
    );

    /// Handle backspace during block insert/append.
    fn handle_block_insert_backspace(
        &self,
        editor: &mut Gd<CodeEdit>,
        lines: (usize, usize),
        col: usize,
        text: &str,
    );
}

impl BlockOpsHandler for VimController {
    fn handle_begin_block_insert(&mut self, lines: (usize, usize), col: usize, origin: Position) {
        // Use the parameters from the BeginBlockInsert shell request.
        // The mode has already been set to BlockInsert by the processor,
        // so checking for VisualBlock here is no longer valid.
        if let Some(mut editor) = self.get_editor() {
            let (min_line, max_line) = lines;

            log::debug!(
                "Begin block insert lines=({}, {}) col={}",
                min_line,
                max_line,
                col
            );

            // Remove secondary carets before inserting; TypeChar would otherwise
            // insert at every caret position.
            editor.remove_secondary_carets();
            editor.deselect();

            // Position cursor at top-left of block for insert
            editor.set_caret_line(usize_to_i32(min_line));
            editor.set_caret_column(usize_to_i32(col));

            // The mode was already set to BlockInsert by the processor; synchronise here.
            self.engine.set_mode(Mode::Insert(InsertMode::BlockInsert {
                lines: (min_line, max_line),
                col,
                origin,
            }));
        }
    }

    fn handle_begin_block_append(
        &mut self,
        lines: (usize, usize),
        end_col: usize,
        origin: Position,
    ) {
        if let Some(mut editor) = self.get_editor() {
            let (min_line, max_line) = lines;

            log::debug!(
                "Begin block append lines=({}, {}) end_col={}",
                min_line,
                max_line,
                end_col
            );

            // Remove secondary carets before inserting.
            editor.remove_secondary_carets();
            editor.deselect();

            // Position the cursor at the top line, after the block selection end (append).
            editor.set_caret_line(usize_to_i32(min_line));
            // Position at the append column (already calculated as selection_end + 1)
            editor.set_caret_column(usize_to_i32(end_col));

            // Set mode to BlockAppend so Esc triggers FinishBlockAppend (not FinishBlockInsert)
            self.engine.set_mode(Mode::Insert(InsertMode::BlockAppend {
                lines: (min_line, max_line),
                col: end_col,
                origin,
            }));
        }
    }

    fn handle_finish_block_insert(
        &self,
        editor: &mut Gd<CodeEdit>,
        lines: (usize, usize),
        col: usize,
        text: &str,
        origin: Position,
    ) {
        // Text has already been inserted via realtime preview; remove secondary carets and reposition.
        log::debug!(
            "Finish block insert lines={:?} col={} text='{}'",
            lines,
            col,
            text
        );

        // Clean up any secondary carets from visual block
        editor.remove_secondary_carets();
        editor.set_caret_blink_enabled(true);

        // Position cursor at top-left of block (origin), matching Vim behavior
        editor.set_caret_line(usize_to_i32(origin.line));
        editor.set_caret_column(usize_to_i32(origin.col));
    }

    fn handle_finish_block_append(
        &self,
        editor: &mut Gd<CodeEdit>,
        lines: (usize, usize),
        col: usize,
        text: &str,
        origin: Position,
    ) {
        // Text has already been inserted via realtime preview; remove secondary carets and reposition.
        log::debug!(
            "Finish block append lines={:?} col={} text='{}'",
            lines,
            col,
            text
        );

        // Clean up any secondary carets from visual block
        editor.remove_secondary_carets();
        editor.set_caret_blink_enabled(true);

        // Position cursor at top-left of block (origin), matching Vim behavior
        editor.set_caret_line(usize_to_i32(origin.line));
        editor.set_caret_column(usize_to_i32(origin.col));
    }

    fn handle_block_insert_preview(
        &self,
        editor: &mut Gd<CodeEdit>,
        lines: (usize, usize),
        col: usize,
        text: &str,
    ) {
        let (start_line, end_line) = lines;
        let Some(new_char) = text.chars().last() else {
            return;
        };

        log::trace!(
            "Block insert preview lines=({}, {}) col={} char='{}'",
            start_line,
            end_line,
            col,
            new_char
        );

        editor.begin_complex_operation();

        // Calculate insert column (current position after previous chars)
        // For first char: insert at col
        // For subsequent: insert at col + (char_count - 1)
        // Use char count for correct positioning with Unicode
        let text_char_count = text.chars().count();
        let insert_col = col + text_char_count - 1;

        // First, add secondary carets on all lines except primary
        for line_idx in (start_line + 1)..=end_line {
            let line_len = editor.get_line(usize_to_i32(line_idx)).len();
            let actual_col = insert_col.min(line_len);
            editor.add_caret(usize_to_i32(line_idx), usize_to_i32(actual_col));
        }

        // Position primary caret
        let primary_line_len = editor.get_line(usize_to_i32(start_line)).len();
        let primary_col = insert_col.min(primary_line_len);
        editor.set_caret_line(usize_to_i32(start_line));
        editor.set_caret_column(usize_to_i32(primary_col));

        // Insert at the primary caret; Godot replicates the insertion at every active caret.
        let mut buf = [0u8; 4];
        let s: &str = new_char.encode_utf8(&mut buf);
        editor.insert_text_at_caret(s);

        // Remove secondary carets - they served their purpose
        editor.remove_secondary_carets();

        // Keep primary caret at new position (after inserted char)
        editor.set_caret_line(usize_to_i32(start_line));
        // Use char count for correct cursor positioning with Unicode
        editor.set_caret_column(usize_to_i32(col + text.chars().count()));

        editor.end_complex_operation();
    }

    fn handle_block_insert_backspace(
        &self,
        editor: &mut Gd<CodeEdit>,
        lines: (usize, usize),
        col: usize,
        text: &str,
    ) {
        let (start_line, end_line) = lines;

        log::trace!(
            "Block insert backspace lines=({}, {}) col={} remaining='{}'",
            start_line,
            end_line,
            col,
            text
        );

        editor.begin_complex_operation();

        // Compute the deletion column from the char count of the remaining text prefix
        // (Unicode-aware), accounting for the character that was removed at that position.
        let text_char_count = text.chars().count();
        let delete_col = col + text_char_count;

        // Add secondary carets on all lines except primary
        for line_idx in (start_line + 1)..=end_line {
            let line_len = editor.get_line(usize_to_i32(line_idx)).len();
            if delete_col < line_len {
                // Position caret after the char to delete
                editor.add_caret(usize_to_i32(line_idx), usize_to_i32(delete_col + 1));
            }
        }

        // Position primary caret after the char to delete
        let primary_line_len = editor.get_line(usize_to_i32(start_line)).len();
        if delete_col < primary_line_len {
            editor.set_caret_line(usize_to_i32(start_line));
            editor.set_caret_column(usize_to_i32(delete_col + 1));

            // Backspace at all carets
            editor.backspace();
        }

        // Remove secondary carets
        editor.remove_secondary_carets();

        // Position cursor correctly
        editor.set_caret_line(usize_to_i32(start_line));
        // Use char count for correct cursor positioning with Unicode
        editor.set_caret_column(usize_to_i32(col + text_char_count));

        editor.end_complex_operation();
    }
}
