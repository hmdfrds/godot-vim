//! The undo store's rules end to end: keys through the real engine and the
//! real dispatch (`MockHost`), `u` checked against the text.

use super::mock_host::Harness;
use vim_core::primitives::Mode;

/// A register's keys are not typed, so `@q` syncs undo once
/// (getchar.c `gotchars()`, `may_sync_undo()`): one `u` undoes the replay.
#[test]
fn u_after_a_macro_undoes_the_replay() {
    let mut h = Harness::new("abc\ndef\nghi");
    h.feed("qqxjq@q");
    assert_eq!(h.text(), "bc\nef\nghi");

    h.feed("u");
    assert_eq!(h.text(), "bc\ndef\nghi");
    assert_eq!(h.caret(), (1, 0));
    h.feed("u");
    assert_eq!(h.text(), "abc\ndef\nghi");
    assert_eq!(h.orphaned_undo_groups(), 0);
}

#[test]
fn u_after_a_counted_macro_undoes_every_replay() {
    let mut h = Harness::new("abc\ndef\nghi\njkl\nmno");
    h.feed("qqxjq3@q");
    assert_eq!(h.text(), "bc\nef\nhi\nkl\nmno");

    h.feed("u");
    assert_eq!(h.text(), "bc\ndef\nghi\njkl\nmno");
    assert_eq!(h.orphaned_undo_groups(), 0);
}

/// The replay reports the node it did not create; its snapshot stays.
#[test]
fn u_after_a_macro_that_only_moves_undoes_the_change_before_it() {
    let mut h = Harness::new("abc\ndef\nghi");
    h.feed("xqqjq@q");
    assert_eq!(h.caret(), (2, 0));

    h.feed("u");
    assert_eq!(h.text(), "abc\ndef\nghi");
    assert_eq!(h.caret(), (0, 0));
}

/// Insert `<C-o>` runs one command in Normal while the Insert group stays
/// open: the host keeps the session's start text.
#[test]
fn insert_ctrl_o_keeps_the_session() {
    let mut h = Harness::new("abc");
    h.feed("lifoo<C-o>");
    assert_eq!(h.mode(), Mode::Normal);
    assert_eq!(h.orphaned_undo_groups(), 0);

    h.feed("xbar<Esc>");
    assert_eq!(h.text(), "afoobarc");
    assert_eq!(h.orphaned_undo_groups(), 0);
}

/// `:norm` reuses node 1's id after `:set ul=1` pruned it, for a `guu`
/// that changed nothing: `u` must not replay the old node's `x`.
#[test]
fn recycled_id_is_not_kept() {
    let mut h = Harness::new("abc");
    h.feed("xu");
    h.feed(":set ul=1<CR>");
    h.feed(":norm xguu<CR>");
    assert_eq!(h.text(), "bc");

    h.feed("u");
    assert_eq!(h.text(), "bc", "the guu node changes nothing");
    h.feed("u");
    assert_eq!(h.text(), "abc");
    assert_eq!(h.orphaned_undo_groups(), 0);
}

/// `i<Esc>` makes a node that changes nothing; its `u` must push no Godot
/// step, or Godot chains the second `x` to the first and Ctrl+Z takes both.
#[test]
fn identity_undo_pushes_no_godot_step() {
    let mut h = Harness::new("abc");
    h.feed("xxi<Esc>u");
    assert_eq!(h.text(), "c");

    h.godot_undo();
    assert_eq!(h.text(), "bc", "only the second x undone");
    assert_eq!(h.editor().empty_complex_pairs(), 0);
}

/// The same for `<C-r>` of a node that changes nothing.
#[test]
fn identity_redo_pushes_no_godot_step() {
    let mut h = Harness::new("abc");
    h.feed("xxi<Esc>u<C-r>");
    assert_eq!(h.text(), "c");

    h.godot_undo();
    assert_eq!(h.text(), "bc", "only the second x undone");
    assert_eq!(h.editor().empty_complex_pairs(), 0);
}

/// A pass that opens or closes a Vim undo group, or moves through the tree,
/// ends Godot's edit action after its commit; a motion does not.
#[test]
fn boundary_closes_godot_action() {
    let mut h = Harness::new("abc\ndef");
    for (keys, closes) in [
        ("l", 0),
        ("x", 1),
        ("i", 1),
        ("z", 0),
        ("<Esc>", 1),
        ("u", 1),
        ("<C-r>", 1),
        ("j", 0),
    ] {
        let before = h.editor().godot_action_closes();
        h.feed(keys);
        assert_eq!(h.editor().godot_action_closes() - before, closes, "{keys}");
    }
    assert_eq!(h.text(), "azc\ndef");
}
