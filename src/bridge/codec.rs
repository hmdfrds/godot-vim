//! Coordinate codec: bridges Godot's `(line, col)` (Unicode-scalar columns)
//! and vim-core's byte offsets. Handles three column representations:
//!
//! - **Godot chars**: Unicode scalar values (codepoints) — what `get_caret_column()` returns
//! - **Grapheme clusters**: user-perceived characters — what the engine's sticky column uses
//! - **Byte offsets**: raw UTF-8 positions — what vim-core operates on internally
//!
//! The [`LineIndex`] provides O(log n) lookups via pre-computed line-start offsets,
//! with incremental `apply_insert`/`apply_delete` to avoid full rebuilds on each edit.

// ─────────────────────────────────────────────────────────────────────────────
// LineIndex — cached line-start offsets for O(log n) lookups
// ─────────────────────────────────────────────────────────────────────────────

use unicode_segmentation::UnicodeSegmentation;

use crate::types::{CharLineCol, GraphemeLineCol};

/// Pre-computed line-start byte offsets for O(log n) line/column conversion.
///
/// Built once via [`LineIndex::new`], then kept in sync with edits via
/// [`apply_insert`]/[`apply_delete`] to avoid O(n) rebuilds on every keystroke.
///
/// # Invariants
///
/// - `line_starts[0] == 0` (line 0 always starts at byte 0)
/// - `line_starts[i] < line_starts[i+1]` (strictly increasing)
/// - `line_starts.len()` == number of lines in the document
/// - `text_len` == byte length of the text this index was built from
#[derive(Debug, Clone)]
pub(crate) struct LineIndex {
    line_starts: Vec<usize>,
    text_len: usize,
}

impl LineIndex {
    /// Build from full document text. O(n) — amortized by incremental updates thereafter.
    #[must_use]
    pub(crate) fn new(text: &str) -> Self {
        let bytes = text.as_bytes();
        // Pre-allocate assuming ~40 bytes per line (typical for source code).
        let mut line_starts = Vec::with_capacity((bytes.len() / 40).max(8));
        line_starts.push(0);
        for (i, &byte) in bytes.iter().enumerate() {
            if byte == b'\n' {
                line_starts.push(i + 1);
            }
        }
        debug_assert_eq!(line_starts[0], 0, "LineIndex: first line must start at offset 0");
        debug_assert!(
            line_starts.windows(2).all(|w| w[0] < w[1]),
            "LineIndex: line_starts must be strictly increasing"
        );

        Self {
            line_starts,
            text_len: text.len(),
        }
    }

    #[inline]
    pub(crate) fn line_count(&self) -> usize {
        self.line_starts.len()
    }

    #[cfg(test)]
    pub(crate) fn line_start(&self, target_line: usize) -> Option<usize> {
        self.line_starts.get(target_line).copied()
    }

    /// Exclusive end offset of `target_line`: points at the `\n`, or `text_len` for the last line.
    #[inline]
    pub(crate) fn line_end(&self, target_line: usize) -> usize {
        if target_line + 1 < self.line_starts.len() {
            // -1 excludes the newline, so text[start..end] yields line content without '\n'.
            self.line_starts[target_line + 1] - 1
        } else {
            self.text_len
        }
    }

    /// Godot `(line, char_col)` -> byte offset. O(1) line lookup + O(col) scalar walk.
    ///
    /// All inputs are clamped: negative values to 0, out-of-range lines to the
    /// last line, out-of-range columns to line end. This matches Godot's own
    /// clamping behavior so callers never need to bounds-check.
    pub(crate) fn line_col_to_byte(&self, text: &str, line: i32, col: i32) -> usize {
        if text.is_empty() {
            return 0;
        }

        let target_line = i32_to_usize(line);
        let target_char_col = i32_to_usize(col);

        let clamped_line = target_line.min(self.line_starts.len() - 1);
        let line_start = self.line_starts[clamped_line];

        let line_text = self.line_text_at(text, clamped_line);
        let byte_col = char_col_to_byte_offset(line_text, target_char_col);

        line_start + byte_col
    }

    /// Byte offset -> Godot `(line, char_col)`. O(log n) binary search + O(col) char count.
    pub(crate) fn byte_to_line_col(&self, text: &str, offset: usize) -> CharLineCol {
        if text.is_empty() {
            return CharLineCol::new(0, 0);
        }

        let clamped_offset = offset.min(self.text_len);
        // Snap down to the nearest char boundary if offset lands mid-UTF-8 sequence.
        // Defensive: should never happen with correct engine offsets, but prevents
        // a release-mode panic from slicing inside a multi-byte codepoint.
        let clamped_offset = text.floor_char_boundary(clamped_offset);

        let line_number = match self.line_starts.binary_search(&clamped_offset) {
            Ok(exact) => exact,
            // Safety: insert_pos > 0 is guaranteed because line_starts[0] == 0 <= any offset.
            Err(insert_pos) => insert_pos - 1,
        };

        let line_start = self.line_starts[line_number];
        let byte_col_in_line = clamped_offset - line_start;
        let line_text = self.line_text_at(text, line_number);
        let char_col = byte_offset_to_char_col(line_text, byte_col_in_line);

        CharLineCol::new(usize_to_i32(line_number), usize_to_i32(char_col))
    }

    /// Line content as a `&str` (excluding the trailing `\n`).
    ///
    /// Returns `""` for out-of-range lines in release builds; debug builds
    /// assert so invalid line numbers are caught early during development.
    #[inline]
    pub(crate) fn line_text_at<'a>(&self, text: &'a str, line_number: usize) -> &'a str {
        debug_assert!(
            line_number < self.line_count(),
            "line_text_at: line_number {line_number} is out of range (line_count = {})",
            self.line_count()
        );
        if line_number >= self.line_count() {
            return "";
        }
        let start = self.line_starts[line_number];
        let end = self.line_end(line_number);
        &text[start..end]
    }

    /// Unicode scalar count for the line. Uses cached offsets to slice the
    /// Rust-side text, avoiding a Godot FFI roundtrip through `editor.get_line()`.
    #[must_use]
    pub(crate) fn line_char_count(&self, text: &str, line_number: usize) -> usize {
        self.line_text_at(text, line_number).chars().count()
    }

    /// Byte offset -> `(line, grapheme_col)`. Returns `None` if offset > text_len
    /// or lands inside a multi-byte character (not on a char boundary).
    pub(crate) fn offset_to_line_col(&self, text: &str, offset: usize) -> Option<GraphemeLineCol> {
        if offset > self.text_len {
            return None;
        }
        let line = match self.line_starts.binary_search(&offset) {
            Ok(exact) => exact,
            Err(insert_point) => insert_point.saturating_sub(1),
        };
        let line_start = self.line_starts[line];
        if !text.is_char_boundary(offset) {
            return None;
        }
        let col = text[line_start..offset].graphemes(true).count();
        Some(GraphemeLineCol::new(usize_to_i32(line), usize_to_i32(col)))
    }

    /// `(line, grapheme_col)` -> byte offset. Columns past line end clamp to the
    /// last grapheme boundary (consistent with Vim's `$` motion behavior).
    pub(crate) fn line_col_to_offset(&self, text: &str, line: usize, col: usize) -> Option<usize> {
        if line >= self.line_starts.len() {
            return None;
        }
        let line_start = self.line_starts[line];
        let line_text = self.line_text_at(text, line);
        let mut byte_offset = line_start;
        for (i, g) in line_text.graphemes(true).enumerate() {
            if i == col {
                return Some(byte_offset);
            }
            byte_offset += g.len();
        }
        Some(byte_offset) // col past end -> clamp to end of line
    }

    /// Incrementally update after an insertion. O(inserted_newlines + lines_after)
    /// instead of rebuilding the entire index in O(total_text).
    pub(crate) fn apply_insert(&mut self, offset: usize, new_text: &str) {
        let new_bytes = new_text.len();
        if new_bytes == 0 {
            return;
        }

        let line = match self.line_starts.binary_search(&offset) {
            Ok(exact) => exact,
            Err(insert_point) => insert_point.saturating_sub(1),
        };

        // Collect new line starts from the inserted text (already at final offsets).
        let new_line_starts: Vec<usize> = new_text.bytes()
            .enumerate()
            .filter(|(_, b)| *b == b'\n')
            .map(|(i, _)| offset + i + 1)
            .collect();

        // Shift existing lines after the insertion point *before* splicing,
        // so the splice indices remain valid against the original vec layout.
        for ls in &mut self.line_starts[line + 1..] {
            *ls += new_bytes;
        }

        if !new_line_starts.is_empty() {
            let insert_pos = line + 1;
            self.line_starts.splice(insert_pos..insert_pos, new_line_starts);
        }

        self.text_len += new_bytes;
    }

    /// Incrementally update after a deletion of bytes `[start..end)`.
    pub(crate) fn apply_delete(&mut self, start: usize, end: usize) {
        if start >= end {
            return;
        }
        let removed_bytes = end - start;

        // A line_start falls within the deleted range if it was created by a newline
        // that is being removed. The `<= end` bound (not `< end`) is needed because
        // a newline at byte `end-1` creates a line_start at `end`.
        let first_removed = self.line_starts.partition_point(|&ls| ls <= start);
        let last_removed = self.line_starts.partition_point(|&ls| ls <= end);

        if first_removed < last_removed {
            self.line_starts.drain(first_removed..last_removed);
        }

        for ls in &mut self.line_starts[first_removed..] {
            *ls -= removed_bytes;
        }

        self.text_len -= removed_bytes;
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// DocumentView — borrowed text + LineIndex pair
// ─────────────────────────────────────────────────────────────────────────────

/// Borrowed `(&str, &LineIndex)` pair kept in sync. Passed through the effects
/// layer so text and its index are never accidentally separated.
#[derive(Debug, Clone, Copy)]
pub(crate) struct DocumentView<'a> {
    pub(crate) text: &'a str,
    pub(crate) line_index: &'a LineIndex,
}

impl<'a> DocumentView<'a> {
    #[must_use]
    pub(crate) fn new(text: &'a str, line_index: &'a LineIndex) -> Self {
        Self { text, line_index }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Free functions — O(n) linear-scan versions used only in tests as reference
// implementations to cross-check LineIndex results.
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
fn find_line_start(text: &str, target_line: usize) -> Option<usize> {
    if target_line == 0 {
        return Some(0);
    }

    let mut current_line: usize = 0;
    for (i, &byte) in text.as_bytes().iter().enumerate() {
        if byte == b'\n' {
            current_line += 1;
            if current_line == target_line {
                return Some(i + 1);
            }
        }
    }

    None
}

/// Slice from `line_start` to the next `\n` (or end of string).
#[cfg(test)]
fn line_text_at(text: &str, line_start: usize) -> &str {
    let remaining = &text[line_start..];
    remaining.split_once('\n').map_or(remaining, |(line, _)| line)
}

#[cfg(test)]
#[allow(dead_code)]
fn line_col_to_byte(text: &str, line: i32, col: i32) -> usize {
    if text.is_empty() {
        return 0;
    }

    let target_line = i32_to_usize(line);
    let target_char_col = i32_to_usize(col);

    let line_start = find_line_start(text, target_line)
        .unwrap_or_else(|| {
            // Out-of-range line: clamp to the last line by walking to its start.
            let mut ls: usize = 0;
            for (i, &byte) in text.as_bytes().iter().enumerate() {
                if byte == b'\n' {
                    ls = i + 1;
                }
            }
            ls
        });

    let line_text = line_text_at(text, line_start);
    let byte_col = char_col_to_byte_offset(line_text, target_char_col);

    line_start + byte_col
}

#[cfg(test)]
fn byte_to_line_col(text: &str, offset: usize) -> CharLineCol {
    if text.is_empty() {
        return CharLineCol::new(0, 0);
    }

    let clamped_offset = offset.min(text.len());
    // Snap to nearest char boundary down if the offset falls mid-UTF-8.
    // This is defensive -- it should not happen with correct engine offsets,
    // but prevents a release-mode panic on corrupted byte offsets.
    let clamped_offset = text.floor_char_boundary(clamped_offset);

    let prefix = &text.as_bytes()[..clamped_offset];

    let mut line_number: usize = 0;
    let mut line_start: usize = 0;

    for (i, &byte) in prefix.iter().enumerate() {
        if byte == b'\n' {
            line_number += 1;
            line_start = i + 1;
        }
    }

    let byte_col_in_line = clamped_offset - line_start;
    let line_text = line_text_at(text, line_start);
    let char_col = byte_offset_to_char_col(line_text, byte_col_in_line);

    CharLineCol::new(usize_to_i32(line_number), usize_to_i32(char_col))
}

/// Walk `char_col` Unicode scalars into `line`, returning the byte offset.
/// Clamps to `line.len()` if `char_col` exceeds the line's char count.
fn char_col_to_byte_offset(line: &str, char_col: usize) -> usize {
    for (char_count, (byte_offset, _)) in line.char_indices().enumerate() {
        if char_count == char_col {
            return byte_offset;
        }
    }
    line.len()
}

/// Inverse of `char_col_to_byte_offset`: count chars in `line[..byte_col]`.
fn byte_offset_to_char_col(line: &str, byte_col: usize) -> usize {
    let clamped = byte_col.min(line.len());
    line[..clamped].chars().count()
}

/// Godot `i32` -> Rust `usize`. Negatives clamp to 0 (Godot APIs can return -1 as a sentinel).
pub(crate) fn i32_to_usize(val: i32) -> usize {
    if val < 0 {
        log::warn!("i32_to_usize: negative value {} clamped to 0", val);
        0
    } else {
        val as usize
    }
}

/// Rust `usize` -> Godot `i32`. Overflows clamp to `i32::MAX` (documents > 2 GiB are unsupported).
pub(crate) fn usize_to_i32(val: usize) -> i32 {
    if val > i32::MAX as usize {
        log::warn!("usize_to_i32: overflow {} clamped to i32::MAX", val);
        i32::MAX
    } else {
        val as i32
    }
}

/// Godot char column (Unicode scalars) -> grapheme column (user-perceived characters).
///
/// Godot's `get_caret_column()` counts codepoints, but vim-core's sticky column
/// counts grapheme clusters. These differ for combining marks (e.g., `e` + `\u{0301}`
/// = 2 codepoints, 1 grapheme) and multi-codepoint emoji. ASCII text is unaffected.
pub(crate) fn char_col_to_grapheme_col(line_text: &str, char_col: usize) -> usize {
    use unicode_segmentation::UnicodeSegmentation;
    let mut chars_seen = 0usize;
    let mut grapheme_col = 0usize;
    for grapheme in line_text.graphemes(true) {
        if chars_seen >= char_col {
            break;
        }
        chars_seen += grapheme.chars().count();
        grapheme_col += 1;
    }
    grapheme_col
}

#[cfg(test)]
mod tests {
    use super::*;

    // ─── LineIndex tests ──────────────────────────────────────────────

    #[test]
    fn line_index_simple() {
        let text = "hello\nworld\n";
        let idx = LineIndex::new(text);
        assert_eq!(idx.line_count(), 3); // "hello", "world", ""
        assert_eq!(idx.line_start(0), Some(0));
        assert_eq!(idx.line_start(1), Some(6));
        assert_eq!(idx.line_start(2), Some(12));
        assert_eq!(idx.line_start(3), None);
    }

    #[test]
    fn line_index_line_col_to_byte() {
        let text = "hello\nworld\n";
        let idx = LineIndex::new(text);
        assert_eq!(idx.line_col_to_byte(text, 0, 0), 0);
        assert_eq!(idx.line_col_to_byte(text, 0, 3), 3);
        assert_eq!(idx.line_col_to_byte(text, 1, 0), 6);
        assert_eq!(idx.line_col_to_byte(text, 1, 2), 8);
    }

    #[test]
    fn line_index_byte_to_line_col() {
        let text = "hello\nworld\n";
        let idx = LineIndex::new(text);
        assert_eq!(idx.byte_to_line_col(text, 0), CharLineCol::new(0, 0));
        assert_eq!(idx.byte_to_line_col(text, 3), CharLineCol::new(0, 3));
        assert_eq!(idx.byte_to_line_col(text, 6), CharLineCol::new(1, 0));
        assert_eq!(idx.byte_to_line_col(text, 8), CharLineCol::new(1, 2));
    }

    #[test]
    fn line_index_roundtrip_unicode() {
        let text = "hëllo\n日本語\na😀b\n";
        let idx = LineIndex::new(text);
        for (offset, _) in text.char_indices() {
            let pos = idx.byte_to_line_col(text, offset);
            let back = idx.line_col_to_byte(text, pos.line, pos.col);
            assert_eq!(back, offset, "roundtrip failed at offset {offset}");
        }
    }

    #[test]
    fn line_index_empty() {
        let text = "";
        let idx = LineIndex::new(text);
        assert_eq!(idx.line_count(), 1);
        assert_eq!(idx.line_col_to_byte(text, 0, 0), 0);
        assert_eq!(idx.byte_to_line_col(text, 0), CharLineCol::new(0, 0));
    }

    #[test]
    fn line_index_clamp_beyond_end() {
        let text = "hello\nworld";
        let idx = LineIndex::new(text);
        // Line 99 clamps to last line
        assert_eq!(idx.line_col_to_byte(text, 99, 0), 6);
        // Byte 100 clamps to text.len()
        assert_eq!(idx.byte_to_line_col(text, 100), CharLineCol::new(1, 5));
    }

    #[test]
    fn line_index_multiple_empty_lines() {
        let text = "\n\n\n";
        let idx = LineIndex::new(text);
        assert_eq!(idx.line_count(), 4);
        assert_eq!(idx.line_col_to_byte(text, 0, 0), 0);
        assert_eq!(idx.line_col_to_byte(text, 1, 0), 1);
        assert_eq!(idx.line_col_to_byte(text, 2, 0), 2);
        assert_eq!(idx.line_col_to_byte(text, 3, 0), 3);
        assert_eq!(idx.byte_to_line_col(text, 0), CharLineCol::new(0, 0));
        assert_eq!(idx.byte_to_line_col(text, 1), CharLineCol::new(1, 0));
        assert_eq!(idx.byte_to_line_col(text, 2), CharLineCol::new(2, 0));
        assert_eq!(idx.byte_to_line_col(text, 3), CharLineCol::new(3, 0));
    }

    // ─── Free-function reference tests (cross-check against LineIndex) ────

    #[test]
    fn ascii_first_line_start() {
        let text = "hello\nworld\n";
        assert_eq!(line_col_to_byte(text, 0, 0), 0);
    }

    #[test]
    fn ascii_first_line_middle() {
        let text = "hello\nworld\n";
        assert_eq!(line_col_to_byte(text, 0, 3), 3);
    }

    #[test]
    fn ascii_second_line_start() {
        let text = "hello\nworld\n";
        assert_eq!(line_col_to_byte(text, 1, 0), 6);
    }

    #[test]
    fn ascii_second_line_middle() {
        let text = "hello\nworld\n";
        assert_eq!(line_col_to_byte(text, 1, 2), 8);
    }

    #[test]
    fn negative_line_clamps_to_zero() {
        let text = "hello\nworld";
        assert_eq!(line_col_to_byte(text, -5, 2), 2);
    }

    #[test]
    fn negative_col_clamps_to_zero() {
        let text = "hello\nworld";
        assert_eq!(line_col_to_byte(text, 1, -3), 6);
    }

    #[test]
    fn line_beyond_end_clamps_to_last() {
        let text = "hello\nworld";
        // Line 99 clamps to line 1 ("world"), col 0
        assert_eq!(line_col_to_byte(text, 99, 0), 6);
    }

    #[test]
    fn col_beyond_end_clamps_to_line_end() {
        let text = "hello\nworld";
        // Line 0, col 99 clamps to byte 5 (end of "hello")
        assert_eq!(line_col_to_byte(text, 0, 99), 5);
    }

    #[test]
    fn empty_text_returns_zero() {
        assert_eq!(line_col_to_byte("", 0, 0), 0);
        assert_eq!(line_col_to_byte("", 5, 3), 0);
        assert_eq!(line_col_to_byte("", -1, -1), 0);
    }

    #[test]
    fn utf8_two_byte_char() {
        // "hë" = 3 bytes ('h'=1, 'ë'=2)
        let text = "hëllo";
        // char col 2 = 'l' at byte 3
        assert_eq!(line_col_to_byte(text, 0, 2), 3);
    }

    #[test]
    fn utf8_three_byte_cjk() {
        // "日" is 3 bytes
        let text = "a日b";
        // char col 1 = '日' at byte 1
        assert_eq!(line_col_to_byte(text, 0, 1), 1);
        // char col 2 = 'b' at byte 4
        assert_eq!(line_col_to_byte(text, 0, 2), 4);
    }

    #[test]
    fn utf8_four_byte_emoji() {
        // "😀" is 4 bytes
        let text = "a😀b";
        // char col 2 = 'b' at byte 5
        assert_eq!(line_col_to_byte(text, 0, 2), 5);
    }

    #[test]
    fn utf8_multiline() {
        // "hë\n日b"
        // Line 0: "hë" (3 bytes) + '\n' = 4 bytes
        // Line 1: "日b" starts at byte 4
        let text = "hë\n日b";
        // Line 1, char col 0 = '日' at byte 4
        assert_eq!(line_col_to_byte(text, 1, 0), 4);
        // Line 1, char col 1 = 'b' at byte 7
        assert_eq!(line_col_to_byte(text, 1, 1), 7);
    }

    #[test]
    fn byte_to_lc_first_line_start() {
        let text = "hello\nworld\n";
        assert_eq!(byte_to_line_col(text, 0), CharLineCol::new(0, 0));
    }

    #[test]
    fn byte_to_lc_first_line_middle() {
        let text = "hello\nworld\n";
        assert_eq!(byte_to_line_col(text, 3), CharLineCol::new(0, 3));
    }

    #[test]
    fn byte_to_lc_at_newline() {
        let text = "hello\nworld\n";
        // Byte 5 is '\n', still on line 0, col 5
        assert_eq!(byte_to_line_col(text, 5), CharLineCol::new(0, 5));
    }

    #[test]
    fn byte_to_lc_second_line_start() {
        let text = "hello\nworld\n";
        assert_eq!(byte_to_line_col(text, 6), CharLineCol::new(1, 0));
    }

    #[test]
    fn byte_to_lc_second_line_middle() {
        let text = "hello\nworld\n";
        assert_eq!(byte_to_line_col(text, 8), CharLineCol::new(1, 2));
    }

    #[test]
    fn byte_offset_beyond_end_clamps() {
        let text = "abc";
        // Offset 100 clamps to offset 3 (text.len())
        assert_eq!(byte_to_line_col(text, 100), CharLineCol::new(0, 3));
    }

    #[test]
    fn byte_offset_at_exact_end() {
        let text = "abc";
        assert_eq!(byte_to_line_col(text, 3), CharLineCol::new(0, 3));
    }

    #[test]
    fn byte_to_lc_empty_text() {
        assert_eq!(byte_to_line_col("", 0), CharLineCol::new(0, 0));
        assert_eq!(byte_to_line_col("", 99), CharLineCol::new(0, 0));
    }

    #[test]
    fn byte_to_lc_utf8_two_byte() {
        let text = "hëllo";
        // Byte 3 = 'l', which is char col 2
        assert_eq!(byte_to_line_col(text, 3), CharLineCol::new(0, 2));
    }

    #[test]
    fn byte_to_lc_utf8_cjk() {
        let text = "a日b";
        // Byte 4 = 'b', which is char col 2
        assert_eq!(byte_to_line_col(text, 4), CharLineCol::new(0, 2));
    }

    #[test]
    fn byte_to_lc_utf8_multiline() {
        // "hë\n日b"
        let text = "hë\n日b";
        // Byte 4 = '日' on line 1, char col 0
        assert_eq!(byte_to_line_col(text, 4), CharLineCol::new(1, 0));
        // Byte 7 = 'b' on line 1, char col 1
        assert_eq!(byte_to_line_col(text, 7), CharLineCol::new(1, 1));
    }

    #[test]
    fn roundtrip_ascii() {
        let text = "hello\nworld\nfoo bar\n";
        for offset in 0..text.len() {
            let pos = byte_to_line_col(text, offset);
            let back = line_col_to_byte(text, pos.line, pos.col);
            assert_eq!(back, offset, "roundtrip failed at offset {offset}");
        }
    }

    #[test]
    fn roundtrip_unicode() {
        let text = "hëllo\n日本語\na😀b\n";
        // Only test at char boundaries for valid round-trips.
        for (offset, _) in text.char_indices() {
            let pos = byte_to_line_col(text, offset);
            let back = line_col_to_byte(text, pos.line, pos.col);
            assert_eq!(back, offset, "roundtrip failed at offset {offset}");
        }
    }

    #[test]
    fn i32_to_usize_positive() {
        assert_eq!(i32_to_usize(42), 42);
    }

    #[test]
    fn i32_to_usize_zero() {
        assert_eq!(i32_to_usize(0), 0);
    }

    #[test]
    fn i32_to_usize_negative() {
        assert_eq!(i32_to_usize(-1), 0);
        assert_eq!(i32_to_usize(i32::MIN), 0);
    }

    #[test]
    fn usize_to_i32_normal() {
        assert_eq!(usize_to_i32(42), 42);
    }

    #[test]
    fn usize_to_i32_max_clamp() {
        assert_eq!(usize_to_i32(usize::MAX), i32::MAX);
    }

    #[test]
    fn char_col_to_byte_offset_ascii() {
        assert_eq!(char_col_to_byte_offset("hello", 3), 3);
    }

    #[test]
    fn char_col_to_byte_offset_utf8() {
        // 'ë' is 2 bytes
        assert_eq!(char_col_to_byte_offset("hëllo", 2), 3);
    }

    #[test]
    fn char_col_to_byte_offset_clamp() {
        assert_eq!(char_col_to_byte_offset("abc", 99), 3);
    }

    #[test]
    fn byte_offset_to_char_col_ascii() {
        assert_eq!(byte_offset_to_char_col("hello", 3), 3);
    }

    #[test]
    fn byte_offset_to_char_col_utf8() {
        // Byte 3 in "hëllo" is char 2
        assert_eq!(byte_offset_to_char_col("hëllo", 3), 2);
    }

    #[test]
    fn byte_offset_to_char_col_clamp() {
        assert_eq!(byte_offset_to_char_col("abc", 99), 3);
    }

    mod prop_tests {
        use super::*;
        use proptest::prelude::*;

        fn arb_ascii_doc() -> impl Strategy<Value = String> {
            prop::collection::vec("[a-zA-Z0-9 ]{0,80}", 1..50)
                .prop_map(|lines| lines.join("\n"))
        }

        /// Generate text with Unicode characters: accented Latin, CJK, emoji.
        fn arb_unicode_doc() -> impl Strategy<Value = String> {
            prop::collection::vec(
                prop::string::string_regex("[a-zA-Z\u{00e0}-\u{00ff}\u{4e00}-\u{4e10}\u{0300}-\u{0302}]{0,40}")
                    .unwrap(),
                1..20,
            )
            .prop_map(|lines| lines.join("\n"))
        }

        proptest! {
            #[test]
            fn line_col_roundtrip(text in arb_ascii_doc()) {
                let index = LineIndex::new(&text);
                for offset in 0..text.len() {
                    if !text.is_char_boundary(offset) { continue; }
                    if let Some(lc) = index.offset_to_line_col(&text, offset) {
                        let back = index.line_col_to_offset(
                            &text,
                            lc.line as usize,
                            lc.col as usize,
                        );
                        prop_assert_eq!(back, Some(offset),
                            "Round-trip failed: offset={} -> ({},{}) -> {:?}",
                            offset, lc.line, lc.col, back);
                    }
                }
            }

            /// Unicode roundtrip: generate text with accented, CJK chars and
            /// verify that offset -> (line,col) -> offset is lossless for
            /// offsets that land on grapheme boundaries.
            ///
            /// `offset_to_line_col` returns grapheme columns, so offsets in the
            /// middle of a multi-codepoint grapheme cluster (e.g. between a base
            /// char and its combining mark) are not expected to roundtrip.
            #[test]
            fn unicode_line_col_roundtrip(text in arb_unicode_doc()) {
                use unicode_segmentation::UnicodeSegmentation;
                let index = LineIndex::new(&text);
                // Collect all grapheme-boundary byte offsets.
                let mut grapheme_offsets = std::collections::HashSet::new();
                let mut pos = 0usize;
                for line in text.split('\n') {
                    let mut byte = pos;
                    for g in line.graphemes(true) {
                        grapheme_offsets.insert(byte);
                        byte += g.len();
                    }
                    grapheme_offsets.insert(byte); // end-of-line
                    pos = byte + 1; // skip the '\n'
                }
                grapheme_offsets.insert(text.len()); // past last line (no trailing \n)

                for &offset in &grapheme_offsets {
                    if offset > text.len() { continue; }
                    if !text.is_char_boundary(offset) { continue; }
                    if let Some(lc) = index.offset_to_line_col(&text, offset) {
                        let back = index.line_col_to_offset(
                            &text,
                            lc.line as usize,
                            lc.col as usize,
                        );
                        prop_assert_eq!(back, Some(offset),
                            "Unicode round-trip failed: offset={} -> ({},{}) -> {:?}",
                            offset, lc.line, lc.col, back);
                    }
                }
            }

            /// Grapheme columns are always <= char columns, because a single
            /// grapheme cluster can span multiple Unicode code points.
            #[test]
            fn grapheme_col_le_char_col(text in arb_unicode_doc()) {
                for line_text in text.lines() {
                    let char_count = line_text.chars().count();
                    for char_col in 0..=char_count {
                        let grapheme_col = char_col_to_grapheme_col(line_text, char_col);
                        prop_assert!(grapheme_col <= char_col,
                            "grapheme_col ({}) > char_col ({}) for line {:?}",
                            grapheme_col, char_col, line_text);
                    }
                }
            }
        }
    }

    #[test]
    fn single_line_no_newline() {
        let text = "abc";
        assert_eq!(line_col_to_byte(text, 0, 0), 0);
        assert_eq!(line_col_to_byte(text, 0, 3), 3);
        assert_eq!(byte_to_line_col(text, 0), CharLineCol::new(0, 0));
        assert_eq!(byte_to_line_col(text, 3), CharLineCol::new(0, 3));
    }

    #[test]
    fn trailing_newline() {
        let text = "abc\n";
        // Byte 3 is '\n' on line 0
        assert_eq!(byte_to_line_col(text, 3), CharLineCol::new(0, 3));
        // Byte 4 is start of line 1 (empty line)
        assert_eq!(byte_to_line_col(text, 4), CharLineCol::new(1, 0));
    }

    #[test]
    fn multiple_empty_lines() {
        let text = "\n\n\n";
        assert_eq!(line_col_to_byte(text, 0, 0), 0);
        assert_eq!(line_col_to_byte(text, 1, 0), 1);
        assert_eq!(line_col_to_byte(text, 2, 0), 2);
        assert_eq!(line_col_to_byte(text, 3, 0), 3);
        assert_eq!(byte_to_line_col(text, 0), CharLineCol::new(0, 0));
        assert_eq!(byte_to_line_col(text, 1), CharLineCol::new(1, 0));
        assert_eq!(byte_to_line_col(text, 2), CharLineCol::new(2, 0));
        assert_eq!(byte_to_line_col(text, 3), CharLineCol::new(3, 0));
    }

    // ─── find_line_start tests ──────────────────────────────────────

    #[test]
    fn find_line_start_line_zero() {
        assert_eq!(find_line_start("hello\nworld", 0), Some(0));
    }

    #[test]
    fn find_line_start_second_line() {
        assert_eq!(find_line_start("hello\nworld", 1), Some(6));
    }

    #[test]
    fn find_line_start_third_line() {
        assert_eq!(find_line_start("a\nb\nc", 2), Some(4));
    }

    #[test]
    fn find_line_start_out_of_range() {
        assert_eq!(find_line_start("hello\nworld", 5), None);
    }

    #[test]
    fn find_line_start_empty_text() {
        assert_eq!(find_line_start("", 0), Some(0));
        assert_eq!(find_line_start("", 1), None);
    }

    #[test]
    fn find_line_start_only_newlines() {
        let text = "\n\n\n";
        assert_eq!(find_line_start(text, 0), Some(0));
        assert_eq!(find_line_start(text, 1), Some(1));
        assert_eq!(find_line_start(text, 2), Some(2));
        assert_eq!(find_line_start(text, 3), Some(3));
        assert_eq!(find_line_start(text, 4), None);
    }

    // ─── line_text_at tests ─────────────────────────────────────────

    #[test]
    fn line_text_at_first_line() {
        assert_eq!(line_text_at("hello\nworld", 0), "hello");
    }

    #[test]
    fn line_text_at_second_line() {
        assert_eq!(line_text_at("hello\nworld", 6), "world");
    }

    #[test]
    fn line_text_at_no_trailing_newline() {
        assert_eq!(line_text_at("abc", 0), "abc");
    }

    #[test]
    fn line_text_at_empty_line() {
        // "a\n\nb" — line at offset 2 is the empty line between newlines
        assert_eq!(line_text_at("a\n\nb", 2), "");
    }

    #[test]
    fn line_text_at_last_empty_line() {
        // "abc\n" — line at offset 4 is the empty trailing line
        assert_eq!(line_text_at("abc\n", 4), "");
    }

    // ─── Incremental LineIndex update tests ───────────────────────────

    #[test]
    fn incremental_insert_no_newlines() {
        let text = "hello\nworld";
        let mut index = LineIndex::new(text);
        assert_eq!(index.line_count(), 2);
        // Insert " cruel" at offset 5 -> "hello cruel\nworld"
        index.apply_insert(5, " cruel");
        assert_eq!(index.line_count(), 2);
        assert_eq!(index.text_len, 17);
    }

    #[test]
    fn incremental_insert_with_newline() {
        let text = "hello\nworld";
        let mut index = LineIndex::new(text);
        // Insert "\nnew\n" at offset 5 -> "hello\nnew\n\nworld"
        index.apply_insert(5, "\nnew\n");
        assert_eq!(index.line_count(), 4);
    }

    #[test]
    fn incremental_delete_within_line() {
        let text = "hello cruel\nworld";
        let mut index = LineIndex::new(text);
        // Delete " cruel" (offset 5..11) -> "hello\nworld"
        index.apply_delete(5, 11);
        assert_eq!(index.line_count(), 2);
        assert_eq!(index.text_len, 11);
    }

    #[test]
    fn incremental_delete_across_lines() {
        let text = "hello\ncruel\nworld";
        let mut index = LineIndex::new(text);
        // Delete "cruel\n" (offset 6..12) -> "hello\nworld"
        index.apply_delete(6, 12);
        assert_eq!(index.line_count(), 2);
    }

    #[test]
    fn incremental_insert_at_start() {
        let text = "hello";
        let mut index = LineIndex::new(text);
        index.apply_insert(0, "prefix\n");
        assert_eq!(index.line_count(), 2);
    }

    #[test]
    fn incremental_empty_operations() {
        let text = "hello\nworld";
        let mut index = LineIndex::new(text);
        let original_count = index.line_count();
        index.apply_insert(3, "");
        assert_eq!(index.line_count(), original_count);
        index.apply_delete(3, 3);
        assert_eq!(index.line_count(), original_count);
    }
}
