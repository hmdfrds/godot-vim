//! Shared text-change reconciliation for external edits (IME commit,
//! completion acceptance, Find-and-Replace, etc.).
//!
//! Diffs before/after text via common-prefix/common-suffix, constructs an
//! [`ExternalEdit`], and feeds it to the engine for undo/dot-repeat tracking.

use vim_core::execution::{ExternalEdit, ExternalEditKind, VimEngine};
use vim_core::primitives::{Offset, Range};

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
) {
    if before_text == after_text {
        log::trace!("reconcile: no text change, skipping");
        return;
    }

    // Common prefix (bytes before the edit region), snapped to char boundary.
    let raw_prefix = before_text
        .bytes()
        .zip(after_text.bytes())
        .take_while(|(a, b)| a == b)
        .count()
        .min(cursor_byte);
    let common_prefix = snap_to_char_boundary_down(before_text, raw_prefix);

    // Common suffix (bytes after the edit region). Clamped so that
    // prefix + suffix never exceeds the shorter text — otherwise
    // overlap produces inverted ranges.
    let max_suffix = before_text.len().saturating_sub(cursor_byte)
        .min(after_text.len().saturating_sub(common_prefix));
    let raw_suffix = before_text
        .bytes()
        .rev()
        .zip(after_text.bytes().rev())
        .take_while(|(a, b)| a == b)
        .count()
        .min(max_suffix);
    let common_suffix = before_text.len()
        - snap_to_char_boundary_up(before_text, before_text.len() - raw_suffix);

    let deleted_end = before_text.len().saturating_sub(common_suffix);
    let inserted_end = after_text.len().saturating_sub(common_suffix);

    if common_prefix > deleted_end || common_prefix > inserted_end {
        log::trace!("reconcile: inverted range, skipping");
        return;
    }

    let deleted_range = Range::new(
        Offset::new(common_prefix),
        Offset::new(deleted_end),
    );
    let deleted_text = &before_text[common_prefix..deleted_end];
    let inserted_text = &after_text[common_prefix..inserted_end];

    log::debug!(
        "reconcile: deleted={}b inserted={}b",
        deleted_end - common_prefix, inserted_end - common_prefix
    );

    let edit = ExternalEdit::new(
        deleted_range,
        inserted_text,
        Offset::new(cursor_byte),
        ExternalEditKind::PasteOrIme,
    );
    // Response discarded: the host (CodeEdit) already applied the text change.
    // Processing effects here would double-apply them.
    let _response = engine.apply_external_edit_with_recording(edit, deleted_text);
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

    #[test]
    fn reconcile_simple_insertion() {
        let mut engine = VimEngine::new();
        reconcile_external_text_change(
            &mut engine,
            "hello\n",
            "hello world\n",
            11,
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
        );
    }

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
}
