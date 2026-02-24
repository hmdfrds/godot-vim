use crate::bridge::vim_adapter::core::cast::{i32_to_usize, usize_to_i32};
use godot::classes::text_edit::SearchFlags;
use godot::classes::CodeEdit;
use godot::prelude::*;
use vim_core::domain::position::Position;
use vim_core::domain::selection::Selection;
use vim_core::runtime::pure as pure_motion;
use vim_core::inputs::commands::motions::Motion;
use vim_core::state::VimState;

/// Godot adapter for `SearchProvider` — wraps a `&Gd<CodeEdit>` for regex search.
#[allow(dead_code)]
struct GodotSearchProvider<'a> {
    editor: &'a Gd<CodeEdit>,
}

impl vim_core::domain::search_provider::SearchProvider for GodotSearchProvider<'_> {
    fn find_match(
        &self,
        pattern: &str,
        from: Position,
        forward: bool,
        wrap: bool,
    ) -> Option<(Position, Position)> {
        let flags = if forward {
            SearchFlags::MATCH_CASE
        } else {
            SearchFlags::MATCH_CASE | SearchFlags::BACKWARDS
        };

        let mut result = self.editor.search(
            &GString::from(pattern),
            flags,
            usize_to_i32(from.line),
            usize_to_i32(from.col),
        );

        // Wrap around if not found and wrapping requested
        if result.x == -1 && wrap {
            let (wrap_line, wrap_col) = if forward {
                (0, 0)
            } else {
                let last_line = self.editor.get_line_count() - 1;
                let last_col = self.editor.get_line(last_line).len() as i32;
                (last_line, last_col)
            };
            result = self
                .editor
                .search(&GString::from(pattern), flags, wrap_line, wrap_col);
        }

        if result.x == -1 {
            return None;
        }

        let match_start = Position::new(i32_to_usize(result.y), i32_to_usize(result.x));
        let pattern_len = pattern.chars().count();
        let end_col = i32_to_usize(result.x) + pattern_len.saturating_sub(1);
        let match_end = Position::new(i32_to_usize(result.y), end_col);

        Some((match_start, match_end))
    }
}

/// Handles search motions (gn/gN).
///
/// Thin shell adapter: delegates to `vim_core::search_nth_match`.
/// Returns `Option<Selection>` if match is found.
#[allow(dead_code)]
pub fn execute_search_motion(
    editor: &mut Gd<CodeEdit>,
    vim_state: &mut VimState,
    motion: Motion,
    count: usize,
) -> Option<Selection> {
    if motion != Motion::SearchNextSelection && motion != Motion::SearchPrevSelection {
        return None;
    }

    let pattern = vim_state.search.last_search().map(String::from)?;
    if pattern.is_empty() {
        return None;
    }

    let search_forward = vim_state.search.last_search_forward();
    let forward = match motion {
        Motion::SearchNextSelection => search_forward,
        Motion::SearchPrevSelection => !search_forward,
        _ => return None,
    };

    let from = Position::new(
        i32_to_usize(editor.get_caret_line()),
        i32_to_usize(editor.get_caret_column()),
    );

    let provider = GodotSearchProvider { editor: &*editor };
    pure_motion::search_nth_selection(&provider, &pattern, from, forward, count)
}
