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
