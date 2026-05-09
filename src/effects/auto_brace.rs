//! Auto-brace completion for insert mode.
//!
//! Reimplements Godot's `CodeEdit::_handle_unicode_input_internal` auto-brace
//! logic (code_edit.cpp:770-807) using CodeEdit's bound query APIs. This is
//! necessary because `handle_unicode_input` is not callable from gdext (it's
//! not registered via `ClassDB::bind_method`).
//!
//! The decision tree and helper functions (`find_close_pair_at_pos`,
//! `find_open_pair_at_pos`, `is_symbol`) are direct ports of Godot's C++
//! implementation, using the same ordering and short-circuit logic.

use std::rc::Rc;

use crate::bridge::codec::{i32_to_usize, usize_to_i32, DocumentView};
use crate::bridge::port::TextEditorPort;
use crate::bridge::{AutoBraceSnapshot, SyntaxRegion};
use crate::effects::text::insert_at;

/// Result of an auto-brace insert operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use]
pub(super) enum AutoBraceResult {
    /// Text was inserted (possibly with auto-close). Invalidates text cache.
    Inserted,
    /// Skip-over: no text was inserted, cursor was moved past existing close
    /// brace. Does NOT invalidate text cache.
    SkippedOver,
}

/// Handle a single-character insert with auto-brace completion.
///
/// Mirrors the no-selection branch of `CodeEdit::_handle_unicode_input_internal`
/// (code_edit.cpp:782-803). Returns whether text was actually modified.
///
/// Preconditions:
/// - `editor.is_auto_brace_completion_enabled()` is `true`
/// - `ch` is a printable character (not a control char like `\n` or `\t`).
///   Godot's `_handle_unicode_input_internal` never receives control characters;
///   the dispatcher must enforce this before calling.
/// - `offset` is the byte offset in `text` where the insert should occur
pub(super) fn handle_insert_with_auto_brace(
    editor: &mut impl TextEditorPort,
    doc: &DocumentView,
    offset: usize,
    ch: char,
    auto_brace: &AutoBraceSnapshot,
    syntax: &SyntaxRegion,
) -> AutoBraceResult {
    debug_assert!(
        !ch.is_control(),
        "auto-brace received control char U+{:04X} — dispatcher should filter these",
        ch as u32,
    );
    let lc = doc.line_index.byte_to_line_col(doc.text, offset);
    let line = lc.line;
    let col = lc.col;
    let pairs = Rc::clone(&auto_brace.pairs);
    let mut char_buf = [0u8; 4];
    let ch_str = ch.encode_utf8(&mut char_buf);

    let line_text = editor.get_line(line);
    let char_col = i32_to_usize(col);
    let line_char_len = line_text.chars().count();

    let post_brace_pair = if char_col < line_char_len {
        find_close_pair_at_pos_str(&pairs, &line_text, char_col)
    } else {
        None
    };

    // Branch 1: String delimiter after non-symbol char, no post_brace_pair.
    // e.g., typing `"` after `x` (non-symbol) → just insert, no auto-close.
    if auto_brace.has_string_delimiter(ch_str)
        && char_col > 0
        && !is_symbol(nth_char(&line_text, char_col - 1).unwrap_or(' '))
        && post_brace_pair.is_none()
    {
        log::trace!("auto_brace: branch=string_delimiter ch='{}'", ch);
        insert_at(editor, line, col, ch_str);
        return AutoBraceResult::Inserted;
    }

    // Branch 2: Next char is not a symbol → just insert, no auto-close.
    if char_col < line_char_len && !is_symbol(nth_char(&line_text, char_col).unwrap_or(' ')) {
        log::trace!("auto_brace: branch=non_symbol_next ch='{}'", ch);
        insert_at(editor, line, col, ch_str);
        return AutoBraceResult::Inserted;
    }

    // Branch 3: Skip-over — close brace at cursor matches the typed char.
    if let Some(pair_idx) = post_brace_pair {
        let close_key = &pairs[pair_idx].1;
        if close_key.starts_with(ch) {
            log::trace!("auto_brace: branch=skip_over ch='{}'", ch);
            let move_offset = usize_to_i32(close_key.chars().count());
            editor.set_caret_line(line);
            editor.set_caret_column(col + move_offset);
            return AutoBraceResult::SkippedOver;
        }
    }

    // Branch 4: Inside comment, or inside string and char is string delimiter.
    // Uses pre-captured syntax region from the snapshot (matching Godot's
    // code_edit.cpp:793 which passes `is_in_string(cl, char_col)`).
    if matches!(syntax, SyntaxRegion::Comment)
        || (matches!(syntax, SyntaxRegion::String) && auto_brace.has_string_delimiter(ch_str))
    {
        log::trace!("auto_brace: branch=in_comment_or_string ch='{}'", ch);
        insert_at(editor, line, col, ch_str);
        return AutoBraceResult::Inserted;
    }

    // Branch 5 (default): Insert char, then auto-close if it forms an open pair.
    log::trace!("auto_brace: branch=default_with_close ch='{}'", ch);
    insert_at(editor, line, col, ch_str);

    // Re-fetch line text after insert, then auto-close if an open pair ends
    // at the new caret position. The close key is inserted after the open key
    // (e.g., typing `(` yields `(|)`).
    let updated_line_text = editor.get_line(line);
    if let Some(pair_idx) = find_open_pair_at_pos_str(&pairs, &updated_line_text, char_col + 1) {
        let close_key = &pairs[pair_idx].1;
        editor.insert_text(close_key, line, col + 1);
    }

    editor.set_caret_column(col + 1);

    AutoBraceResult::Inserted
}

/// Handle auto-brace deletion for backspace in insert mode.
///
/// Mirrors `CodeEdit::_backspace_internal` (code_edit.cpp:856-864).
/// Called AFTER the engine's `Effect::Delete` has been applied (which already
/// deleted the character before the cursor). This function checks if the
/// deletion removed an opening brace and, if so, also deletes the matching
/// close brace at the cursor position.
///
/// `deleted_text` is the text of the document BEFORE the delete was applied.
/// `start`/`end` are the byte offsets of the deleted range.
pub(super) fn handle_delete_with_auto_brace(
    editor: &mut impl TextEditorPort,
    doc: &DocumentView,
    start: usize,
    end: usize,
    auto_brace: &AutoBraceSnapshot,
) {
    let pairs = Rc::clone(&auto_brace.pairs);
    if pairs.is_empty() {
        return;
    }

    let end_lc = doc.line_index.byte_to_line_col(doc.text, end);
    let del_line = end_lc.line;
    let end_col = end_lc.col;

    // Use the pre-delete text to check whether the deleted range was an opening
    // brace with a matching close brace immediately adjacent.
    let original_line_str = doc
        .line_index
        .line_text_at(doc.text, i32_to_usize(del_line));

    let end_char_col = i32_to_usize(end_col);

    if let Some(pair_idx) = find_open_pair_at_pos_str(&pairs, original_line_str, end_char_col) {
        let close_key = &pairs[pair_idx].1;
        let close_char_len = close_key.chars().count();
        let original_char_len = original_line_str.chars().count();

        if end_char_col + close_char_len <= original_char_len
            && chars_match_at(original_line_str, end_char_col, close_key)
        {
            // The close brace was adjacent. After the engine's delete
            // removed the open brace, the close brace shifted left. It's
            // now at the caret position. Delete it.
            let start_lc = doc.line_index.byte_to_line_col(doc.text, start);
            let start_line = start_lc.line;
            let start_col = start_lc.col;
            let post_delete_line = editor.get_line(start_line);
            let caret_char_col = i32_to_usize(start_col);
            let post_char_len = post_delete_line.chars().count();

            if caret_char_col + close_char_len <= post_char_len
                && chars_match_at(&post_delete_line, caret_char_col, close_key)
            {
                log::trace!(
                    "auto_brace_delete: removed matching close brace at line={} col={}",
                    start_line,
                    start_col
                );
                let close_end_col = start_col + usize_to_i32(close_char_len);
                editor.remove_text(start_line, start_col, start_line, close_end_col);
            }
        }
    }
}

// ── Helpers ──────────────────────────────────────────────────────────────

/// Godot's auto-brace "symbol" predicate: ASCII punctuation (excluding `_`)
/// plus whitespace. Non-ASCII punctuation is treated as a word character,
/// matching `code_edit.cpp`'s `char_utils` behavior.
fn is_symbol(ch: char) -> bool {
    (ch.is_ascii_punctuation() && ch != '_') || ch == '\t' || ch == ' '
}

#[inline]
fn nth_char(s: &str, n: usize) -> Option<char> {
    s.chars().nth(n)
}

/// Check if `needle` matches `haystack` starting at char index `col`.
fn chars_match_at(haystack: &str, col: usize, needle: &str) -> bool {
    let mut haystack_iter = haystack.chars().skip(col);
    for expected in needle.chars() {
        match haystack_iter.next() {
            Some(c) if c == expected => {}
            _ => return false,
        }
    }
    true
}

/// Port of `CodeEdit::_get_auto_brace_pair_close_at_pos` (code_edit.cpp:3111-3133).
///
/// String-based version: checks if a close key of any pair starts at char
/// index `col` in `line_text`. Returns the pair index if found.
fn find_close_pair_at_pos_str(
    pairs: &[(String, String)],
    line_text: &str,
    col: usize,
) -> Option<usize> {
    let line_char_len = line_text.chars().count();
    for (i, (_open, close)) in pairs.iter().enumerate() {
        let close_char_len = close.chars().count();
        if col + close_char_len > line_char_len {
            continue;
        }
        if chars_match_at(line_text, col, close) {
            return Some(i);
        }
    }
    None
}

/// Port of `CodeEdit::_get_auto_brace_pair_open_at_pos` (code_edit.cpp:3085-3109).
///
/// String-based version: checks if an open key of any pair ends at char
/// index `col` in `line_text`. Returns the pair index if found.
fn find_open_pair_at_pos_str(
    pairs: &[(String, String)],
    line_text: &str,
    col: usize,
) -> Option<usize> {
    let line_char_len = line_text.chars().count();
    let caret_col = col.min(line_char_len);
    for (i, (open, _close)) in pairs.iter().enumerate() {
        let open_char_len = open.chars().count();
        if caret_col < open_char_len {
            continue;
        }
        let start = caret_col - open_char_len;
        if chars_match_at(line_text, start, open) {
            return Some(i);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_symbol_paren() {
        assert!(is_symbol('('));
        assert!(is_symbol(')'));
        assert!(is_symbol('{'));
        assert!(is_symbol('}'));
        assert!(is_symbol('['));
        assert!(is_symbol(']'));
        assert!(is_symbol('"'));
        assert!(is_symbol('\''));
        assert!(is_symbol(';'));
        assert!(is_symbol(':'));
        assert!(is_symbol(' '));
        assert!(is_symbol('\t'));
    }

    #[test]
    fn is_symbol_non_symbol() {
        assert!(!is_symbol('a'));
        assert!(!is_symbol('Z'));
        assert!(!is_symbol('0'));
        assert!(!is_symbol('_'));
    }

    #[test]
    fn find_close_pair_paren() {
        let pairs = vec![
            ("(".to_string(), ")".to_string()),
            ("{".to_string(), "}".to_string()),
        ];
        assert_eq!(find_close_pair_at_pos_str(&pairs, "foo()", 4), Some(0)); // ')' at col 4
        assert_eq!(find_close_pair_at_pos_str(&pairs, "foo()", 3), None); // '(' at col 3
    }

    #[test]
    fn find_open_pair_paren() {
        let pairs = vec![
            ("(".to_string(), ")".to_string()),
            ("{".to_string(), "}".to_string()),
        ];
        assert_eq!(find_open_pair_at_pos_str(&pairs, "foo(", 4), Some(0)); // '(' ends at col 4
        assert_eq!(find_open_pair_at_pos_str(&pairs, "foo(", 3), None);
    }

    #[test]
    fn find_close_pair_multichar() {
        let pairs = vec![("/*".to_string(), "*/".to_string())];
        assert_eq!(find_close_pair_at_pos_str(&pairs, "foo*/bar", 3), Some(0)); // "*/" at col 3
        assert_eq!(find_close_pair_at_pos_str(&pairs, "foo*/bar", 4), None);
    }

    #[test]
    fn find_open_pair_multichar() {
        let pairs = vec![("/*".to_string(), "*/".to_string())];
        assert_eq!(find_open_pair_at_pos_str(&pairs, "foo/*", 5), Some(0)); // "/*" ends at col 5
    }
}
