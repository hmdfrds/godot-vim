//! Adapter implementing vim-core's [`Document`] trait over a `&str` + [`LineIndex`].
//!
//! `GodotDocument` is created once per keystroke from the CodeEdit text snapshot.
//! It owns a `LineIndex` (a sorted vec of line-start byte offsets) that enables
//! O(log n) conversion between byte offsets and (line, col) positions — the
//! central coordinate translation between vim-core (byte offsets) and Godot
//! (line/column pairs, measured in Unicode scalar values).

use vim_core::document::Document;
use vim_core::primitives::{Offset, Position};

use super::codec::{self, i32_to_usize};

/// Read-only document backed by a borrowed `&str` with a pre-computed [`LineIndex`].
///
/// Columns are measured in Unicode scalar values (matching Godot's indexing),
/// not UTF-8 bytes or grapheme clusters. The `LineIndex` handles the
/// byte-offset-to-scalar-column conversion.
pub(crate) struct GodotDocument<'a> {
    text: &'a str,
    line_index: codec::LineIndex,
}

impl<'a> GodotDocument<'a> {
    #[must_use]
    pub(crate) fn new(text: &'a str) -> Self {
        Self {
            text,
            line_index: codec::LineIndex::new(text),
        }
    }

    #[must_use]
    pub(crate) fn text(&self) -> &str {
        self.text
    }

    #[must_use]
    pub(crate) fn line_index(&self) -> &codec::LineIndex {
        &self.line_index
    }

    /// Consume the document to extract the `LineIndex` without cloning.
    ///
    /// Used in effect dispatch: the text `&str` is borrowed from the engine
    /// response, but the `LineIndex` is needed independently for coordinate
    /// conversion during effect application.
    #[must_use]
    pub(crate) fn into_line_index(self) -> codec::LineIndex {
        self.line_index
    }
}

impl PartialEq for GodotDocument<'_> {
    fn eq(&self, other: &Self) -> bool {
        self.text == other.text
    }
}

impl Eq for GodotDocument<'_> {}

impl std::fmt::Debug for GodotDocument<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GodotDocument")
            .field("len", &self.text.len())
            .finish()
    }
}

impl Document for GodotDocument<'_> {
    fn text(&self) -> &str {
        self.text
    }

    fn line_count(&self) -> usize {
        self.line_index.line_count()
    }

    fn offset_to_pos(&self, offset: Offset) -> Option<Position> {
        let lc = self.line_index.offset_to_line_col(self.text, offset.get())?;
        Some(Position::from_raw(i32_to_usize(lc.line), i32_to_usize(lc.col)))
    }

    fn pos_to_offset(&self, pos: Position) -> Option<Offset> {
        let offset = self.line_index.line_col_to_offset(self.text, pos.line().get(), pos.col().get())?;
        Some(Offset::new(offset))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_creates_document() {
        let doc = GodotDocument::new("hello world");
        assert_eq!(doc.text(), "hello world");
    }

    #[test]
    fn empty_document() {
        let doc = GodotDocument::new("");
        assert_eq!(doc.text(), "");
        assert!(doc.is_empty());
    }

    #[test]
    fn len_returns_byte_count() {
        let doc = GodotDocument::new("hello");
        assert_eq!(doc.len(), 5);
    }

    #[test]
    fn len_utf8_multibyte() {
        // "hë" = 3 bytes
        let doc = GodotDocument::new("hë");
        assert_eq!(doc.len(), 3);
    }

    #[test]
    fn is_empty_true_for_empty() {
        assert!(GodotDocument::new("").is_empty());
    }

    #[test]
    fn is_empty_false_for_nonempty() {
        assert!(!GodotDocument::new("a").is_empty());
    }

    #[test]
    fn line_count_empty() {
        assert_eq!(GodotDocument::new("").line_count(), 1);
    }

    #[test]
    fn line_count_single_line() {
        assert_eq!(GodotDocument::new("hello").line_count(), 1);
    }

    #[test]
    fn line_count_two_lines() {
        assert_eq!(GodotDocument::new("hello\nworld").line_count(), 2);
    }

    #[test]
    fn line_count_trailing_newline() {
        // "hello\n" splits into ["hello", ""] -> 2 segments
        assert_eq!(GodotDocument::new("hello\n").line_count(), 2);
    }

    #[test]
    fn line_count_multiple_lines() {
        assert_eq!(GodotDocument::new("a\nb\nc\nd").line_count(), 4);
    }

    #[test]
    fn line_count_only_newlines() {
        // "\n\n" -> ["", "", ""] -> 3 lines
        assert_eq!(GodotDocument::new("\n\n").line_count(), 3);
    }

    fn o(n: usize) -> Offset { Offset::new(n) }

    #[test]
    fn offset_to_pos_start() {
        let doc = GodotDocument::new("hello\nworld");
        assert_eq!(doc.offset_to_pos(o(0)), Some(Position::from_raw(0, 0)));
    }

    #[test]
    fn offset_to_pos_mid_first_line() {
        let doc = GodotDocument::new("hello\nworld");
        assert_eq!(doc.offset_to_pos(o(3)), Some(Position::from_raw(0, 3)));
    }

    #[test]
    fn offset_to_pos_at_newline() {
        let doc = GodotDocument::new("hello\nworld");
        // Byte 5 = '\n', still line 0 col 5
        assert_eq!(doc.offset_to_pos(o(5)), Some(Position::from_raw(0, 5)));
    }

    #[test]
    fn offset_to_pos_second_line_start() {
        let doc = GodotDocument::new("hello\nworld");
        assert_eq!(doc.offset_to_pos(o(6)), Some(Position::from_raw(1, 0)));
    }

    #[test]
    fn offset_to_pos_second_line_middle() {
        let doc = GodotDocument::new("hello\nworld");
        assert_eq!(doc.offset_to_pos(o(8)), Some(Position::from_raw(1, 2)));
    }

    #[test]
    fn offset_to_pos_at_end() {
        let doc = GodotDocument::new("hello");
        // offset == len is valid (cursor past last char)
        assert_eq!(doc.offset_to_pos(o(5)), Some(Position::from_raw(0, 5)));
    }

    #[test]
    fn offset_to_pos_out_of_bounds() {
        let doc = GodotDocument::new("hello");
        assert_eq!(doc.offset_to_pos(o(6)), None);
    }

    #[test]
    fn offset_to_pos_empty() {
        let doc = GodotDocument::new("");
        assert_eq!(doc.offset_to_pos(o(0)), Some(Position::from_raw(0, 0)));
        assert_eq!(doc.offset_to_pos(o(1)), None);
    }

    #[test]
    fn pos_to_offset_origin() {
        let doc = GodotDocument::new("hello\nworld");
        assert_eq!(doc.pos_to_offset(Position::from_raw(0, 0)), Some(o(0)));
    }

    #[test]
    fn pos_to_offset_mid_first_line() {
        let doc = GodotDocument::new("hello\nworld");
        assert_eq!(doc.pos_to_offset(Position::from_raw(0, 3)), Some(o(3)));
    }

    #[test]
    fn pos_to_offset_second_line_start() {
        let doc = GodotDocument::new("hello\nworld");
        assert_eq!(doc.pos_to_offset(Position::from_raw(1, 0)), Some(o(6)));
    }

    #[test]
    fn pos_to_offset_second_line_middle() {
        let doc = GodotDocument::new("hello\nworld");
        assert_eq!(doc.pos_to_offset(Position::from_raw(1, 2)), Some(o(8)));
    }

    #[test]
    fn pos_to_offset_col_clamped_to_line_end() {
        let doc = GodotDocument::new("abc\ndef");
        // Col 99 on line 0 clamps to byte 3 (end of "abc")
        assert_eq!(doc.pos_to_offset(Position::from_raw(0, 99)), Some(o(3)));
    }

    #[test]
    fn pos_to_offset_line_out_of_range() {
        let doc = GodotDocument::new("hello\nworld");
        assert_eq!(doc.pos_to_offset(Position::from_raw(5, 0)), None);
    }

    #[test]
    fn pos_to_offset_empty() {
        let doc = GodotDocument::new("");
        assert_eq!(doc.pos_to_offset(Position::from_raw(0, 0)), Some(o(0)));
        assert_eq!(doc.pos_to_offset(Position::from_raw(1, 0)), None);
    }

    #[test]
    fn roundtrip_offset_pos_ascii() {
        let doc = GodotDocument::new("hello\nworld\nfoo");
        for offset in 0..=doc.len() {
            if let Some(pos) = doc.offset_to_pos(o(offset)) {
                let back = doc.pos_to_offset(pos).unwrap();
                assert_eq!(back, o(offset), "roundtrip failed at offset {offset}");
            }
        }
    }

    #[test]
    fn eq_same_text() {
        let a = GodotDocument::new("hello");
        let b = GodotDocument::new("hello");
        assert_eq!(a, b);
    }

    #[test]
    fn ne_different_text() {
        let a = GodotDocument::new("hello");
        let b = GodotDocument::new("world");
        assert_ne!(a, b);
    }

    #[test]
    fn debug_shows_len() {
        let doc = GodotDocument::new("hello");
        let debug = format!("{doc:?}");
        assert!(debug.contains("GodotDocument"));
        assert!(debug.contains("5"));
    }

    // ─── Multibyte / grapheme column tests ──────────────────────────

    #[test]
    fn offset_to_pos_two_byte_char() {
        // "héllo" — 'é' is 2 bytes, but 1 grapheme
        let doc = GodotDocument::new("héllo");
        assert_eq!(doc.offset_to_pos(o(0)), Some(Position::from_raw(0, 0)));
        assert_eq!(doc.offset_to_pos(o(1)), Some(Position::from_raw(0, 1)));
        assert_eq!(doc.offset_to_pos(o(3)), Some(Position::from_raw(0, 2)));
        assert_eq!(doc.offset_to_pos(o(4)), Some(Position::from_raw(0, 3)));
        assert_eq!(doc.offset_to_pos(o(5)), Some(Position::from_raw(0, 4)));
    }

    #[test]
    fn offset_to_pos_cjk() {
        let doc = GodotDocument::new("a日b");
        assert_eq!(doc.offset_to_pos(o(0)), Some(Position::from_raw(0, 0)));
        assert_eq!(doc.offset_to_pos(o(1)), Some(Position::from_raw(0, 1)));
        assert_eq!(doc.offset_to_pos(o(4)), Some(Position::from_raw(0, 2)));
    }

    #[test]
    fn offset_to_pos_emoji() {
        let doc = GodotDocument::new("a😀b");
        assert_eq!(doc.offset_to_pos(o(0)), Some(Position::from_raw(0, 0)));
        assert_eq!(doc.offset_to_pos(o(1)), Some(Position::from_raw(0, 1)));
        assert_eq!(doc.offset_to_pos(o(5)), Some(Position::from_raw(0, 2)));
    }

    #[test]
    fn offset_to_pos_multiline_multibyte() {
        let doc = GodotDocument::new("héllo\n世界");
        assert_eq!(doc.offset_to_pos(o(7)), Some(Position::from_raw(1, 0)));
        assert_eq!(doc.offset_to_pos(o(10)), Some(Position::from_raw(1, 1)));
    }

    #[test]
    fn pos_to_offset_two_byte_char() {
        let doc = GodotDocument::new("héllo");
        assert_eq!(doc.pos_to_offset(Position::from_raw(0, 0)), Some(o(0)));
        assert_eq!(doc.pos_to_offset(Position::from_raw(0, 1)), Some(o(1)));
        assert_eq!(doc.pos_to_offset(Position::from_raw(0, 2)), Some(o(3)));
    }

    #[test]
    fn pos_to_offset_cjk() {
        let doc = GodotDocument::new("a日b");
        assert_eq!(doc.pos_to_offset(Position::from_raw(0, 0)), Some(o(0)));
        assert_eq!(doc.pos_to_offset(Position::from_raw(0, 1)), Some(o(1)));
        assert_eq!(doc.pos_to_offset(Position::from_raw(0, 2)), Some(o(4)));
    }

    #[test]
    fn pos_to_offset_emoji() {
        let doc = GodotDocument::new("a😀b");
        assert_eq!(doc.pos_to_offset(Position::from_raw(0, 0)), Some(o(0)));
        assert_eq!(doc.pos_to_offset(Position::from_raw(0, 1)), Some(o(1)));
        assert_eq!(doc.pos_to_offset(Position::from_raw(0, 2)), Some(o(5)));
    }

    #[test]
    fn pos_to_offset_multiline_multibyte() {
        let doc = GodotDocument::new("héllo\n世界");
        assert_eq!(doc.pos_to_offset(Position::from_raw(1, 0)), Some(o(7)));
        assert_eq!(doc.pos_to_offset(Position::from_raw(1, 1)), Some(o(10)));
    }

    #[test]
    fn pos_to_offset_col_clamp_multibyte() {
        let doc = GodotDocument::new("日");
        assert_eq!(doc.pos_to_offset(Position::from_raw(0, 99)), Some(o(3)));
    }

    #[test]
    fn roundtrip_multibyte_grapheme() {
        let doc = GodotDocument::new("héllo\n世界\na😀b\n");
        for offset in 0..doc.text().len() {
            if !doc.text().is_char_boundary(offset) {
                continue;
            }
            if let Some(pos) = doc.offset_to_pos(o(offset)) {
                let back = doc.pos_to_offset(pos).unwrap();
                assert_eq!(
                    back, o(offset),
                    "roundtrip failed at offset {offset} (pos {pos:?})"
                );
            }
        }
    }

    #[test]
    fn combining_character_grapheme() {
        let doc = GodotDocument::new("ae\u{0301}b");
        assert_eq!(doc.text().len(), 5);
        assert_eq!(doc.offset_to_pos(o(0)), Some(Position::from_raw(0, 0)));
        assert_eq!(doc.offset_to_pos(o(1)), Some(Position::from_raw(0, 1)));
        assert_eq!(doc.offset_to_pos(o(4)), Some(Position::from_raw(0, 2)));
        assert_eq!(doc.pos_to_offset(Position::from_raw(0, 1)), Some(o(1)));
        assert_eq!(doc.pos_to_offset(Position::from_raw(0, 2)), Some(o(4)));
    }

    #[test]
    fn flag_emoji_grapheme() {
        let doc = GodotDocument::new("a🇺🇸b");
        assert_eq!(doc.text().len(), 10);
        assert_eq!(doc.offset_to_pos(o(0)), Some(Position::from_raw(0, 0)));
        assert_eq!(doc.offset_to_pos(o(1)), Some(Position::from_raw(0, 1)));
        assert_eq!(doc.offset_to_pos(o(9)), Some(Position::from_raw(0, 2)));
        assert_eq!(doc.pos_to_offset(Position::from_raw(0, 0)), Some(o(0)));
        assert_eq!(doc.pos_to_offset(Position::from_raw(0, 1)), Some(o(1)));
        assert_eq!(doc.pos_to_offset(Position::from_raw(0, 2)), Some(o(9)));
    }
}
