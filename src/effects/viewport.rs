//! The view in screen rows, as Godot scrolls it: a closed fold is one row
//! and a wrapped line one row per wrap, while carets count document lines.

use crate::bridge::port::TextEditorPort;

fn row_of(editor: &impl TextEditorPort, line: i32, wrap: i32) -> i32 {
    editor.get_scroll_pos_for_line(line, wrap) as i32
}

pub(crate) fn caret_row(editor: &impl TextEditorPort) -> i32 {
    row_of(
        editor,
        editor.get_caret_line(),
        editor.get_caret_wrap_index(),
    )
}

/// The row at the top. `v_scroll` lags behind a fold change above the top
/// line, so it only gives the top line's wrap row.
pub(crate) fn top_row(editor: &impl TextEditorPort) -> i32 {
    let line = editor.get_first_visible_line();
    let start = row_of(editor, line, 0);
    let wrap =
        (editor.get_v_scroll().floor() as i32 - start).clamp(0, editor.get_line_wrap_count(line));
    start + wrap
}

/// Scrolls so the caret's row is `below_top` rows under the top, or as near
/// as the start of the document allows.
pub(crate) fn put_caret_row(editor: &mut impl TextEditorPort, below_top: i32) {
    let line = editor.get_caret_line();
    let wrap = editor.get_caret_wrap_index();
    let rows = below_top.clamp(0, row_of(editor, line, wrap));
    let (walked, top_wrap) = editor.get_next_visible_line_index_offset_from(line, wrap, -rows - 1);
    editor.set_line_as_first_visible((line - walked + 1).max(0), top_wrap);
}
