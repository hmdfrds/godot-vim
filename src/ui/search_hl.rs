//! Search match highlighting — delegates to CodeEdit's built-in search API.
//!
//! Uses `editor.set_search_text(pattern)` to leverage CodeEdit's native search
//! result highlighting (yellow background). Dirty-checks via `last_pattern` to
//! avoid redundant calls.
//!
//! # Regex filtering
//!
//! CodeEdit's search is literal text only (no regex). Patterns containing regex
//! metacharacters are filtered out to avoid misleading highlights (e.g. searching
//! for `/foo.*bar` would otherwise highlight the literal string "foo.*bar"
//! instead of the regex match). No highlight is better than a wrong highlight.
//!
//! **Exception**: Vim word-boundary patterns (`\<word\>`) produced by `*` and `#`
//! are unwrapped to extract the literal word, since these are the most common
//! search operations and the inner word is a plain literal. When a word-boundary
//! pattern is detected, `SEARCH_WHOLE_WORDS` is set via `set_search_flags` so
//! Godot's highlight matches Vim's whole-word semantics.
//!
//! A future custom renderer could support full regex highlighting using per-match
//! ranges from the engine's `Effect::HighlightMatches`.

use compact_str::CompactString;
use godot::classes::CodeEdit;
use godot::prelude::*;

use vim_core::primitives::Direction;

use crate::bridge::godot_calls;

/// Manages search match highlighting on a `CodeEdit`.
///
/// Wraps `editor.set_search_text()` with dirty-flag caching to skip
/// redundant FFI calls. Not a GodotClass -- no Godot lifecycle needed.
pub(crate) struct SearchHighlighter {
    last_pattern: Option<String>,
    /// Mirrors the `set_search_flags` state on the editor to avoid redundant calls.
    last_whole_word: bool,
}

impl SearchHighlighter {
    #[must_use]
    pub(crate) fn new() -> Self {
        Self {
            last_pattern: None,
            last_whole_word: false,
        }
    }

    /// Sync the editor's search highlight with the engine's current pattern.
    ///
    /// `hlsearch_enabled` is `false` after `:noh` -- highlights are cleared
    /// even though the underlying search pattern still exists (matching Vim's
    /// behavior where `:noh` hides highlights until the next search).
    pub(crate) fn update(
        &mut self,
        editor: &mut Gd<CodeEdit>,
        search_pattern: Option<&(CompactString, Direction)>,
        hlsearch_enabled: bool,
    ) {
        // Two cases collapse to "no highlight": (1) :noh disables hlsearch,
        // (2) pattern contains regex metacharacters that CodeEdit would match
        // literally. In both cases, showing nothing beats showing wrong matches.
        let (new_pattern, whole_word) = if hlsearch_enabled {
            match search_pattern
                .map(|(p, _)| p.as_str())
                .and_then(extract_literal_pattern)
            {
                Some((pat, ww)) => (Some(pat), ww),
                None => (None, false),
            }
        } else {
            (None, false)
        };
        let old_pattern = self.last_pattern.as_deref();

        if whole_word != self.last_whole_word {
            let flags: u32 = if whole_word {
                godot_calls::SEARCH_WHOLE_WORDS
            } else {
                0
            };
            godot_calls::set_search_flags(editor, flags);
            self.last_whole_word = whole_word;
        }

        if new_pattern == old_pattern {
            return;
        }
        log::trace!(
            "search_hl: pattern changed to {} (whole_word={})",
            new_pattern.unwrap_or("<none>"),
            whole_word
        );

        match new_pattern {
            Some(pattern) => {
                godot_calls::set_search_text(editor, pattern);
                self.last_pattern = Some(pattern.to_owned());
            }
            None => {
                godot_calls::set_search_text(editor, "");
                self.last_pattern = None;
            }
        }
    }

    /// Clear highlights and reset dirty-tracking state (used on detach).
    pub(crate) fn clear(&mut self, editor: &mut Gd<CodeEdit>) {
        if self.last_pattern.is_some() {
            godot_calls::set_search_text(editor, "");
            self.last_pattern = None;
        }
        if self.last_whole_word {
            godot_calls::set_search_flags(editor, 0);
            self.last_whole_word = false;
        }
    }
}

/// Try to extract a literal string from the pattern that can be used with
/// CodeEdit's literal `set_search_text` without producing misleading highlights.
///
/// Returns `Some((literal, whole_word))` if the pattern is either:
/// - A plain literal string (no regex metacharacters) → `whole_word = false`, or
/// - A Vim word-boundary pattern `\<word\>` wrapping a plain literal
///   (produced by `*` and `#` commands) → `whole_word = true`.
///
/// Returns `None` if the pattern contains regex metacharacters that would
/// cause CodeEdit's literal search to highlight wrong text.
fn extract_literal_pattern(pattern: &str) -> Option<(&str, bool)> {
    // `*` and `#` wrap the word in `\<...\>` boundaries. Strip them and
    // enable SEARCH_WHOLE_WORDS so Godot's literal search matches Vim semantics.
    let inner = pattern
        .strip_prefix("\\<")
        .and_then(|s| s.strip_suffix("\\>"));
    if let Some(word) = inner {
        if !contains_regex_metacharacters(word) {
            return Some((word, true));
        }
        return None;
    }

    // Reject any pattern with metacharacters -- CodeEdit matches literally.
    if contains_regex_metacharacters(pattern) {
        return None;
    }
    Some((pattern, false))
}

/// Returns `true` if `pattern` contains any regex metacharacters that would
/// cause CodeEdit's literal `set_search_text` to produce misleading highlights.
///
/// Covers both standard regex syntax and Vim's `magic` mode metacharacters.
fn contains_regex_metacharacters(pattern: &str) -> bool {
    pattern.contains([
        '\\', '.', '^', '$', '*', '+', '?', '(', ')', '[', ']', '{', '}', '|',
    ])
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── Plain literal strings (no metacharacters) ───────────────────────

    #[test]
    fn plain_word_has_no_metacharacters() {
        assert!(!contains_regex_metacharacters("hello"));
    }

    #[test]
    fn plain_sentence_has_no_metacharacters() {
        assert!(!contains_regex_metacharacters("hello world"));
    }

    #[test]
    fn digits_have_no_metacharacters() {
        assert!(!contains_regex_metacharacters("12345"));
    }

    #[test]
    fn empty_string_has_no_metacharacters() {
        assert!(!contains_regex_metacharacters(""));
    }

    #[test]
    fn underscores_and_dashes_are_not_metacharacters() {
        assert!(!contains_regex_metacharacters("foo_bar-baz"));
    }

    #[test]
    fn slashes_are_not_metacharacters() {
        assert!(!contains_regex_metacharacters("path/to/file"));
    }

    #[test]
    fn at_sign_hash_percent_are_not_metacharacters() {
        assert!(!contains_regex_metacharacters("@#%!&"));
    }

    // ── Individual metacharacters ───────────────────────────────────────

    #[test]
    fn backslash_detected() {
        assert!(contains_regex_metacharacters("foo\\bar"));
    }

    #[test]
    fn dot_detected() {
        assert!(contains_regex_metacharacters("foo.bar"));
    }

    #[test]
    fn caret_detected() {
        assert!(contains_regex_metacharacters("^start"));
    }

    #[test]
    fn dollar_detected() {
        assert!(contains_regex_metacharacters("end$"));
    }

    #[test]
    fn asterisk_detected() {
        assert!(contains_regex_metacharacters("foo*"));
    }

    #[test]
    fn plus_detected() {
        assert!(contains_regex_metacharacters("foo+"));
    }

    #[test]
    fn question_mark_detected() {
        assert!(contains_regex_metacharacters("foo?"));
    }

    #[test]
    fn open_paren_detected() {
        assert!(contains_regex_metacharacters("foo(bar"));
    }

    #[test]
    fn close_paren_detected() {
        assert!(contains_regex_metacharacters("foo)bar"));
    }

    #[test]
    fn open_bracket_detected() {
        assert!(contains_regex_metacharacters("[abc]"));
    }

    #[test]
    fn close_bracket_detected() {
        assert!(contains_regex_metacharacters("abc]"));
    }

    #[test]
    fn open_brace_detected() {
        assert!(contains_regex_metacharacters("a{3}"));
    }

    #[test]
    fn close_brace_detected() {
        assert!(contains_regex_metacharacters("a}"));
    }

    #[test]
    fn pipe_detected() {
        assert!(contains_regex_metacharacters("foo|bar"));
    }

    // ── Common regex patterns ───────────────────────────────────────────

    #[test]
    fn vim_word_boundary_pattern() {
        assert!(contains_regex_metacharacters("\\<word\\>"));
    }

    #[test]
    fn wildcard_dot_star() {
        assert!(contains_regex_metacharacters("foo.*bar"));
    }

    #[test]
    fn character_class() {
        assert!(contains_regex_metacharacters("[a-z]+"));
    }

    #[test]
    fn anchored_pattern() {
        assert!(contains_regex_metacharacters("^hello$"));
    }

    #[test]
    fn alternation_pattern() {
        assert!(contains_regex_metacharacters("cat|dog"));
    }

    #[test]
    fn grouping_pattern() {
        assert!(contains_regex_metacharacters("(foo|bar)"));
    }

    #[test]
    fn quantifier_braces() {
        assert!(contains_regex_metacharacters("a{2,5}"));
    }

    // ── Edge cases ──────────────────────────────────────────────────────

    #[test]
    fn single_metacharacter_only() {
        assert!(contains_regex_metacharacters("."));
        assert!(contains_regex_metacharacters("*"));
        assert!(contains_regex_metacharacters("\\"));
        assert!(contains_regex_metacharacters("|"));
    }

    #[test]
    fn metacharacter_at_start() {
        assert!(contains_regex_metacharacters("^foo"));
    }

    #[test]
    fn metacharacter_at_end() {
        assert!(contains_regex_metacharacters("foo$"));
    }

    #[test]
    fn metacharacter_in_middle() {
        assert!(contains_regex_metacharacters("fo.o"));
    }

    #[test]
    fn unicode_without_metacharacters() {
        assert!(!contains_regex_metacharacters("日本語"));
    }

    #[test]
    fn unicode_with_metacharacters() {
        assert!(contains_regex_metacharacters("日.*語"));
    }

    #[test]
    fn single_non_meta_char() {
        assert!(!contains_regex_metacharacters("a"));
    }

    // ── extract_literal_pattern ────────────────────────────────────────

    #[test]
    fn extract_plain_literal() {
        assert_eq!(extract_literal_pattern("hello"), Some(("hello", false)));
    }

    #[test]
    fn extract_rejects_regex() {
        assert_eq!(extract_literal_pattern("foo.*bar"), None);
    }

    #[test]
    fn extract_word_boundary_pattern() {
        assert_eq!(extract_literal_pattern("\\<word\\>"), Some(("word", true)));
    }

    #[test]
    fn extract_word_boundary_with_underscores() {
        assert_eq!(
            extract_literal_pattern("\\<my_var\\>"),
            Some(("my_var", true))
        );
    }

    #[test]
    fn extract_word_boundary_with_digits() {
        assert_eq!(
            extract_literal_pattern("\\<foo123\\>"),
            Some(("foo123", true))
        );
    }

    #[test]
    fn extract_word_boundary_with_regex_inside() {
        assert_eq!(extract_literal_pattern("\\<foo.*bar\\>"), None);
    }

    #[test]
    fn extract_only_prefix_no_suffix() {
        assert_eq!(extract_literal_pattern("\\<word"), None);
    }

    #[test]
    fn extract_only_suffix_no_prefix() {
        assert_eq!(extract_literal_pattern("word\\>"), None);
    }

    #[test]
    fn extract_empty_word_boundary() {
        assert_eq!(extract_literal_pattern("\\<\\>"), Some(("", true)));
    }

    #[test]
    fn extract_empty_string() {
        assert_eq!(extract_literal_pattern(""), Some(("", false)));
    }
}
