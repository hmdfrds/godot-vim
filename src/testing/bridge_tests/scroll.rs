//! Scroll tests: vertical/horizontal scroll state, viewport adjustment, and
//! scroll effect handlers (center, top, bottom, left/right).

use super::macros::{apply_set_selection, DispatchCtx};
use crate::bridge::codec::{DocumentView, LineIndex};
use crate::bridge::port::{FoldCapable, TextEditorPort, ViewportAdjust};
use crate::testing::MockTextEdit;
use vim_core::primitives::SelectionShape;

fn numbered(lines: usize) -> String {
    (0..lines)
        .map(|i| format!("line {}", i))
        .collect::<Vec<_>>()
        .join("\n")
}

// ── Scroll basics (validates MockTextEdit's scroll model) ───────────────

#[test]
fn scroll_basics() {
    let mut mock = MockTextEdit::new("a\nb\nc\nd\ne");
    mock.set_visible_line_count(2);
    mock.set_v_scroll(2.0);
    assert_editor!(mock, scroll: 2, h_scroll: 0);

    mock.set_h_scroll(10);
    assert_editor!(mock, h_scroll: 10);
}

#[test]
fn scroll_no_negative() {
    let mut mock = MockTextEdit::new("hello");
    mock.set_v_scroll(-5.0);
    assert_editor!(mock, scroll: 0);
    mock.set_h_scroll(-10);
    assert_editor!(mock, h_scroll: 0);
}

#[test]
fn adjust_viewport_scrolls_down() {
    let mut mock = MockTextEdit::new(
        &(0..50)
            .map(|i| format!("line {}", i))
            .collect::<Vec<_>>()
            .join("\n"),
    );
    mock.set_visible_line_count(10);
    mock.set_caret_line(30);
    mock.adjust_viewport_to_caret();
    // Row 20 stays a sliver at the top, as on Godot 4.6+.
    assert_eq!(mock.get_v_scroll(), 20.999);
    assert_editor!(mock, scroll: 20);
}

#[test]
fn v_scroll_stops_at_the_last_page() {
    let mut mock = MockTextEdit::new(&numbered(20));
    mock.set_visible_line_count(10);
    mock.set_v_scroll(50.0);
    assert_editor!(mock, scroll: 10);
}

#[test]
fn closed_fold_takes_no_rows() {
    let mut mock = MockTextEdit::new(&numbered(40));
    mock.set_visible_line_count(10);
    mock.add_fold_region(5, 14);
    mock.fold_line(5);
    mock.set_v_scroll(6.0);
    assert_editor!(mock, scroll: 15);
    mock.unfold_line(5);
    assert_editor!(mock, scroll: 15);
    assert_eq!(mock.get_v_scroll(), 15.0);
}

#[test]
fn fold_keeps_the_top_line() {
    let mut mock = MockTextEdit::new(&numbered(40));
    mock.set_visible_line_count(10);
    mock.add_fold_region(2, 9);
    mock.set_v_scroll(20.0);
    mock.fold_line(2);
    assert_editor!(mock, scroll: 20);
    assert_eq!(mock.get_v_scroll(), 13.0);
}

#[test]
fn wrapped_line_takes_a_row_per_wrap() {
    let mut mock = MockTextEdit::new(&numbered(40));
    mock.remove_text(2, 0, 2, 6);
    mock.insert_text(&"x".repeat(25), 2, 0);
    mock.set_visible_line_count(10);
    mock.set_wrap_width(10);
    mock.set_v_scroll(4.0);
    assert_editor!(mock, scroll: 2);
    mock.set_v_scroll(5.0);
    assert_editor!(mock, scroll: 3);
}

#[test]
fn caret_on_a_folded_line_lands_after_the_fold() {
    let mut mock = MockTextEdit::new(&numbered(40));
    mock.add_fold_region(5, 14);
    mock.add_fold_region(30, 39);
    mock.fold_all_lines();
    mock.set_caret_line_unfold(8, ViewportAdjust::NoAdjust);
    assert_eq!(mock.get_caret_line(), 15);
    mock.set_caret_line_unfold(35, ViewportAdjust::NoAdjust);
    assert_eq!(mock.get_caret_line(), 30);
}

#[test]
fn fold_moves_a_caret_inside_onto_its_header() {
    let mut mock = MockTextEdit::new(&numbered(40));
    mock.add_fold_region(5, 14);
    mock.set_caret_line(9);
    mock.fold_line(5);
    assert_editor!(mock, cursor: (5, 6));
}

// ── Scrolloff in screen rows ────────────────────────────────────────────

fn offset_of(mock: &MockTextEdit, line: i32) -> usize {
    let text = mock.get_text();
    LineIndex::new(&text).line_col_to_byte(&text, line, 0)
}

fn caret_to(mock: &mut MockTextEdit, line: i32, scrolloff: i32) {
    let text = mock.get_text();
    let li = LineIndex::new(&text);
    let doc = DocumentView::new(&text, &li);
    let offset = li.line_col_to_byte(&text, line, 0);
    crate::effects::cursor::handle_set_cursor(mock, &doc, offset, scrolloff);
}

fn caret_row(mock: &MockTextEdit) -> i32 {
    mock.get_scroll_pos_for_line(mock.get_caret_line(), mock.get_caret_wrap_index()) as i32
}

fn last_top_row(mock: &MockTextEdit) -> i32 {
    let last = mock.get_line_count() - 1;
    let rows = mock.get_scroll_pos_for_line(last, mock.get_line_wrap_count(last)) as i32 + 1;
    (rows - mock.get_visible_line_count()).max(0)
}

/// Puts the caret on each of `lines` in turn, as `j` and `k` do. The caret
/// keeps `so` rows above and below it, and the view never moves further
/// than the caret, nor the other way.
fn walk(mock: &mut MockTextEdit, lines: impl IntoIterator<Item = i32>, so: i32) {
    let rows = mock.get_visible_line_count();
    let mut before: Option<(i32, i32)> = None;
    for line in lines {
        caret_to(mock, line, so);
        assert_eq!(
            mock.get_v_scroll().fract(),
            0.0,
            "line {line}: part of a row"
        );
        let (top, caret) = (mock.get_v_scroll() as i32, caret_row(mock));
        let above = caret - top;
        let at_end = top == last_top_row(mock);
        if rows > 2 * so {
            assert!(above >= so.min(caret), "line {line}: {above} rows above");
            assert!(
                above <= rows - 1 - so || at_end,
                "line {line}: {above} rows above"
            );
        } else {
            assert!(
                above == (rows / 2).min(caret) || at_end,
                "line {line}: {above}"
            );
        }
        if let Some((top_was, caret_was)) = before {
            let (view, moved) = (top - top_was, caret - caret_was);
            assert!(
                view * moved >= 0 && view.abs() <= moved.abs(),
                "line {line}: the caret moved {moved} rows, the view {view}"
            );
        }
        before = Some((top, caret));
    }
}

fn folded(lines: usize, header: i32, last: i32, rows: i32) -> MockTextEdit {
    let mut mock = MockTextEdit::new(&numbered(lines));
    mock.set_visible_line_count(rows);
    mock.add_fold_region(header, last);
    mock.fold_line(header);
    mock
}

fn down_and_up(mock: &MockTextEdit, header: i32, last: i32) -> Vec<i32> {
    let lines: Vec<i32> = (0..mock.get_line_count())
        .filter(|l| *l <= header || *l > last)
        .collect();
    lines.iter().chain(lines.iter().rev()).copied().collect()
}

#[test]
fn scrolloff_moves_the_view_a_row_at_a_time_past_a_fold() {
    let mut mock = folded(80, 10, 22, 20);
    let lines = down_and_up(&mock, 10, 22);
    walk(&mut mock, lines, 5);
}

#[test]
fn scrolloff_counts_a_closed_fold_as_one_row() {
    let mut mock = folded(80, 10, 22, 20);
    walk(&mut mock, (0..=10).chain(23..=34), 5);
    // Line 34 is row 22, 14 rows under the top.
    assert_editor!(mock, cursor: (34, 0), scroll: 8);
    // Line 23 is row 11, 5 rows under the top.
    walk(&mut mock, (23..=34).rev(), 5);
    assert_editor!(mock, cursor: (23, 0), scroll: 6);
}

#[test]
fn scrolloff_past_a_fold_taller_than_the_view() {
    let mut mock = folded(120, 10, 70, 20);
    let lines = down_and_up(&mock, 10, 70);
    walk(&mut mock, lines, 5);
}

#[test]
fn scrolloff_keeps_the_caret_centred_in_a_short_view() {
    let mut mock = folded(80, 10, 22, 8);
    let lines = down_and_up(&mock, 10, 22);
    walk(&mut mock, lines, 5);
}

#[test]
fn scrolloff_counts_wrapped_rows() {
    let text: Vec<String> = (0..60)
        .map(|i| match i % 4 {
            0 => format!("{i:02} {}", "x".repeat(30)),
            _ => format!("line {i}"),
        })
        .collect();
    let mut mock = MockTextEdit::new(&text.join("\n"));
    mock.set_visible_line_count(20);
    mock.set_wrap_width(10);
    walk(&mut mock, (0..60).chain((0..60).rev()), 5);
}

#[test]
fn scrolloff_counts_the_carets_wrap_row() {
    let mut mock = MockTextEdit::new(&numbered(40));
    mock.remove_text(12, 0, 12, 7);
    mock.insert_text(&"x".repeat(45), 12, 0);
    mock.set_visible_line_count(20);
    mock.set_wrap_width(10);
    // Column 30 of line 12 is row 15, four rows above the bottom.
    let text = mock.get_text();
    let li = LineIndex::new(&text);
    let offset = li.line_col_to_byte(&text, 12, 30);
    crate::effects::cursor::handle_set_cursor(&mut mock, &DocumentView::new(&text, &li), offset, 5);
    assert_eq!(mock.get_v_scroll(), 1.0);
}

#[test]
fn scrolloff_follows_the_caret_without_folds() {
    let mut mock = MockTextEdit::new(&numbered(50));
    mock.set_visible_line_count(10);
    walk(&mut mock, (0..50).chain((0..50).rev()), 3);
    caret_to(&mut mock, 45, 3);
    assert_editor!(mock, scroll: 39);
}

#[test]
fn line_visual_head_below_a_fold_is_on_screen() {
    let mut mock = folded(80, 10, 22, 20);
    let (anchor, on_screen, off_screen) = (
        offset_of(&mock, 0),
        offset_of(&mock, 30),
        offset_of(&mock, 40),
    );
    apply_set_selection(&mut mock, anchor, on_screen, SelectionShape::Line);
    assert_editor!(mock, scroll: 0);
    // Line 40 is row 28; the view centres it.
    apply_set_selection(&mut mock, anchor, off_screen, SelectionShape::Line);
    assert_editor!(mock, scroll: 30);
}

// ── Scroll effect handlers ──────────────────────────────────────────────

#[test]
fn effect_center_cursor() {
    let mut mock = MockTextEdit::new(
        &(0..50)
            .map(|i| format!("line {}", i))
            .collect::<Vec<_>>()
            .join("\n"),
    );
    mock.set_visible_line_count(10);
    mock.set_caret_line(20);
    crate::effects::scroll::handle_center_cursor(&mut mock);
    assert_editor!(mock, scroll: 15);
}

#[test]
fn effect_cursor_to_top() {
    let mut mock = MockTextEdit::new(
        &(0..50)
            .map(|i| format!("line {}", i))
            .collect::<Vec<_>>()
            .join("\n"),
    );
    mock.set_caret_line(20);
    crate::effects::scroll::handle_cursor_to_top(&mut mock);
    assert_editor!(mock, scroll: 20);
}

#[test]
fn effect_scroll_left_right() {
    let mut mock = MockTextEdit::new("hello");
    mock.set_h_scroll(10);
    crate::effects::scroll::handle_scroll_left(&mut mock, 3);
    assert_editor!(mock, h_scroll: 7);
    crate::effects::scroll::handle_scroll_right(&mut mock, 5);
    assert_editor!(mock, h_scroll: 12);
}

#[test]
fn effect_cursor_to_bottom() {
    let mut mock = MockTextEdit::new(
        &(0..50)
            .map(|i| format!("line {}", i))
            .collect::<Vec<_>>()
            .join("\n"),
    );
    mock.set_visible_line_count(10);
    mock.set_caret_line(20);
    crate::effects::scroll::handle_cursor_to_bottom(&mut mock);
    assert_editor!(mock, scroll: 11);
}

#[test]
fn effect_center_cursor_precise() {
    let mut mock = MockTextEdit::new(
        &(0..50)
            .map(|i| format!("line {}", i))
            .collect::<Vec<_>>()
            .join("\n"),
    );
    mock.set_visible_line_count(10);
    mock.set_caret_line(20);
    crate::effects::scroll::handle_center_cursor(&mut mock);
    assert_editor!(mock, scroll: 15);
}

// half_left / half_right are #[cfg(test)] private in effects::scroll,
// so they are tested there, not here.

// ── zt, zz, zb and ScrollTo in screen rows ──────────────────────────────

fn two_folds() -> MockTextEdit {
    let mut mock = MockTextEdit::new(&numbered(80));
    mock.set_visible_line_count(20);
    mock.add_fold_region(2, 5);
    mock.add_fold_region(10, 22);
    mock.fold_all_lines();
    mock
}

fn scroll_to(mock: &mut MockTextEdit, line: i32) {
    let offset = offset_of(mock, line);
    DispatchCtx::new().dispatch(mock, effects![scroll_to(offset)]);
}

#[test]
fn zt_zz_zb_count_folded_rows() {
    let mut mock = two_folds();
    // Line 60 is row 45.
    mock.set_caret_line(60);
    crate::effects::scroll::handle_cursor_to_top(&mut mock);
    assert_editor!(mock, scroll: 60);
    crate::effects::scroll::handle_center_cursor(&mut mock);
    assert_editor!(mock, scroll: 50);
    crate::effects::scroll::handle_cursor_to_bottom(&mut mock);
    assert_editor!(mock, scroll: 41);
}

#[test]
fn zt_zz_zb_count_wrapped_rows() {
    let mut mock = MockTextEdit::new(&numbered(80));
    mock.remove_text(5, 0, 5, 6);
    mock.insert_text(&"x".repeat(45), 5, 0);
    mock.set_visible_line_count(20);
    mock.set_wrap_width(10);
    // Line 5 takes rows 5 to 9, so line 40 is row 44.
    mock.set_caret_line(40);
    crate::effects::scroll::handle_cursor_to_top(&mut mock);
    assert_editor!(mock, scroll: 40);
    crate::effects::scroll::handle_center_cursor(&mut mock);
    assert_editor!(mock, scroll: 30);
    crate::effects::scroll::handle_cursor_to_bottom(&mut mock);
    assert_editor!(mock, scroll: 21);
    // Line 12 is row 16, ten rows under line 5's second row.
    mock.set_caret_line(12);
    crate::effects::scroll::handle_center_cursor(&mut mock);
    assert_editor!(mock, scroll: 5);
    assert_eq!(mock.get_v_scroll(), 6.0);
}

#[test]
fn scroll_to_moves_one_row_over_a_fold_header() {
    let mut mock = two_folds();
    scroll_to(&mut mock, 10);
    assert_editor!(mock, scroll: 10);
    // Ctrl-E from the header asks for its first hidden line.
    scroll_to(&mut mock, 11);
    assert_editor!(mock, scroll: 23);
    // Ctrl-Y asks for the last hidden line.
    scroll_to(&mut mock, 22);
    assert_editor!(mock, scroll: 10);
    // Ctrl-D from above the header asks for a hidden line.
    scroll_to(&mut mock, 7);
    scroll_to(&mut mock, 14);
    assert_editor!(mock, scroll: 10);
}

#[test]
fn scroll_to_goes_up_from_the_top_before_the_caret_moved() {
    let mut mock = two_folds();
    scroll_to(&mut mock, 23);
    // Ctrl-U moves the caret first, and the view with it.
    let (caret, top) = (offset_of(&mock, 7), offset_of(&mock, 15));
    DispatchCtx::new().dispatch(&mut mock, effects![set_cursor(caret); scroll_to(top)]);
    assert_editor!(mock, scroll: 10);
}

#[test]
fn scroll_to_judges_the_direction_before_scrolloff_moves_the_view() {
    let mut ctx = DispatchCtx::new().with_scrolloff(1);
    let mut mock = two_folds();
    scroll_to(&mut mock, 10);
    // Ctrl-D from the header: scrolloff takes the view off the header.
    let (caret, top) = (offset_of(&mock, 41), offset_of(&mock, 20));
    ctx.dispatch(&mut mock, effects![set_cursor(caret); scroll_to(top)]);
    assert_editor!(mock, scroll: 23);
    // Ctrl-U from below the fold: scrolloff puts the view on the header.
    let (caret, top) = (offset_of(&mock, 23), offset_of(&mock, 13));
    ctx.dispatch(&mut mock, effects![set_cursor(caret); scroll_to(top)]);
    assert_editor!(mock, scroll: 10);
}

#[test]
fn scroll_to_puts_the_line_on_top_below_a_fold() {
    let mut mock = two_folds();
    scroll_to(&mut mock, 7);
    assert_editor!(mock, scroll: 7);
    scroll_to(&mut mock, 30);
    assert_editor!(mock, scroll: 30);
}
