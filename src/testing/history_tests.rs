//! Regression tests for bugs users reported, one section per issue. Keys go
//! through the real engine and the real dispatch (`MockHost`), so each test
//! fails if either side brings its bug back.

use super::mock_host::Harness;
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
