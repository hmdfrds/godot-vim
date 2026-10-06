//! Keeps Godot's completion popup derived from the live text after an engine
//! edit. CodeEdit re-filters only inside its own key handler
//! (`_filter_code_completion_candidates_impl`, code_edit.cpp), which never
//! runs for text the engine types.

use crate::bridge::port::IdeCapable;

/// What one engine pass did, seen from the main caret.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Edit {
    None,
    /// Text was added at the caret; the line before where it began is intact.
    Inserted,
    /// The caret moved back over exactly one removed character.
    Erased,
    /// Any other text change, or a caret-only move.
    Other,
}

/// The caret as a byte offset, with its line's text up to it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Caret {
    offset: usize,
    head: String,
}

impl Caret {
    pub(crate) fn of(text: &str, offset: usize) -> Self {
        let before = text.get(..offset).unwrap_or(text);
        let start = before.rfind('\n').map_or(0, |i| i + 1);
        Self {
            offset,
            head: before[start..].to_owned(),
        }
    }

    fn prev(&self) -> Option<char> {
        self.head.chars().next_back()
    }
}

impl Edit {
    /// Compares line heads, not offsets, so an indent shift (`<C-t>`, `<C-d>`)
    /// that moves the caret is not mistaken for typing.
    pub(crate) fn classify(text_changed: bool, before: &Caret, after: &Caret) -> Self {
        if !text_changed {
            return if before.offset == after.offset {
                Self::None
            } else {
                Self::Other
            };
        }
        if after
            .head
            .strip_prefix(before.head.as_str())
            .is_some_and(|added| !added.is_empty())
        {
            return Self::Inserted;
        }
        match before.prev() {
            Some(c) if before.head[..before.head.len() - c.len_utf8()] == after.head => {
                Self::Erased
            }
            _ => Self::Other,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Follow {
    Keep,
    Close,
    /// Cancel, then request: Godot ignores a request while the open popup
    /// holds only SIGNAL / NODE_PATH / FILE_PATH options (code_edit.cpp
    /// `request_code_completion`), and the cancel lifts that guard.
    Requery {
        force: bool,
    },
}

/// A typed character keeps the popup when Godot's completion word continues
/// across it: anything inside a string (code_edit.cpp:3920), and `/` in a path
/// popup (code_edit.cpp:3933). `prev` is the character now before the caret.
pub(crate) fn follow(
    edit: Edit,
    prev: Option<char>,
    auto_complete: bool,
    editor: &impl IdeCapable,
) -> Follow {
    let open = editor.completion_popup_open();
    let may_open = open || auto_complete;
    match edit {
        Edit::None => Follow::Keep,
        Edit::Inserted => {
            let continues = prev.is_some_and(|c| {
                c.is_alphanumeric()
                    || c == '_'
                    || editor.is_completion_prefix(c)
                    || editor.caret_in_string()
                    || (open && after_path_slash(Some(c), editor))
            });
            if continues {
                if may_open {
                    requery(open, prev, editor)
                } else {
                    Follow::Keep
                }
            } else if open {
                Follow::Close
            } else {
                Follow::Keep
            }
        }
        Edit::Erased if may_open => requery(open, prev, editor),
        Edit::Other if open => requery(open, prev, editor),
        Edit::Erased | Edit::Other => Follow::Keep,
    }
}

fn after_path_slash(prev: Option<char>, editor: &impl IdeCapable) -> bool {
    prev == Some('/') && editor.completion_popup_holds_paths()
}

/// An unforced request emits nothing after a symbol outside a string
/// (code_edit.cpp:2505-2510), so only `/` in a path popup needs the force.
fn requery(open: bool, prev: Option<char>, editor: &impl IdeCapable) -> Follow {
    Follow::Requery {
        force: open && after_path_slash(prev, editor) && !editor.caret_in_string(),
    }
}

/// Runs once at the end of every engine pass, whatever started it (a typed
/// key, a mapping-timeout flush). Goes to the raw editor, never
/// `CompletionPort`, whose writes mark the selection explicit.
pub(crate) fn follow_engine_edit(
    editor: &mut impl IdeCapable,
    edit: Edit,
    after: &Caret,
    insert_like: bool,
    auto_complete: bool,
) {
    if !insert_like || edit == Edit::None {
        return;
    }
    match follow(edit, after.prev(), auto_complete, editor) {
        Follow::Keep => {}
        Follow::Close => editor.cancel_code_completion(),
        Follow::Requery { force } => {
            editor.cancel_code_completion();
            editor.request_code_completion(force);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::MockTextEdit;

    fn at(text: &str, offset: usize) -> Caret {
        Caret::of(text, offset)
    }

    #[test]
    fn classify_is_a_total_table() {
        let rows: &[(&str, bool, Caret, Caret, Edit)] = &[
            ("nothing moved", false, at("ab", 2), at("ab", 2), Edit::None),
            (
                "caret-only move",
                false,
                at("ab", 2),
                at("ab", 1),
                Edit::Other,
            ),
            (
                "typed a letter",
                true,
                at("$", 1),
                at("$A", 2),
                Edit::Inserted,
            ),
            (
                "typed a multibyte letter",
                true,
                at("x", 1),
                at("xé", 3),
                Edit::Inserted,
            ),
            (
                "an auto-pair leaves the caret after the opener",
                true,
                at("f", 1),
                at("f()", 2),
                Edit::Inserted,
            ),
            (
                "Replace mode overwrites in place",
                true,
                at("abc", 1),
                at("aXc", 2),
                Edit::Inserted,
            ),
            (
                "typing on a later line",
                true,
                at("a\n$", 3),
                at("a\n$A", 4),
                Edit::Inserted,
            ),
            (
                "a newline",
                true,
                at("\t$A", 3),
                at("\t$A\n\t", 5),
                Edit::Other,
            ),
            ("backspace", true, at("$An", 3), at("$A", 2), Edit::Erased),
            (
                "backspace over a multibyte letter",
                true,
                at("xé", 3),
                at("x", 1),
                Edit::Erased,
            ),
            (
                "a word erased",
                true,
                at("$Ani", 4),
                at("$", 1),
                Edit::Other,
            ),
            (
                "a register pasted",
                true,
                at("$", 1),
                at("$Ani", 4),
                Edit::Inserted,
            ),
            (
                "a flushed mapping prefix and the key after it",
                true,
                at("$P", 2),
                at("$Pju", 4),
                Edit::Inserted,
            ),
            (
                "text changed away from the caret",
                true,
                at("ab", 2),
                at("xab", 3),
                Edit::Other,
            ),
            (
                "<C-t> shifts the line under the caret",
                true,
                at("\tvar a = pri", 12),
                at("    \tvar a = pri", 16),
                Edit::Other,
            ),
            (
                "<C-d> shifts the line under the caret",
                true,
                at("\t\tvar a = pri", 13),
                at("\tvar a = pri", 12),
                Edit::Other,
            ),
        ];
        for (what, changed, before, after, want) in rows {
            assert_eq!(Edit::classify(*changed, before, after), *want, "{what}");
        }
    }

    #[derive(Clone, Copy)]
    enum Popup {
        Closed,
        Words,
        Paths,
    }

    fn editor(popup: Popup, in_string: bool) -> MockTextEdit {
        let mut ed = MockTextEdit::new("");
        ed.popup_open = !matches!(popup, Popup::Closed);
        ed.popup_paths = matches!(popup, Popup::Paths);
        ed.in_string = in_string;
        ed
    }

    #[test]
    fn follow_is_a_total_table() {
        use Edit::{Erased, Inserted, Other};
        use Follow::{Close, Keep, Requery};
        use Popup::{Closed, Paths, Words};
        const SOFT: Follow = Requery { force: false };
        const FORCE: Follow = Requery { force: true };
        // (edit, char now before the caret, popup, in a string, auto-complete on, expected)
        let rows: &[(&str, Edit, char, Popup, bool, bool, Follow)] = &[
            ("no edit", Edit::None, 'n', Words, false, true, Keep),
            (
                "a word char narrows",
                Inserted,
                'n',
                Words,
                false,
                true,
                SOFT,
            ),
            (
                "a word char opens",
                Inserted,
                'n',
                Closed,
                false,
                true,
                SOFT,
            ),
            (
                "a prefix char opens",
                Inserted,
                '$',
                Closed,
                false,
                true,
                SOFT,
            ),
            (
                "auto-complete off opens nothing",
                Inserted,
                'n',
                Closed,
                false,
                false,
                Keep,
            ),
            (
                "a Ctrl+Space popup narrows with auto-complete off",
                Inserted,
                'n',
                Words,
                false,
                false,
                SOFT,
            ),
            (
                "a non-word char closes",
                Inserted,
                ';',
                Words,
                false,
                true,
                Close,
            ),
            ("a space closes", Inserted, ' ', Words, false, true, Close),
            (
                "a slash keeps a node path popup",
                Inserted,
                '/',
                Paths,
                false,
                true,
                FORCE,
            ),
            (
                "a slash is division in an identifier popup",
                Inserted,
                '/',
                Words,
                false,
                true,
                Close,
            ),
            (
                "a slash opens nothing",
                Inserted,
                '/',
                Closed,
                false,
                true,
                Keep,
            ),
            (
                "a non-word char with no popup",
                Inserted,
                ';',
                Closed,
                false,
                true,
                Keep,
            ),
            (
                "any char continues a string popup",
                Inserted,
                '-',
                Paths,
                true,
                false,
                SOFT,
            ),
            (
                "a slash in a string needs no force",
                Inserted,
                '/',
                Paths,
                true,
                true,
                SOFT,
            ),
            (
                "any char in a string opens",
                Inserted,
                ':',
                Closed,
                true,
                true,
                SOFT,
            ),
            (
                "a char in a string opens nothing with auto-complete off",
                Inserted,
                ':',
                Closed,
                true,
                false,
                Keep,
            ),
            ("backspace widens", Erased, 'A', Paths, false, false, SOFT),
            ("backspace opens", Erased, 'A', Closed, false, true, SOFT),
            (
                "backspace onto a slash keeps a node path popup",
                Erased,
                '/',
                Paths,
                false,
                false,
                FORCE,
            ),
            (
                "backspace with everything off",
                Erased,
                'A',
                Closed,
                false,
                false,
                Keep,
            ),
            (
                "<C-w> or a caret move re-derive",
                Other,
                'A',
                Words,
                false,
                false,
                SOFT,
            ),
            (
                "<C-w> onto a slash keeps a node path popup",
                Other,
                '/',
                Paths,
                false,
                false,
                FORCE,
            ),
            ("<C-w> opens nothing", Other, 'A', Closed, false, true, Keep),
        ];
        for (what, edit, prev, popup, in_string, auto, want) in rows {
            let ed = editor(*popup, *in_string);
            assert_eq!(follow(*edit, Some(*prev), *auto, &ed), *want, "{what}");
        }
    }

    #[test]
    fn a_requery_cancels_first_and_never_runs_outside_insert() {
        let typed = at("$An", 3);
        let mut ed = editor(Popup::Paths, false);
        follow_engine_edit(&mut ed, Edit::Inserted, &typed, true, false);
        assert_eq!(ed.completion_log, ["cancel", "request(force=false)"]);

        let mut ed = editor(Popup::Paths, false);
        follow_engine_edit(&mut ed, Edit::Inserted, &typed, false, true);
        assert!(ed.completion_log.is_empty());
    }
}
