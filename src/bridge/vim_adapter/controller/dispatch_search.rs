//! Search command dispatchers for VimController.
//!
//! Handles Search and FindAndReplace commands that use Godot's
//! `editor.search()` API. Extracted from the main dispatch module
//! to keep each file focused on a single concern.

use crate::bridge::vim_adapter::core::cast::{i32_to_usize, usize_to_i32};
use crate::bridge::vim_adapter::core::column_codec;
use crate::bridge::vim_wrapper::VimController;
use godot::prelude::*;
use vim_core::domain::position::Position;

impl VimController {
    /// Execute an inline search and move cursor to the match.
    pub(crate) fn dispatch_search(&mut self, pattern: String, forward: bool) {
        let Some(mut editor) = self.get_editor() else {
            return;
        };

        let cursor_pos = Self::cursor_from_editor(&editor);
        let cursor = usize_to_i32(column_codec::byte_to_editor_col_in_editor(
            &editor,
            cursor_pos.line,
            cursor_pos.col.as_usize(),
        ));
        let line = usize_to_i32(cursor_pos.line);

        let mut flags = godot::classes::text_edit::SearchFlags::MATCH_CASE;
        if !forward {
            flags |= godot::classes::text_edit::SearchFlags::BACKWARDS;
        }

        let result = editor.search(&GString::from(&pattern), flags, line, cursor);
        if result.x >= 0 {
            let result_line = i32_to_usize(result.y);
            let result_col = column_codec::editor_col_to_byte_in_editor(
                &editor,
                result_line,
                i32_to_usize(result.x),
            );
            let target = Position::from_byte(result_line, result_col);
            column_codec::apply_core_position_to_editor(&mut editor, target);
            self.engine.sync_cursor(target);
        } else {
            log::debug!("Search pattern not found: {}", pattern);
        }
    }

    /// Execute a find-and-replace operation across the document.
    #[expect(
        clippy::cast_possible_wrap,
        reason = "pattern/replacement lengths always fit i32"
    )]
    pub(crate) fn dispatch_find_and_replace(
        &mut self,
        pattern: String,
        replacement: String,
        flags: String,
    ) {
        let Some(mut editor) = self.get_editor() else {
            return;
        };

        const MAX_REPLACEMENTS: usize = 100_000;
        let mut current_line = 0;
        let mut current_col = 0;
        let mut count: usize = 0;

        let search_flags = godot::classes::text_edit::SearchFlags::MATCH_CASE;
        let pattern_gstr = GString::from(&pattern);
        let replacement_gstr = GString::from(&replacement);

        let pattern_char_len = pattern.chars().count() as i32;
        let replacement_char_len = replacement.chars().count() as i32;

        loop {
            let result = editor.search(&pattern_gstr, search_flags, current_line, current_col);
            if result.x < 0 {
                break;
            }

            editor.set_caret_line(result.y);
            editor.set_caret_column(result.x);

            let end_col = result.x + pattern_char_len;
            editor.select(result.y, result.x, result.y, end_col);
            editor.insert_text_at_caret(&replacement_gstr);

            current_line = result.y;
            current_col = result.x + replacement_char_len;
            count += 1;

            if !flags.contains('g') {
                break;
            }
            if count >= MAX_REPLACEMENTS {
                log::warn!(
                    "Find/replace safety limit reached ({} replacements)",
                    MAX_REPLACEMENTS
                );
                break;
            }
        }
        log::info!("Replaced {} occurrences of '{}'", count, pattern);
    }
}
