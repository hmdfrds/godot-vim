//! Fold-aware cursor movement for CodeEdit.
//!
//! Vim's `j`/`k` motions treat a folded region as a single line. Godot's
//! `get_next_visible_line_offset_from` provides the skip distance, but its
//! semantics are non-obvious (see impl comments), so this trait wraps it
//! into a clear up/down interface.

use godot::classes::CodeEdit;
use godot::prelude::*;

/// Fold-aware vertical movement on `Gd<CodeEdit>`.
///
/// Vim distinguishes two fold behaviors:
/// - **Line motions** (`j`, `k`): skip folded lines (fold = one visible line).
/// - **Jump motions** (`G`, `gg`, search, marks): unfold to reveal the target.
///
/// This trait handles the first case. Jump-target unfolding is done separately
/// by `set_caret_line_unfold` (which passes `can_be_hidden(false)` to Godot).
pub(crate) trait CodeEditExt {
    fn move_up_visible(&self, current_line: i32) -> i32;
    fn move_down_visible(&self, current_line: i32) -> i32;
}

impl CodeEditExt for Gd<CodeEdit> {
    fn move_up_visible(&self, current_line: i32) -> i32 {
        if current_line <= 0 {
            return 0;
        }
        // Godot's offset_from returns how many *document* lines to skip to
        // reach 1 visible line. Probing from (current-1) with direction -1
        // gives the distance to the previous visible line above.
        let offset = self.get_next_visible_line_offset_from(current_line - 1, -1);
        (current_line - offset).max(0)
    }

    fn move_down_visible(&self, current_line: i32) -> i32 {
        let line_count = self.get_line_count();
        if current_line >= line_count - 1 {
            return line_count - 1;
        }
        // Probe from (current+1) forward to find the next visible line.
        // Clamping `from` prevents out-of-range input to the Godot API.
        let from = (current_line + 1).clamp(0, line_count - 1);
        let offset = self.get_next_visible_line_offset_from(from, 1);
        (current_line + offset).min(line_count - 1)
    }
}
