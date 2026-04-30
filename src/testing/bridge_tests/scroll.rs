//! Scroll tests: vertical/horizontal scroll state, viewport adjustment, and
//! scroll effect handlers (center, top, bottom, left/right).

use crate::bridge::port::TextEditorPort;
use crate::testing::MockTextEdit;

// ── Scroll basics (validates MockTextEdit's scroll model) ───────────────

#[test]
fn scroll_basics() {
    let mut mock = MockTextEdit::new("a\nb\nc\nd\ne");
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
    assert_editor!(mock, scroll: 21);
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
