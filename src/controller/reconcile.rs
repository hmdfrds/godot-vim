//! Shared text-change reconciliation for external edits (IME commit,
//! completion acceptance, Find-and-Replace, etc.).
//!
//! Diffs before/after text via common-prefix/common-suffix, constructs an
//! [`ExternalEdit`], and feeds it to the engine for undo/dot-repeat tracking.

use vim_core::execution::{ExternalEdit, ExternalEditKind, VimEngine};
use vim_core::primitives::{Offset, Range};

/// Result of diffing before/after text.
#[derive(Debug, PartialEq)]
struct TextDiff<'a> {
    /// Byte range `(start, end)` in `before_text` that was deleted.
    deleted_range: (usize, usize),
    /// Slice of `before_text` that was deleted.
    deleted_text: &'a str,
    /// Slice of `after_text` that was inserted.
    inserted_text: &'a str,
}

/// Pure diff: compute the minimal contiguous edit between two texts.
///
/// `cursor_byte` is the byte offset of the caret in `after_text` — used
/// to bound the common-prefix so it doesn't extend past the edit point.
///
/// Returns `None` if texts are identical or the diff produces an inverted
/// range (overlapping prefix/suffix).
fn diff_texts<'a>(
    before_text: &'a str,
    after_text: &'a str,
    cursor_byte: usize,
) -> Option<TextDiff<'a>> {
    if before_text == after_text {
        return None;
    }

    // Common prefix (bytes before the edit region), snapped to char boundary.
    let raw_prefix = before_text
        .bytes()
        .zip(after_text.bytes())
        .take_while(|(a, b)| a == b)
        .count()
        .min(cursor_byte);
    let common_prefix = snap_to_char_boundary_down(before_text, raw_prefix);

    // Common suffix (bytes after the edit region). cursor_byte is a
    // position in after_text, so bound suffix length by the bytes AFTER
    // the cursor in after_text. The second term prevents overlap with
    // the prefix in before_text.
    let max_suffix = after_text
        .len()
        .saturating_sub(cursor_byte)
        .min(before_text.len().saturating_sub(common_prefix));
    let raw_suffix = before_text
        .bytes()
        .rev()
        .zip(after_text.bytes().rev())
        .take_while(|(a, b)| a == b)
        .count()
        .min(max_suffix);
    let common_suffix =
        before_text.len() - snap_to_char_boundary_up(before_text, before_text.len() - raw_suffix);

    let deleted_end = before_text.len().saturating_sub(common_suffix);
    let inserted_end = after_text.len().saturating_sub(common_suffix);

    if common_prefix > deleted_end || common_prefix > inserted_end {
        log::trace!("reconcile: inverted range, skipping");
        return None;
    }

    Some(TextDiff {
        deleted_range: (common_prefix, deleted_end),
        deleted_text: &before_text[common_prefix..deleted_end],
        inserted_text: &after_text[common_prefix..inserted_end],
    })
}

/// Diff `before_text` against `after_text` and reconcile with the engine
/// via [`VimEngine::apply_external_edit_with_recording`].
///
/// `cursor_byte` is the byte offset of the caret in `after_text` (used
/// to set the engine's cursor position post-edit).
///
/// No-ops when the texts are identical or when the diff produces an
/// inverted range (overlapping prefix/suffix).
pub(crate) fn reconcile_external_text_change(
    engine: &mut VimEngine,
    before_text: &str,
    after_text: &str,
    cursor_byte: usize,
    kind: ExternalEditKind,
) {
    let Some(diff) = diff_texts(before_text, after_text, cursor_byte) else {
        log::trace!("reconcile: no text change, skipping");
        return;
    };

    log::debug!(
        "reconcile: deleted={}b inserted={}b",
        diff.deleted_range.1 - diff.deleted_range.0,
        diff.inserted_text.len(),
    );

    let edit = ExternalEdit::new(
        Range::new(
            Offset::new(diff.deleted_range.0),
            Offset::new(diff.deleted_range.1),
        ),
        diff.inserted_text,
        Offset::new(cursor_byte),
        kind,
    );
    // Response discarded: the host (CodeEdit) already applied the text change.
    // Processing effects here would double-apply them.
    let _response = engine.apply_external_edit_with_recording(edit, diff.deleted_text);
}

/// Snap a byte offset down to the nearest char boundary in `s`.
fn snap_to_char_boundary_down(s: &str, offset: usize) -> usize {
    let mut pos = offset.min(s.len());
    while pos > 0 && !s.is_char_boundary(pos) {
        pos -= 1;
    }
    pos
}

/// Snap a byte offset up to the nearest char boundary in `s`.
fn snap_to_char_boundary_up(s: &str, offset: usize) -> usize {
    let mut pos = offset.min(s.len());
    while pos < s.len() && !s.is_char_boundary(pos) {
        pos += 1;
    }
    pos
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── Crash tests (engine integration) ────────────────────────────────

    #[test]
    fn reconcile_simple_insertion() {
        let mut engine = VimEngine::new();
        reconcile_external_text_change(
            &mut engine,
            "hello\n",
            "hello world\n",
            11,
            ExternalEditKind::HostNotified,
        );
    }

    #[test]
    fn reconcile_no_change_is_noop() {
        let mut engine = VimEngine::new();
        reconcile_external_text_change(
            &mut engine,
            "hello\n",
            "hello\n",
            5,
            ExternalEditKind::HostNotified,
        );
    }

    #[test]
    fn reconcile_cjk_insertion() {
        let mut engine = VimEngine::new();
        reconcile_external_text_change(
            &mut engine,
            "abc\n",
            "ab你好c\n",
            8,
            ExternalEditKind::HostNotified,
        );
    }

    // ── diff_texts pure assertions ──────────────────────────────────────

    #[test]
    fn diff_simple_insertion() {
        // cursor_byte=11: after "hello world" in "hello world\n" (12 bytes).
        // Suffix correctly absorbs the trailing "\n" even though cursor > before.len().
        let diff = diff_texts("hello\n", "hello world\n", 11).unwrap();
        assert_eq!(diff.deleted_range, (5, 5));
        assert_eq!(diff.deleted_text, "");
        assert_eq!(diff.inserted_text, " world");
    }

    #[test]
    fn diff_simple_deletion() {
        let diff = diff_texts("hello world\n", "hello\n", 5).unwrap();
        assert_eq!(diff.deleted_range, (5, 11));
        assert_eq!(diff.deleted_text, " world");
        assert_eq!(diff.inserted_text, "");
    }

    #[test]
    fn diff_cjk_insertion() {
        // cursor_byte=8: after "ab你好" (2 ASCII + 6 UTF-8 bytes) in "ab你好c\n".
        // Suffix correctly absorbs "c\n" even though cursor > before.len().
        let diff = diff_texts("abc\n", "ab你好c\n", 8).unwrap();
        assert_eq!(diff.deleted_range, (2, 2));
        assert_eq!(diff.deleted_text, "");
        assert_eq!(diff.inserted_text, "你好");
    }

    #[test]
    fn diff_replacement() {
        let diff = diff_texts("hello\n", "hullo\n", 2).unwrap();
        assert_eq!(diff.deleted_range, (1, 2));
        assert_eq!(diff.deleted_text, "e");
        assert_eq!(diff.inserted_text, "u");
    }

    #[test]
    fn diff_identical_returns_none() {
        assert!(diff_texts("hello\n", "hello\n", 5).is_none());
    }

    #[test]
    fn diff_empty_to_text() {
        let diff = diff_texts("", "hello", 5).unwrap();
        assert_eq!(diff.deleted_range, (0, 0));
        assert_eq!(diff.deleted_text, "");
        assert_eq!(diff.inserted_text, "hello");
    }

    #[test]
    fn diff_text_to_empty() {
        let diff = diff_texts("hello", "", 0).unwrap();
        assert_eq!(diff.deleted_range, (0, 5));
        assert_eq!(diff.deleted_text, "hello");
        assert_eq!(diff.inserted_text, "");
    }

    #[test]
    fn diff_single_char_replacement_at_start() {
        // cursor_byte=0 clamps prefix to 0; suffix absorbs "bcdef" (5 bytes).
        // Result: 1-byte edit region at the start.
        let diff = diff_texts("abcdef", "xbcdef", 0).unwrap();
        assert_eq!(diff.deleted_range, (0, 1));
        assert_eq!(diff.deleted_text, "a");
        assert_eq!(diff.inserted_text, "x");
    }

    // ── Regression: completion dot-repeat (cursor past before.len()) ────

    #[test]
    fn diff_completion_prefix_replacement() {
        // Simulates: user types "p" on a new line, autocomplete replaces
        // with "physics_interpolation_mode". cursor_byte is at the end of
        // the completed word (past before_text.len()).
        //
        // Before: "func f():\n    p\n    print(\"Test\")\n"
        // After:  "func f():\n    physics_interpolation_mode\n    print(\"Test\")\n"
        let before = "func f():\n    p\n    print(\"Test\")\n";
        let after = "func f():\n    physics_interpolation_mode\n    print(\"Test\")\n";
        let cursor_byte = "func f():\n    physics_interpolation_mode".len(); // 40

        let diff = diff_texts(before, after, cursor_byte).unwrap();

        // The prefix "p" is shared, so the diff is a pure insertion after "p".
        // Critically, the suffix "\n    print(\"Test\")\n" must be absorbed —
        // otherwise dot-repeat records the wrong text.
        assert_eq!(
            diff.deleted_range,
            (15, 15),
            "nothing deleted (pure insertion)"
        );
        assert_eq!(diff.deleted_text, "");
        assert_eq!(
            diff.inserted_text, "hysics_interpolation_mode",
            "only the suffix beyond the shared 'p' prefix"
        );
    }

    // ── snap edge cases ─────────────────────────────────────────────────

    #[test]
    fn snap_down_on_boundary() {
        assert_eq!(snap_to_char_boundary_down("hello", 3), 3);
    }

    #[test]
    fn snap_down_mid_utf8() {
        let s = "a你b";
        assert_eq!(snap_to_char_boundary_down(s, 2), 1);
    }

    #[test]
    fn snap_up_mid_utf8() {
        let s = "a你b";
        assert_eq!(snap_to_char_boundary_up(s, 2), 4);
    }

    #[test]
    fn snap_down_empty_string() {
        assert_eq!(snap_to_char_boundary_down("", 0), 0);
        assert_eq!(snap_to_char_boundary_down("", 5), 0);
    }

    #[test]
    fn snap_up_empty_string() {
        assert_eq!(snap_to_char_boundary_up("", 0), 0);
        assert_eq!(snap_to_char_boundary_up("", 5), 0);
    }
}
