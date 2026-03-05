//! Extension trait for CodeEdit with fold-aware cursor operations.
//!
//! Provides methods that correctly handle folded code regions:
//! - `move_line_skip_folds()` - For line motions (j/k), skips folded lines
//! - `set_line_unfold()` - For jump motions (G/gg/search), unfolds to show target

use godot::classes::CodeEdit;
use godot::prelude::*;

/// Extension trait providing fold-aware cursor operations for CodeEdit.
///
/// # Vim Behavior
/// - **Line motions** (`j`, `k`): Skip over folded lines (treat fold as single line)
/// - **Jump motions** (`G`, `gg`, search, marks): Unfold to reveal target line
pub trait CodeEditExt {
    /// Move up one visible line, skipping folded lines.
    /// Returns the new line index.
    ///
    /// Uses Godot's formula: `current - offset_from(current-1, -1)`
    fn move_up_visible(&self, current_line: i32) -> i32;

    /// Move down one visible line, skipping folded lines.
    /// Returns the new line index.
    ///
    /// Uses Godot's formula: `current + offset_from(current+1, 1)`
    fn move_down_visible(&self, current_line: i32) -> i32;

    /// Sets caret line, UNFOLDING if the target is hidden.
    ///
    /// Use for jump motions like `G`, `gg`, search, marks.
    fn set_line_unfold(&mut self, line: i32);
}

impl CodeEditExt for Gd<CodeEdit> {
    fn move_up_visible(&self, current_line: i32) -> i32 {
        if current_line <= 0 {
            return 0;
        }
        // new_line = get_caret_line(i) - get_next_visible_line_offset_from(get_caret_line(i) - 1, -1)
        let offset = self.get_next_visible_line_offset_from(current_line - 1, -1);
        (current_line - offset).max(0)
    }

    fn move_down_visible(&self, current_line: i32) -> i32 {
        let line_count = self.get_line_count();
        if current_line >= line_count - 1 {
            return line_count - 1;
        }
        // new_line = get_caret_line(i) + get_next_visible_line_offset_from(CLAMP(get_caret_line(i) + 1, 0, text.size() - 1), 1)
        let from = (current_line + 1).clamp(0, line_count - 1);
        let offset = self.get_next_visible_line_offset_from(from, 1);
        (current_line + offset).min(line_count - 1)
    }

    fn set_line_unfold(&mut self, line: i32) {
        // can_be_hidden(false) tells Godot to unfold the line if it's hidden
        self.set_caret_line_ex(line).can_be_hidden(false).done();
    }
}
