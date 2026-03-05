use crate::bridge::vim_adapter::core::cast::{i32_to_usize, usize_to_i32};
use crate::bridge::vim_adapter::core::column_codec;
use crate::bridge::vim_wrapper::VimController;
use godot::classes::text_edit::SearchFlags;
use godot::classes::CodeEdit;
use godot::prelude::*;

impl VimController {
    /// Find the search target position without moving the cursor.
    ///
    /// Returns `Some((line, col))` if found, `None` if not found.
    /// Also stores the pattern for later use with `n`/`N`.
    ///
    /// # Note
    /// Godot's `search()` returns `Point2i(column, line)` - coordinates are swapped
    /// compared to typical (line, col) convention. This function handles that.
    pub(crate) fn find_search_target(
        &mut self,
        pattern: &str,
        forward: bool,
    ) -> Option<(i32, i32)> {
        self.engine.set_search(pattern.to_string(), forward);

        let editor = self.get_editor()?;

        let line = editor.get_caret_line();
        let col = editor.get_caret_column();

        let flags = if forward {
            SearchFlags::MATCH_CASE
        } else {
            SearchFlags::MATCH_CASE | SearchFlags::BACKWARDS
        };

        // First search from current position
        let result = editor.search(&GString::from(pattern), flags, line, col);

        // Godot returns Point2i(column, line), not (line, column).
        if result.x >= 0 && result.y >= 0 {
            let (found_line, found_col) = (result.y, result.x);
            if let Some(validated) = Self::validate_position(&editor, found_line, found_col) {
                log::debug!(
                    "Found pattern={} line={} col={}",
                    pattern,
                    validated.0,
                    validated.1
                );
                return Some(validated);
            }
        }

        // Wrap around: search from beginning (forward) or end (backward)
        let (wrap_line, wrap_col) = if forward {
            (0, 0)
        } else {
            let last_line = column_codec::last_line_index(&editor)?;
            let last_col = usize_to_i32(column_codec::editor_line_char_len(
                &editor,
                i32_to_usize(last_line),
            ));
            (last_line, last_col)
        };

        let wrap_result = editor.search(&GString::from(pattern), flags, wrap_line, wrap_col);
        if wrap_result.x >= 0 && wrap_result.y >= 0 {
            let (found_line, found_col) = (wrap_result.y, wrap_result.x);
            if let Some(validated) = Self::validate_position(&editor, found_line, found_col) {
                log::debug!(
                    "Found pattern={} line={} col={} (wrapped)",
                    pattern,
                    validated.0,
                    validated.1
                );
                return Some(validated);
            }
        }

        log::debug!("Pattern '{}' not found", pattern);
        None
    }

    /// Validate and clamp position to valid bounds.
    ///
    /// Returns `Some((line, col))` with clamped column, or `None` if line is invalid.
    fn validate_position(editor: &Gd<CodeEdit>, line: i32, col: i32) -> Option<(i32, i32)> {
        let line_count = editor.get_line_count();
        if line < 0 || line >= line_count {
            return None;
        }

        let line_len = usize_to_i32(column_codec::editor_line_char_len(
            editor,
            i32_to_usize(line),
        ));
        // Clamp column to valid range [0, line_length]
        let col = col.clamp(0, line_len);
        Some((line, col))
    }
}
