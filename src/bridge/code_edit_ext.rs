//! Safe accessors and fold-aware cursor movement for CodeEdit.
//!
//! **Safe accessors** — Godot's `get_indent_size()`, `get_tab_size()`,
//! `get_line_height()`, `get_visible_line_count()`, and `get_line_count()`
//! all return `i32` and can theoretically be zero or negative. Every call
//! site previously needed `.max(1)` and `codec::i32_to_usize()` to prevent
//! division-by-zero or underflow. These helpers centralize the clamping.
//!
//! **Fold-aware movement** — Vim's `j`/`k` motions treat a folded region as
//! a single line. Godot's `get_next_visible_line_offset_from` provides the
//! skip distance, but its semantics are non-obvious (see impl comments), so
//! `move_up_visible`/`move_down_visible` wrap it into a clear interface.

use godot::classes::CodeEdit;
use godot::prelude::*;

use super::codec;

/// Safe accessors and fold-aware vertical movement on `Gd<CodeEdit>`.
///
/// ## Safe accessors
///
/// Five `safe_*` methods replace scattered `.max(1)` + `codec::i32_to_usize()`
/// chains throughout the codebase. Each clamps to at least 1 before converting,
/// preventing division-by-zero, zero-pixel heights, and usize underflow.
///
/// ## Fold-aware movement
///
/// Vim distinguishes two fold behaviors:
/// - **Line motions** (`j`, `k`): skip folded lines (fold = one visible line).
/// - **Jump motions** (`G`, `gg`, search, marks): unfold to reveal the target.
///
/// `move_up_visible`/`move_down_visible` handle the first case. Jump-target
/// unfolding is done separately by `set_caret_line_unfold` (which passes
/// `can_be_hidden(false)` to Godot).
pub(crate) trait CodeEditExt {
    /// Indent size in characters, clamped to at least 1.
    fn safe_indent_size(&self) -> usize;
    /// Tab stop width in characters, clamped to at least 1.
    fn safe_tab_size(&self) -> usize;
    /// Line height in pixels, clamped to at least 1.
    fn safe_line_height(&self) -> i32;
    /// Number of visible lines in the viewport, clamped to at least 1.
    fn safe_visible_line_count(&self) -> usize;
    /// Total document line count, clamped to at least 1.
    fn safe_line_count(&self) -> usize;

    fn move_up_visible(&self, current_line: i32) -> i32;
    fn move_down_visible(&self, current_line: i32) -> i32;
}

impl CodeEditExt for Gd<CodeEdit> {
    fn safe_indent_size(&self) -> usize {
        codec::i32_to_usize(self.get_indent_size().max(1))
    }

    fn safe_tab_size(&self) -> usize {
        codec::i32_to_usize(self.get_tab_size().max(1))
    }

    fn safe_line_height(&self) -> i32 {
        self.get_line_height().max(1)
    }

    fn safe_visible_line_count(&self) -> usize {
        codec::i32_to_usize(self.get_visible_line_count().max(1))
    }

    fn safe_line_count(&self) -> usize {
        codec::i32_to_usize(self.get_line_count().max(1))
    }

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
        let last_line = self.safe_line_count().saturating_sub(1);
        let last_line_i32 = codec::usize_to_i32(last_line);
        if current_line >= last_line_i32 {
            return last_line_i32;
        }
        // Probe from (current+1) forward to find the next visible line.
        // Clamping `from` prevents out-of-range input to the Godot API.
        let from = (current_line + 1).clamp(0, last_line_i32);
        let offset = self.get_next_visible_line_offset_from(from, 1);
        (current_line + offset).min(last_line_i32)
    }
}
