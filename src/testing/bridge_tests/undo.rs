//! Undo/redo tests. Validates that MockTextEdit's undo model (group-based,
//! with caret snapshots) correctly supports the vim plugin's usage pattern:
//! each `begin/end_complex_operation` pair forms one atomic undo step, nested
//! pairs collapse to one step, and redo history is discarded on new edits.

use crate::bridge::port::TextEditorPort;
use crate::testing::MockTextEdit;
use crate::types::CharLineCol;

// ── Undo / Redo via trait API (validates MockTextEdit's undo model) ─────

#[test]
fn undo_single_insert() {
    let mut mock = MockTextEdit::new("hello");
    mock.begin_complex_operation();
    mock.set_caret_column(5);
    mock.insert_text_at_caret(" world");
    mock.end_complex_operation();
    assert_editor!(mock, text: "hello world");

    mock.undo();
    assert_editor!(mock, text: "hello");
}

#[test]
fn redo_after_undo() {
    let mut mock = MockTextEdit::new("hello");
    mock.begin_complex_operation();
    mock.insert_text_at_caret("abc");
    mock.end_complex_operation();
    assert_editor!(mock, text: "abchello");

    mock.undo();
    assert_editor!(mock, text: "hello");

    mock.redo();
    assert_editor!(mock, text: "abchello");
}

#[test]
fn undo_complex_group() {
    let mut mock = MockTextEdit::new("hello");
    mock.begin_complex_operation();
    mock.select(CharLineCol::new(0, 0), CharLineCol::new(0, 5));
    mock.delete_selection();
    mock.insert_text_at_caret("world");
    mock.end_complex_operation();
    assert_editor!(mock, text: "world");

    mock.undo();
    assert_editor!(mock, text: "hello");
}

#[test]
fn undo_redo_multiple_steps() {
    let mut mock = MockTextEdit::new("");

    mock.begin_complex_operation();
    mock.insert_text_at_caret("a");
    mock.end_complex_operation();

    mock.begin_complex_operation();
    mock.insert_text_at_caret("b");
    mock.end_complex_operation();

    mock.begin_complex_operation();
    mock.insert_text_at_caret("c");
    mock.end_complex_operation();

    assert_editor!(mock, text: "abc");

    mock.undo();
    assert_editor!(mock, text: "ab");

    mock.undo();
    assert_editor!(mock, text: "a");

    mock.redo();
    assert_editor!(mock, text: "ab");

    mock.redo();
    assert_editor!(mock, text: "abc");
}

#[test]
fn undo_nothing_is_noop() {
    let mut mock = MockTextEdit::new("hello");
    mock.undo();
    assert_editor!(mock, text: "hello");
}

#[test]
fn redo_nothing_is_noop() {
    let mut mock = MockTextEdit::new("hello");
    mock.redo();
    assert_editor!(mock, text: "hello");
}

#[test]
fn undo_clears_redo_on_new_edit() {
    let mut mock = MockTextEdit::new("");

    mock.begin_complex_operation();
    mock.insert_text_at_caret("a");
    mock.end_complex_operation();

    mock.begin_complex_operation();
    mock.insert_text_at_caret("b");
    mock.end_complex_operation();

    mock.undo();
    assert_editor!(mock, text: "a");

    // New edit after undo must discard redo history.
    mock.begin_complex_operation();
    mock.insert_text_at_caret("x");
    mock.end_complex_operation();

    assert_editor!(mock, text: "ax");
    mock.redo(); // no-op: redo history was cleared
    assert_editor!(mock, text: "ax");
}

#[test]
fn undo_restores_caret_position() {
    let mut mock = MockTextEdit::new("hello");
    mock.set_caret_column(5);
    mock.begin_complex_operation();
    mock.set_caret_column(0);
    mock.insert_text_at_caret("X");
    mock.end_complex_operation();
    assert_editor!(mock, text: "Xhello", cursor: (0, 1));

    mock.undo();
    assert_editor!(mock, text: "hello", cursor: (0, 5));
}

#[test]
fn nested_complex_operations() {
    // Nested begin/end pairs collapse to one undo group (outermost wins).
    let mut mock = MockTextEdit::new("hello");
    mock.begin_complex_operation();
    mock.begin_complex_operation(); // nested
    mock.select(CharLineCol::new(0, 0), CharLineCol::new(0, 5));
    mock.delete_selection();
    mock.insert_text_at_caret("world");
    mock.end_complex_operation(); // inner
    mock.end_complex_operation(); // outer
    assert_editor!(mock, text: "world");

    mock.undo();
    assert_editor!(mock, text: "hello");
}

// ── Undo / Redo inverse property ─────────────────────────────────────────
// For any insert, undo must exactly restore the original text. This is a
// lightweight property test across several input shapes.

#[test]
fn undo_redo_inverse_deterministic() {
    for (initial, insert, pos) in &[
        ("hello world", "XYZ", 5),
        ("line1\nline2\nline3", "NEW", 6),
        ("abcdef", "123", 0),
    ] {
        let mut mock = MockTextEdit::new(initial);
        mock.begin_complex_operation();
        mock.set_caret_line(0);
        mock.set_caret_column(*pos as i32);
        mock.insert_text_at_caret(insert);
        mock.end_complex_operation();
        mock.undo();
        assert_eq!(
            mock.get_text(),
            *initial,
            "Undo failed for initial={:?} insert={:?} pos={}",
            initial,
            insert,
            pos
        );
    }
}

// ── Undo effect handlers (bridge layer, not raw trait API) ──────────────

#[test]
fn effect_undo_redo_group() {
    let mut mock = MockTextEdit::new("hello");
    let mut depth = crate::effects::UndoDepth::new();

    crate::effects::undo::handle_begin_undo_group(&mut mock, &mut depth);

    super::macros::apply_delete(&mut mock, 0, 5);
    super::macros::apply_insert(&mut mock, 0, "world");

    crate::effects::undo::handle_end_undo_group(&mut mock, &mut depth);
    assert_editor!(mock, text: "world");

    crate::effects::undo::handle_undo(&mut mock, 1);
    assert_editor!(mock, text: "hello");

    crate::effects::undo::handle_redo(&mut mock, 1);
    assert_editor!(mock, text: "world");
}
