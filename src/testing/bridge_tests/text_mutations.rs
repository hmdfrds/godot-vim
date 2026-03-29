//! Text mutation tests. Covers both the raw MockTextEdit trait API (verifying the
//! mock itself is correct) and the bridge effect handlers (verifying byte-offset
//! to line/col translation produces correct mutations).

use crate::bridge::port::TextEditorPort;
use crate::testing::MockTextEdit;
use crate::types::CharLineCol;
use super::macros::{apply_insert, apply_delete, apply_replace};

// ── Text storage (validates MockTextEdit's text model) ──────────────────

#[test]
fn new_empty_document() {
    let mock = MockTextEdit::new("");
    assert_editor!(mock, text: "", line_count: 1, line(0): "");
}

#[test]
fn new_single_line() {
    let mock = MockTextEdit::new("hello");
    assert_editor!(mock, text: "hello", line_count: 1, line(0): "hello");
}

#[test]
fn new_multi_line() {
    let mock = MockTextEdit::new("hello\nworld");
    assert_editor!(mock,
        text: "hello\nworld",
        line_count: 2,
        line(0): "hello",
        line(1): "world",
    );
}

#[test]
fn get_line_out_of_bounds() {
    let mock = MockTextEdit::new("hello");
    assert_eq!(mock.get_line(-1), "");
    assert_eq!(mock.get_line(5), "");
}

// ── insert_text_at_caret ────────────────────────────────────────────────

#[test]
fn insert_text_at_caret_beginning() {
    let mut mock = MockTextEdit::new("hello");
    mock.insert_text_at_caret("abc");
    assert_editor!(mock, text: "abchello", cursor: (0, 3));
}

#[test]
fn insert_text_at_caret_with_selection() {
    let mut mock = MockTextEdit::new("hello world");
    mock.select(CharLineCol::new(0, 0), CharLineCol::new(0, 5)); // select "hello"
    mock.insert_text_at_caret("goodbye");
    assert_editor!(mock, text: "goodbye world", cursor: (0, 7));
}

#[test]
fn insert_newline_at_caret() {
    let mut mock = MockTextEdit::new("helloworld");
    mock.set_caret_column(5);
    mock.insert_text_at_caret("\n");
    assert_editor!(mock, text: "hello\nworld", cursor: (1, 0));
}

#[test]
fn insert_text_at_caret_multiline_with_selection() {
    let mut mock = MockTextEdit::new("hello world");
    mock.select(CharLineCol::new(0, 6), CharLineCol::new(0, 11)); // select "world"
    mock.insert_text_at_caret("beautiful\nworld");
    assert_editor!(mock, text: "hello beautiful\nworld", cursor: (1, 5));
}

// ── delete_selection ────────────────────────────────────────────────────

#[test]
fn delete_selection_single_line() {
    let mut mock = MockTextEdit::new("hello world");
    mock.select(CharLineCol::new(0, 5), CharLineCol::new(0, 11));
    mock.delete_selection();
    assert_editor!(mock, text: "hello", no_selection, cursor: (0, 5));
}

#[test]
fn delete_selection_no_selection() {
    let mut mock = MockTextEdit::new("hello");
    mock.delete_selection(); // should be no-op
    assert_editor!(mock, text: "hello");
}

#[test]
fn delete_selection_across_lines() {
    let mut mock = MockTextEdit::new("hello\nworld\nfoo");
    mock.select(CharLineCol::new(0, 3), CharLineCol::new(1, 2));
    mock.delete_selection();
    assert_editor!(mock, text: "helrld\nfoo", no_selection, cursor: (0, 3));
}

// ── Multi-caret insert ─────────────────────────────────────────────────
// Block-mode visual insert in Vim produces multi-caret inserts. The mock
// must correctly offset subsequent carets after each insertion.

#[test]
fn multi_caret_insert() {
    let mut mock = MockTextEdit::new("aaa\nbbb");
    mock.set_caret_line(0);
    mock.set_caret_column(0);
    mock.add_caret(1, 0);

    mock.insert_text_at_caret("X");
    assert_editor!(mock, text: "Xaaa\nXbbb");
}

// ── Effect handler integration (byte-offset -> line/col translation) ────

#[test]
fn effect_handle_insert() {
    let mut mock = MockTextEdit::new("hello\nworld");
    apply_insert(&mut mock, 5, " cruel");
    assert_editor!(mock, text: "hello cruel\nworld");
}

#[test]
fn effect_handle_delete() {
    let mut mock = MockTextEdit::new("hello world");
    apply_delete(&mut mock, 5, 11);
    assert_editor!(mock, text: "hello");
}

#[test]
fn effect_handle_replace() {
    let mut mock = MockTextEdit::new("hello world");
    apply_replace(&mut mock, 6, 11, "rust");
    assert_editor!(mock, text: "hello rust");
}
