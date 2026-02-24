//! Search operations: SearchNext, SearchPrev, SearchWord*.

use crate::bridge::vim_adapter::core::cast::{i32_to_usize, usize_to_i32};
use crate::bridge::vim_adapter::core::cursor::CursorMoveType;
use crate::bridge::vim_wrapper::VimController;
use crate::bridge::vim_wrapper_util::extract_word_at_col;
use godot::classes::text_edit::SearchFlags;
use godot::classes::CodeEdit;
use godot::prelude::*;
use vim_core::domain::position::Position;

impl VimController {
    pub(super) fn handle_search_repeat(&mut self, editor: &mut Gd<CodeEdit>, is_next: bool) {
        let Some(pattern) = self.engine.last_search().map(|s| s.to_string()) else {
            log::debug!("No previous search pattern");
            return;
        };

        let forward = is_next == self.engine.last_search_forward();

        log::debug!(
            "Search repeat cmd={} last_forward={} effective_forward={}",
            if is_next { "n" } else { "N" },
            self.engine.last_search_forward(),
            forward
        );

        if let Some(pos) = perform_search_and_locate(editor, &pattern, forward) {
            self.engine.move_cursor_tracked(pos, CursorMoveType::Jump);
            editor
                .set_caret_line_ex(usize_to_i32(pos.line))
                .can_be_hidden(false)
                .done();
            editor.set_caret_column(usize_to_i32(pos.col));
            log::debug!("Search found pattern={} forward={}", pattern, forward);
        } else {
            log::debug!("Pattern '{}' not found", pattern);
        }
    }

    pub(super) fn handle_search_word(&mut self, editor: &mut Gd<CodeEdit>, forward: bool) {
        let line = editor.get_caret_line();
        let col = editor.get_caret_column();
        let line_text = editor.get_line(line).to_string();

        let Some(word) = extract_word_at_col(&line_text, i32_to_usize(col)) else {
            return;
        };

        // Store the search pattern
        self.engine.set_search(word.clone(), forward);

        // Enable visual highlighting
        editor.set_search_flags(SearchFlags::MATCH_CASE);
        editor.set_search_text(&GString::from(&word));
        editor.queue_redraw();

        if let Some(pos) = perform_search_and_locate(editor, &word, forward) {
            self.engine.move_cursor_tracked(pos, CursorMoveType::Jump);
            editor
                .set_caret_line_ex(usize_to_i32(pos.line))
                .can_be_hidden(false)
                .done();
            editor.set_caret_column(usize_to_i32(pos.col));
            log::debug!("Word search found word={} forward={}", word, forward);
        } else {
            log::debug!("Word search: '{}' not found", word);
        }
    }
}

/// Performs a search in the editor and returns the match position.
///
/// Godot's `search()` returns `Point2i(column, line)` - coordinates are swapped.
pub fn perform_search_and_locate(
    editor: &mut Gd<CodeEdit>,
    pattern: &str,
    forward: bool,
) -> Option<Position> {
    let line = editor.get_caret_line();
    let col = editor.get_caret_column();

    let line_len = usize_to_i32(editor.get_line(line).len());
    let clamped_col = col.min(line_len);

    let search_col = if forward {
        clamped_col + 1
    } else if clamped_col > 0 {
        clamped_col - 1
    } else {
        clamped_col
    };

    let flags = if forward {
        SearchFlags::MATCH_CASE
    } else {
        SearchFlags::MATCH_CASE | SearchFlags::BACKWARDS
    };

    let result = editor.search(&GString::from(pattern), flags, line, search_col);

    if result.x >= 0 && result.y >= 0 {
        if let Some((line, col)) = validate_search_position(editor, result.y, result.x) {
            return Some(Position::new(i32_to_usize(line), i32_to_usize(col)));
        }
    }

    // Wrap around
    let (wrap_line, wrap_col) = if forward {
        (0, 0)
    } else {
        let last_line = editor.get_line_count() - 1;
        let last_col = usize_to_i32(editor.get_line(last_line).len());
        (last_line, last_col)
    };

    let wrap_result = editor.search(&GString::from(pattern), flags, wrap_line, wrap_col);
    if wrap_result.x >= 0 && wrap_result.y >= 0 {
        if let Some((line, col)) = validate_search_position(editor, wrap_result.y, wrap_result.x) {
            log::debug!("Found pattern={} (wrapped)", pattern);
            return Some(Position::new(i32_to_usize(line), i32_to_usize(col)));
        }
    }

    None
}

fn validate_search_position(editor: &Gd<CodeEdit>, line: i32, col: i32) -> Option<(i32, i32)> {
    let line_count = editor.get_line_count();
    if line < 0 || line >= line_count {
        return None;
    }
    let line_len = usize_to_i32(editor.get_line(line).len());
    Some((line, col.clamp(0, line_len)))
}
