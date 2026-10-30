//! Applies scroll effects (scroll-to, center, top, bottom, horizontal scroll)
//! to CodeEdit's viewport.

use super::viewport;
use crate::bridge::codec::{self, DocumentView};
use crate::bridge::port::TextEditorPort;

/// Scroll the viewport so that `offset` (byte position) is at the top.
///
/// `before` is the top line the engine scrolled from: the caret moves first,
/// and scrolloff may have moved the view with it. A target inside a closed
/// fold gives the fold's header, or the line after the fold when the view
/// moves down from that header, so Ctrl-E steps over it.
pub(super) fn handle_scroll_to(
    editor: &mut impl TextEditorPort,
    doc: &DocumentView,
    offset: usize,
    before: i32,
) {
    let line = doc.line_index.byte_to_line_col(doc.text, offset).line;
    let header = nearest_visible_line_at_or_above(editor, line);
    let scroll_line = if header == before && line > before {
        line
    } else {
        header
    };
    log::trace!(
        "scroll_to: offset={} target={} scroll={} before={}",
        offset,
        line,
        scroll_line,
        before
    );
    editor.set_line_as_first_visible(scroll_line, 0);
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

/// `zz` — center viewport around caret (fold-aware).
pub(crate) fn handle_center_cursor(editor: &mut impl TextEditorPort) {
    let rows = editor.get_visible_line_count();
    log::trace!("center_cursor: line={}", editor.get_caret_line());
    viewport::put_caret_row(editor, rows / 2);
}

/// `zt` — caret at top of viewport.
pub(crate) fn handle_cursor_to_top(editor: &mut impl TextEditorPort) {
    log::trace!("cursor_to_top: line={}", editor.get_caret_line());
    viewport::put_caret_row(editor, 0);
}

/// `zb` — caret at bottom of viewport (fold-aware).
pub(crate) fn handle_cursor_to_bottom(editor: &mut impl TextEditorPort) {
    let rows = editor.get_visible_line_count();
    log::trace!("cursor_to_bottom: line={}", editor.get_caret_line());
    viewport::put_caret_row(editor, (rows - 1).max(0));
}

/// `zh` — scroll viewport left by `count` columns.
pub(crate) fn handle_scroll_left(editor: &mut impl TextEditorPort, count: u32) {
    log::trace!("scroll_left: count={}", count);
    let scroll_amount = codec::u32_to_i32_sat(count);
    let new_scroll = editor.get_h_scroll().saturating_sub(scroll_amount).max(0);
    editor.set_h_scroll(new_scroll);
}

/// `zl` — scroll viewport right by `count` columns.
pub(crate) fn handle_scroll_right(editor: &mut impl TextEditorPort, count: u32) {
    log::trace!("scroll_right: count={}", count);
    let scroll_amount = codec::u32_to_i32_sat(count);
    let new_scroll = editor.get_h_scroll().saturating_add(scroll_amount);
    editor.set_h_scroll(new_scroll);
}

/// `zH` — scroll viewport left by half screen width.
pub(crate) fn handle_scroll_half_left(editor: &mut impl TextEditorPort, count: u32) {
    let visible_cols = editor.get_visible_line_count(); // approximate screen width
    let half = (visible_cols / 2).max(1);
    let shift = half * count as i32;
    log::trace!("scroll_half_left: count={} shift={}", count, shift);
    let new_scroll = editor.get_h_scroll().saturating_sub(shift).max(0);
    editor.set_h_scroll(new_scroll);
}

/// `zL` — scroll viewport right by half screen width.
pub(crate) fn handle_scroll_half_right(editor: &mut impl TextEditorPort, count: u32) {
    let visible_cols = editor.get_visible_line_count(); // approximate screen width
    let half = (visible_cols / 2).max(1);
    let shift = half * count as i32;
    log::trace!("scroll_half_right: count={} shift={}", count, shift);
    let new_scroll = editor.get_h_scroll().saturating_add(shift);
    editor.set_h_scroll(new_scroll);
}

/// `zs` — scroll viewport so cursor column is at left edge.
pub(crate) fn handle_cursor_to_left_edge(editor: &mut impl TextEditorPort) {
    let col = editor.get_caret_column();
    log::trace!("cursor_to_left_edge: col={}", col);
    editor.set_h_scroll(col);
}

/// `ze` — scroll viewport so cursor column is at right edge.
pub(crate) fn handle_cursor_to_right_edge(editor: &mut impl TextEditorPort) {
    let col = editor.get_caret_column();
    let visible_cols = editor.get_visible_line_count(); // approximate
    let target = (col - visible_cols + 1).max(0);
    log::trace!("cursor_to_right_edge: col={} target={}", col, target);
    editor.set_h_scroll(target);
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
