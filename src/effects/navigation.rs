//! LSP navigation effect handlers (`gd`, `K`).
//!
//! Both operations piggyback on Godot's built-in symbol lookup infrastructure
//! by emitting the same signals that `ScriptTextEditor` already listens on.
//!
//! Word extraction is implemented locally using standard word-boundary detection
//! (alphanumeric + underscore = word char, Unicode-aware) so that this module
//! stays within the public effects/primitives/document API and does not reach
//! into vim-core's internal command layer.

use crate::bridge::codec::i32_to_usize;
use crate::bridge::port::NavigationCapable;

#[inline]
fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// Extract the identifier at `col`, using the same Unicode word-boundary
/// rules as vim-core's `*`/`#`/`w`/`e`/`b` motions.
fn extract_word_at_col(line_text: &str, col: usize) -> Option<&str> {
    let byte_offset = line_text.char_indices().nth(col).map(|(i, _)| i)?;

    let cursor_char = line_text[byte_offset..].chars().next()?;
    if !is_word_char(cursor_char) {
        return None;
    }

    let mut start = byte_offset;
    for (idx, c) in line_text[..byte_offset].char_indices().rev() {
        if !is_word_char(c) {
            start = idx + c.len_utf8();
            break;
        }
        start = idx;
    }

    let after_cursor = byte_offset + cursor_char.len_utf8();
    let mut end = after_cursor;
    for (idx, c) in line_text[after_cursor..].char_indices() {
        if !is_word_char(c) {
            end = after_cursor + idx;
            break;
        }
        end = after_cursor + idx + c.len_utf8();
    }

    let word = &line_text[start..end];
    if word.is_empty() { None } else { Some(word) }
}

/// Extract the word under the cursor and pass it to `action`.
fn with_word_under_cursor(
    editor: &mut impl NavigationCapable,
    label: &str,
    action: impl FnOnce(&mut dyn NavigationCapable, &str, i32, i32),
) {
    let line = editor.get_caret_line();
    let col = editor.get_caret_column();
    let line_text = editor.get_line(line);

    if let Some(word) = extract_word_at_col(&line_text, i32_to_usize(col)) {
        action(editor, word, line, col);
        log::debug!("{}: signal emitted for '{}' at {}:{}", label, word, line, col);
    } else {
        log::debug!("{}: no symbol under cursor at {}:{}", label, line, col);
    }
}

/// `gd`: emit Godot's `symbol_lookup` signal, which `ScriptTextEditor`
/// handles by performing the actual LSP lookup and navigation.
pub(super) fn handle_goto_definition(editor: &mut impl NavigationCapable) {
    with_word_under_cursor(editor, "gd", |ed, word, line, col| {
        ed.emit_symbol_lookup(word, line, col);
    });
}

/// `K`: warp the mouse to the symbol and emit `symbol_hovered`. Godot's
/// `ScriptTextEditor` displays the hover documentation popup at the mouse
/// position. The mouse warp is necessary because Godot shows the tooltip
/// at the cursor, not at the symbol's screen coordinates.
pub(super) fn handle_show_documentation(editor: &mut impl NavigationCapable) {
    with_word_under_cursor(editor, "K", |ed, word, line, col| {
        ed.emit_symbol_hovered_with_mouse_warp(word, line, col);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_word_at_middle() {
        assert_eq!(extract_word_at_col("hello world", 2), Some("hello"));
    }

    #[test]
    fn extract_word_at_start() {
        assert_eq!(extract_word_at_col("foo_bar baz", 0), Some("foo_bar"));
    }

    #[test]
    fn extract_word_at_end() {
        assert_eq!(extract_word_at_col("hello", 4), Some("hello"));
    }

    #[test]
    fn extract_word_with_underscore() {
        assert_eq!(extract_word_at_col("my_var = 5", 3), Some("my_var"));
    }

    #[test]
    fn extract_word_at_space_returns_none() {
        assert_eq!(extract_word_at_col("hello world", 5), None);
    }

    #[test]
    fn extract_word_at_out_of_bounds_returns_none() {
        assert_eq!(extract_word_at_col("hi", 5), None);
    }

    #[test]
    fn extract_word_empty_string() {
        assert_eq!(extract_word_at_col("", 0), None);
    }

    #[test]
    fn extract_word_at_punctuation() {
        assert_eq!(extract_word_at_col("a.b", 1), None);
    }

    #[test]
    fn extract_word_unicode_accented() {
        assert_eq!(extract_word_at_col("let café = 1", 4), Some("café"));
    }

    #[test]
    fn extract_word_unicode_mixed() {
        assert_eq!(extract_word_at_col("naïve_var", 2), Some("naïve_var"));
    }
}
