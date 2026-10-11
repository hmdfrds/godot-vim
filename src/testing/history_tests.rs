//! Regression tests for bugs users reported, one section per issue. Keys go
//! through the real engine and the real dispatch (`MockHost`), so each test
//! fails if either side brings its bug back.

use super::mock_host::{Harness, GODOT_PAIRS};
use crate::bridge::port::TextEditorPort;
use crate::testing::MockTextEdit;
use vim_core::primitives::{Mode, VisualType};

// ── #40: a Visual-block change is one undo step ─────────────────────────────
// vim-core closed the undo group after the block delete (fixed in v0.7.0).

const ISSUE_40_BUFFER: &str = "func _ready() -> void:\n\
    \tprint(\"This is some code.\")\n\
    \tprint(\"This is also a line of code.\")\n\
    \tprint(\"And one more for good measure.\")";

/// The issue's steps: `print` on three lines, selected from its first letter.
fn issue_40_block_on_print() -> Harness {
    let mut h = Harness::new(ISSUE_40_BUFFER);
    h.place_caret(1, 1);
    h.feed("<C-v>lllljj");
    assert_eq!(h.mode(), Mode::Visual(VisualType::Block));
    h
}

fn assert_issue_40_changed(h: &Harness, word: &str) {
    for (line, rest) in [
        (1, "(\"This is some code.\")"),
        (2, "(\"This is also a line of code.\")"),
        (3, "(\"And one more for good measure.\")"),
    ] {
        assert_eq!(h.line(line), format!("\t{word}{rest}"), "line {line}");
    }
}

#[test]
fn issue_40_block_change_undoes_in_one_step() {
    let mut h = issue_40_block_on_print();
    h.feed("cchange<Esc>");
    assert_issue_40_changed(&h, "change");

    h.feed("u");
    assert_eq!(h.text(), ISSUE_40_BUFFER, "one u restores every line");
    assert_eq!(h.caret(), (1, 1), "u puts the cursor at the block's corner");

    h.feed("<C-r>");
    assert_issue_40_changed(&h, "change");
    assert_eq!(h.orphaned_undo_groups(), 0);
}

#[test]
fn issue_40_backspace_inside_the_block_change_is_part_of_the_step() {
    let mut h = issue_40_block_on_print();
    h.feed("cchanx<BS>ge<Esc>");
    assert_issue_40_changed(&h, "change");

    h.feed("u");
    assert_eq!(h.text(), ISSUE_40_BUFFER);
    assert_eq!(h.orphaned_undo_groups(), 0);
}

#[test]
fn issue_40_block_substitute_undoes_in_one_step() {
    let mut h = issue_40_block_on_print();
    h.feed("sshow<Esc>");
    assert_issue_40_changed(&h, "show");

    h.feed("u");
    assert_eq!(h.text(), ISSUE_40_BUFFER);
    h.feed("<C-r>");
    assert_issue_40_changed(&h, "show");
    assert_eq!(h.orphaned_undo_groups(), 0);
}

// ── #13: the caret jumped after completion, brackets and re-entering Insert ──
// Four symptoms of a refactor in 0.8.18 (likely the column codec).

/// What `CodeEdit::confirm_code_completion(false)` does for `option` when the
/// caret ends `base` on `line`: replace the base, caret after the option.
fn confirm(line: i32, base: &str, option: &str) -> impl FnOnce(&mut MockTextEdit) {
    let base_chars = base.chars().count() as i32;
    let option_chars = option.chars().count() as i32;
    let option = option.to_owned();
    move |editor| {
        let end = editor.get_caret_column();
        let start = end - base_chars;
        editor.remove_text(line, start, line, end);
        editor.insert_text(&option, line, start);
        editor.set_caret_line(line);
        editor.set_caret_column(start + option_chars);
    }
}

#[test]
fn issue_13_typing_after_an_accepted_completion_continues_after_the_word() {
    let mut h = Harness::new("func _ready() -> void:\n\tvar p = .x");
    h.place_caret(1, 9);
    h.feed("ipos");
    h.accept_completion(confirm(1, "pos", "position"));
    assert_eq!(h.line(1), "\tvar p = position.x");
    assert_eq!(h.caret(), (1, 17));

    h.feed("2");
    assert_eq!(h.line(1), "\tvar p = position2.x");
    assert_eq!(h.caret(), (1, 18));
    h.feed("<Esc>");
    assert_eq!(h.caret(), (1, 17), "Esc rests on the last typed character");
}

#[test]
fn issue_13_entering_insert_mid_line_does_not_move_the_caret_forward() {
    let mut h = Harness::new("func _ready() -> void:\n\tprint(value)");
    h.place_caret(1, 3);
    h.feed("i");
    assert_eq!(h.caret(), (1, 3), "i inserts before the cursor character");
    h.feed("X<Esc>");
    assert_eq!(h.line(1), "\tprXint(value)");
    assert_eq!(h.caret(), (1, 3));

    h.feed("a");
    assert_eq!(h.caret(), (1, 4), "a inserts after the cursor character");
    h.feed("Y<Esc>");
    assert_eq!(h.line(1), "\tprXYint(value)");

    // Back into the statement from its end, as the report describes.
    h.feed("$b");
    assert_eq!(h.caret(), (1, 9));
    h.feed("iZ<Esc>");
    assert_eq!(h.line(1), "\tprXYint(Zvalue)");
    assert_eq!(h.caret(), (1, 9));
}

#[test]
fn issue_13_new_lines_and_changes_land_where_the_cursor_is() {
    let mut h = Harness::new("func _ready() -> void:\n\tprint(value)");
    h.place_caret(1, 7);
    h.feed("cwamount<Esc>");
    assert_eq!(h.line(1), "\tprint(amount)");

    h.feed("0fprP");
    assert_eq!(h.line(1), "\tPrint(amount)");

    // Indent comes from the host's indent provider, which MockHost leaves
    // out, so only where the text goes is checked.
    h.feed("otwo<Esc>");
    assert_eq!(h.line(2).trim_start(), "two");
    assert_eq!(h.caret(), (2, h.line(2).chars().count() as i32 - 1));

    h.feed("hi<CR><Esc>");
    assert_eq!(h.line(2).trim_start(), "t");
    assert_eq!(h.line(3).trim_start(), "wo", "Enter splits at the caret");
    assert_eq!(h.mode(), Mode::Normal);
}

#[test]
fn issue_13_typing_after_an_auto_closed_bracket_stays_inside_it() {
    let mut h = Harness::with_auto_brace("func _ready() -> void:\n\tpass", GODOT_PAIRS);
    h.place_caret(1, 4);
    h.feed("A");

    h.feed("(");
    assert_eq!(h.line(1), "\tpass()");
    assert_eq!(h.caret(), (1, 6), "between the pair");
    h.feed("x");
    assert_eq!(h.line(1), "\tpass(x)");
    assert_eq!(h.caret(), (1, 7));
    h.feed(")");
    assert_eq!(h.line(1), "\tpass(x)", "the closer is typed over");
    assert_eq!(h.caret(), (1, 8));

    h.feed("\"");
    assert_eq!(h.line(1), "\tpass(x)\"\"");
    h.feed("y\"");
    assert_eq!(h.line(1), "\tpass(x)\"y\"");
    assert_eq!(h.caret(), (1, 11));
}
