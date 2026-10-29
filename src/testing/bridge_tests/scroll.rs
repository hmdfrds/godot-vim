//! Scroll tests: vertical/horizontal scroll state, viewport adjustment, and
//! scroll effect handlers (center, top, bottom, left/right).

use crate::bridge::port::{FoldCapable, TextEditorPort, ViewportAdjust};
use crate::testing::MockTextEdit;

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
