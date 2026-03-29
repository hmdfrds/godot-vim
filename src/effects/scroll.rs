//! Applies scroll effects (scroll-to, center, top, bottom, horizontal scroll)
//! to CodeEdit's viewport.

use crate::bridge::port::TextEditorPort;
use crate::bridge::codec::DocumentView;

/// Scroll the viewport so that `offset` (byte position) is at the top.
///
/// If the target line is inside a folded region (hidden), the viewport is
/// scrolled to the nearest visible line at or above the target instead of
/// landing on a hidden line.
pub(super) fn handle_scroll_to(editor: &mut impl TextEditorPort, doc: &DocumentView, offset: usize) {
    let before = editor.get_first_visible_line();
    let line = doc.line_index.byte_to_line_col(doc.text, offset).line;
    let scroll_line = nearest_visible_line_at_or_above(editor, line);
    log::trace!("scroll_to: offset={} target={} scroll={} before={}", offset, line, scroll_line, before);
    editor.set_v_scroll(scroll_line.into());
}

/// Scan upward for the nearest visible (non-folded) line at or above `line`.
fn nearest_visible_line_at_or_above(editor: &impl TextEditorPort, line: i32) -> i32 {
    if line <= 0 {
        return 0;
    }
    let offset = editor.get_next_visible_line_offset_from(line, 1);
    if offset == 1 {
        return line;
    }
    // Backward offset counts inclusively, so visible line = line - offset + 1.
    let back_offset = editor.get_next_visible_line_offset_from(line, -1);
    (line - back_offset + 1).max(0)
}

/// Walk `count` visible lines backward from `start`, respecting fold regions.
///
/// Returns the resulting line number. Stops early if it reaches line 0 or
/// encounters a fold boundary that produces no progress (stall detection).
fn walk_visible_lines_back(editor: &impl TextEditorPort, start: i32, count: i32) -> i32 {
    let mut target = start;
    for _ in 0..count {
        let offset = editor.get_next_visible_line_offset_from(target, -1);
        let prev = (target - offset).max(0);
        if prev == target {
            break;
        }
        target = prev;
    }
    target
}

/// `zz` — center viewport around caret (fold-aware).
pub(crate) fn handle_center_cursor(editor: &mut impl TextEditorPort) {
    let caret_line = editor.get_caret_line();
    let half = editor.get_visible_line_count() / 2;
    let target = walk_visible_lines_back(editor, caret_line, half);
    log::trace!("center_cursor: line={}", target);
    editor.set_v_scroll(target.into());
}

/// `zt` — caret at top of viewport.
pub(crate) fn handle_cursor_to_top(editor: &mut impl TextEditorPort) {
    let line = editor.get_caret_line();
    log::trace!("cursor_to_top: line={}", line);
    editor.set_v_scroll(line.into());
}

/// `zb` — caret at bottom of viewport (fold-aware).
pub(crate) fn handle_cursor_to_bottom(editor: &mut impl TextEditorPort) {
    let caret_line = editor.get_caret_line();
    let visible_lines = editor.get_visible_line_count();
    if visible_lines <= 1 {
        editor.set_v_scroll(caret_line.into());
        return;
    }
    let target = walk_visible_lines_back(editor, caret_line, visible_lines - 1);
    log::trace!("cursor_to_bottom: line={}", target);
    editor.set_v_scroll(target.into());
}

/// `zh` — scroll viewport left by `count` columns.
pub(crate) fn handle_scroll_left(editor: &mut impl TextEditorPort, count: u32) {
    log::trace!("scroll_left: count={}", count);
    let scroll_amount = i32::try_from(count).unwrap_or(i32::MAX);
    let new_scroll = editor.get_h_scroll().saturating_sub(scroll_amount).max(0);
    editor.set_h_scroll(new_scroll);
}

/// `zl` — scroll viewport right by `count` columns.
pub(crate) fn handle_scroll_right(editor: &mut impl TextEditorPort, count: u32) {
    log::trace!("scroll_right: count={}", count);
    let scroll_amount = i32::try_from(count).unwrap_or(i32::MAX);
    let new_scroll = editor.get_h_scroll().saturating_add(scroll_amount);
    editor.set_h_scroll(new_scroll);
}

#[cfg(test)]
mod tests {
    use crate::bridge::port::TextEditorPort;
    use crate::testing::MockTextEdit;

    /// `zH` — scroll left by half a screen width.
    fn handle_scroll_half_left(editor: &mut impl TextEditorPort, screen_width: i32) {
        let half = (screen_width / 2).max(1);
        let current = editor.get_h_scroll();
        editor.set_h_scroll((current - half).max(0));
    }

    /// `zL` — scroll right by half a screen width.
    fn handle_scroll_half_right(editor: &mut impl TextEditorPort, screen_width: i32) {
        let half = (screen_width / 2).max(1);
        let current = editor.get_h_scroll();
        editor.set_h_scroll(current + half);
    }

    #[test]
    fn scroll_half_left() {
        let mut mock = MockTextEdit::new("hello");
        mock.set_h_scroll(20);
        handle_scroll_half_left(&mut mock, 80);
        assert_eq!(mock.get_h_scroll(), 0);
    }

    #[test]
    fn scroll_half_right() {
        let mut mock = MockTextEdit::new("hello");
        mock.set_h_scroll(10);
        handle_scroll_half_right(&mut mock, 80);
        assert_eq!(mock.get_h_scroll(), 50);
    }
}
