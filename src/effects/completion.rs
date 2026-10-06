//! Keeps Godot's completion popup derived from the live text after an engine
//! edit. CodeEdit re-filters only inside its own key handler
//! (`_filter_code_completion_candidates_impl`, code_edit.cpp), which never
//! runs for text the engine types.

use crate::bridge::port::IdeCapable;

/// What one engine pass did, seen from the main caret.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Edit {
    None,
    /// Text changed and the caret advanced; this is the character it now
    /// follows. Typing, Replace mode, a flushed mapping, a register paste.
    Inserted(char),
    /// The caret moved back over exactly one removed character.
    Erased,
    /// Any other text change, or a caret-only move.
    Other,
}

/// The caret as a byte offset, with the character just before it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Caret {
    pub(crate) offset: usize,
    pub(crate) prev: Option<char>,
}

impl Caret {
    pub(crate) fn of(text: &str, offset: usize) -> Self {
        Self {
            offset,
            prev: text.get(..offset).and_then(|s| s.chars().next_back()),
        }
    }
}

impl Edit {
    pub(crate) fn classify(text_changed: bool, before: Caret, after: Caret) -> Self {
        if !text_changed {
            return if before.offset == after.offset {
                Self::None
            } else {
                Self::Other
            };
        }
        match (before.prev, after.prev) {
            (_, Some(c)) if after.offset > before.offset => Self::Inserted(c),
            (Some(c), _) if after.offset + c.len_utf8() == before.offset => Self::Erased,
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

/// Non-word characters close the popup, except `/` in an open one: Godot
/// counts it as part of the word (code_edit.cpp:3933), and only a forced
/// request re-queries after a symbol.
pub(crate) fn follow(
    edit: Edit,
    popup_open: bool,
    auto_complete: bool,
    is_prefix: impl Fn(char) -> bool,
) -> Follow {
    let may_open = popup_open || auto_complete;
    match edit {
        Edit::None => Follow::Keep,
        Edit::Inserted(c) if c.is_alphanumeric() || c == '_' || is_prefix(c) => {
            if may_open {
                Follow::Requery { force: false }
            } else {
                Follow::Keep
            }
        }
        Edit::Inserted('/') if popup_open => Follow::Requery { force: true },
        Edit::Inserted(_) if popup_open => Follow::Close,
        Edit::Erased if may_open => Follow::Requery { force: false },
        Edit::Other if popup_open => Follow::Requery { force: false },
        Edit::Inserted(_) | Edit::Erased | Edit::Other => Follow::Keep,
    }
}

/// Runs once at the end of every engine pass, whatever started it (a typed
/// key, a mapping-timeout flush). Goes to the raw editor, never
/// `CompletionPort`, whose writes mark the selection explicit.
pub(crate) fn follow_engine_edit(
    editor: &mut impl IdeCapable,
    edit: Edit,
    insert_like: bool,
    auto_complete: bool,
) {
    if !insert_like || edit == Edit::None {
        return;
    }
    let popup_open = editor.completion_popup_open();
    match follow(edit, popup_open, auto_complete, |c| {
        editor.is_completion_prefix(c)
    }) {
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
                Edit::Inserted('A'),
            ),
            (
                "typed a multibyte letter",
                true,
                at("x", 1),
                at("xé", 3),
                Edit::Inserted('é'),
            ),
            (
                "an auto-pair leaves the caret after the opener",
                true,
                at("f", 1),
                at("f()", 2),
                Edit::Inserted('('),
            ),
            (
                "Replace mode overwrites in place",
                true,
                at("abc", 1),
                at("aXc", 2),
                Edit::Inserted('X'),
            ),
            (
                "a newline",
                true,
                at("\t$A", 3),
                at("\t$A\n", 4),
                Edit::Inserted('\n'),
            ),
            ("backspace", true, at("$An", 3), at("$A", 2), Edit::Erased),
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
                Edit::Inserted('i'),
            ),
            (
                "a flushed mapping prefix and the key after it",
                true,
                at("$P", 2),
                at("$Pju", 4),
                Edit::Inserted('u'),
            ),
            (
                "text changed away from the caret",
                true,
                at("ab", 2),
                at("xab", 2),
                Edit::Other,
            ),
        ];
        for (what, changed, before, after, want) in rows {
            assert_eq!(Edit::classify(*changed, *before, *after), *want, "{what}");
        }
    }

    fn prefix(c: char) -> bool {
        MockTextEdit::new("").is_completion_prefix(c)
    }

    #[test]
    fn follow_is_a_total_table() {
        use Follow::{Close, Keep, Requery};
        const SOFT: Follow = Requery { force: false };
        // (edit, popup open, auto-complete on, expected)
        let rows: &[(&str, Edit, bool, bool, Follow)] = &[
            ("no edit", Edit::None, true, true, Keep),
            ("a word char narrows", Edit::Inserted('n'), true, true, SOFT),
            ("a word char opens", Edit::Inserted('n'), false, true, SOFT),
            (
                "a prefix char opens",
                Edit::Inserted('$'),
                false,
                true,
                SOFT,
            ),
            (
                "auto-complete off opens nothing",
                Edit::Inserted('n'),
                false,
                false,
                Keep,
            ),
            (
                "a Ctrl+Space popup narrows with auto-complete off",
                Edit::Inserted('n'),
                true,
                false,
                SOFT,
            ),
            (
                "a non-word char closes",
                Edit::Inserted(';'),
                true,
                true,
                Close,
            ),
            ("a space closes", Edit::Inserted(' '), true, true, Close),
            ("a newline closes", Edit::Inserted('\n'), true, true, Close),
            (
                "a slash keeps a path popup",
                Edit::Inserted('/'),
                true,
                true,
                Requery { force: true },
            ),
            (
                "a slash opens nothing",
                Edit::Inserted('/'),
                false,
                true,
                Keep,
            ),
            (
                "a non-word char with no popup",
                Edit::Inserted(';'),
                false,
                true,
                Keep,
            ),
            ("backspace widens", Edit::Erased, true, false, SOFT),
            ("backspace opens", Edit::Erased, false, true, SOFT),
            (
                "backspace with everything off",
                Edit::Erased,
                false,
                false,
                Keep,
            ),
            (
                "<C-w> or a caret move re-derive",
                Edit::Other,
                true,
                false,
                SOFT,
            ),
            ("<C-w> opens nothing", Edit::Other, false, true, Keep),
        ];
        for (what, edit, open, auto, want) in rows {
            assert_eq!(follow(*edit, *open, *auto, prefix), *want, "{what}");
        }
    }

    #[test]
    fn a_requery_cancels_first_and_never_runs_outside_insert() {
        let mut ed = MockTextEdit::new("");
        ed.popup_open = true;
        follow_engine_edit(&mut ed, Edit::Inserted('n'), true, false);
        assert_eq!(ed.completion_log, ["cancel", "request(force=false)"]);

        let mut ed = MockTextEdit::new("");
        ed.popup_open = true;
        follow_engine_edit(&mut ed, Edit::Inserted('n'), false, true);
        assert!(ed.completion_log.is_empty());
    }
}
