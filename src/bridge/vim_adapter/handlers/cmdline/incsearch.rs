use super::IncsearchHandler;
use crate::bridge::vim_adapter::managers::preview::{
    find_unescaped_delimiter, parse_substitute_command,
};
use crate::bridge::vim_wrapper::VimController;
use godot::classes::text_edit::SearchFlags;
use godot::prelude::*;

impl IncsearchHandler for VimController {
    fn update_incsearch_highlights(&mut self, text: &str) {
        let pattern = parse_incsearch_pattern(text);

        if let Some(mut editor) = self.get_editor() {
            // Set case-sensitive search for highlighting
            editor.set_search_flags(SearchFlags::MATCH_CASE);
            editor.set_search_text(&GString::from(pattern));
            // Force redraw to show highlights immediately
            editor.queue_redraw();

            // For substitute commands, apply live preview via manager
            if let Some(cmd) = parse_substitute_command(text) {
                self.visuals.substitute_preview.apply(&cmd, &mut editor);
            } else {
                // Not a substitute command - revert any preview
                self.visuals.substitute_preview.revert(&mut editor);
            }
        }
    }

    fn clear_incsearch_highlights(&mut self) {
        if let Some(mut editor) = self.get_editor() {
            editor.set_search_text(&GString::new());
            // Force redraw to clear highlights immediately
            editor.queue_redraw();
            // Revert substitute preview on ESC
            self.visuals.substitute_preview.revert(&mut editor);
        }
    }
}

/// Parse search pattern from command line text for incremental search.
///
/// Extracts pattern from:
/// - `/pattern` or `?pattern` (search commands)
/// - `:s/pattern/...` or `:%s/pattern/...` (substitute commands)
/// - `:1,5s/pattern/...` (range substitute)
///
/// Returns the pattern to highlight, or empty string if none.
fn parse_incsearch_pattern(text: &str) -> &str {
    // Search commands: /pattern or ?pattern
    if let Some(stripped) = text.strip_prefix('/').or_else(|| text.strip_prefix('?')) {
        return stripped;
    }

    // Substitute commands: :s/pattern/... or :%s/pattern/...
    if let Some(after_colon) = text.strip_prefix(':') {
        let pattern_start = after_colon
            .strip_prefix("s/")
            .or_else(|| after_colon.strip_prefix("%s/"))
            .or_else(|| after_colon.find("s/").map(|idx| &after_colon[idx + 2..]));

        if let Some(rest) = pattern_start {
            // Find the closing delimiter (next unescaped /)
            if let Some(end_idx) = find_unescaped_delimiter(rest, '/') {
                return &rest[..end_idx];
            }
            // No closing delimiter yet - use the whole thing as pattern
            return rest;
        }
    }

    // Not a search/substitute command
    ""
}
