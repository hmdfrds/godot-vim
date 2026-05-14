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
    let regions = decompose_multi_site_diff(before_text, after_text, cursor_byte);
    if regions.is_empty() {
        log::trace!("reconcile: no text change, skipping");
        return;
    }

    if regions.len() == 1 {
        let r = &regions[0];
        let deleted_text = &before_text[r.offset..r.offset + r.old_len];

        log::debug!(
            "reconcile: single-site deleted={}b inserted={}b",
            r.old_len,
            r.inserted.len(),
        );

        let edit = ExternalEdit::new(
            Range::new(Offset::new(r.offset), Offset::new(r.offset + r.old_len)),
            r.inserted,
            Offset::new(cursor_byte),
            kind,
        );
        // Response discarded: the host (CodeEdit) already applied the text change.
        // Processing effects here would double-apply them.
        let _response = engine.apply_external_edit_with_recording(edit, deleted_text);
    } else {
        log::debug!(
            "reconcile: multi-site batch ({} regions)",
            regions.len(),
        );

        let edits: Vec<ExternalEdit> = regions
            .iter()
            .map(|r| {
                ExternalEdit::new(
                    Range::new(Offset::new(r.offset), Offset::new(r.offset + r.old_len)),
                    r.inserted,
                    Offset::new(cursor_byte),
                    kind,
                )
            })
            .collect();
        let deleted_texts: Vec<&str> = regions
            .iter()
            .map(|r| &before_text[r.offset..r.offset + r.old_len])
            .collect();
        // Response discarded: the host (CodeEdit) already applied the text change.
        // Processing effects here would double-apply them.
        let _response = engine.apply_external_edits_batch(edits, &deleted_texts);
    }
}

/// A contiguous edit region within a larger diff, representing one site
/// where text changed between before and after.
#[derive(Debug, Clone)]
pub(crate) struct EditRegion<'a> {
    pub offset: usize,
    pub old_len: usize,
    pub inserted: &'a str,
}

/// Decompose a large contiguous diff into multiple edit regions using greedy
/// line-level matching. This enables correct mark remapping for Find-Replace-All,
/// where a single `diff_texts` call would merge all replacement sites into one
/// contiguous change.
///
/// Returns an empty Vec if the texts are identical, or a single-element Vec
/// if the diff is small or cannot be meaningfully decomposed.
pub(crate) fn decompose_multi_site_diff<'a>(
    before: &'a str,
    after: &'a str,
    cursor_byte: usize,
) -> Vec<EditRegion<'a>> {
    // Step 1: get the outer diff envelope.
    let Some(diff) = diff_texts(before, after, cursor_byte) else {
        return Vec::new();
    };

    let old_middle = diff.deleted_text;
    let new_middle = diff.inserted_text;
    let base_offset = diff.deleted_range.0;

    // Step 3: fast path — small diff or one side empty → single region.
    if old_middle.len() + new_middle.len() < 200
        || old_middle.is_empty()
        || new_middle.is_empty()
    {
        return vec![EditRegion {
            offset: base_offset,
            old_len: old_middle.len(),
            inserted: new_middle,
        }];
    }

    // Step 4: split both middles into lines.
    let old_lines: Vec<&str> = old_middle.split_inclusive('\n').collect();
    let new_lines: Vec<&str> = new_middle.split_inclusive('\n').collect();

    // Step 5: greedy synchronized matching with bounded lookahead.
    // Collect hunks as (old_line_start, old_line_end, new_line_start, new_line_end).
    let mut hunks: Vec<(usize, usize, usize, usize)> = Vec::new();
    let mut oi = 0usize; // old line index
    let mut ni = 0usize; // new line index

    while oi < old_lines.len() && ni < new_lines.len() {
        if old_lines[oi] == new_lines[ni] {
            // Lines match — advance both.
            oi += 1;
            ni += 1;
            continue;
        }

        // Mismatch — start a hunk.
        let hunk_old_start = oi;
        let hunk_new_start = ni;

        // Try to re-sync with bounded lookahead (3 lines).
        let mut synced = false;
        'outer: for look in 1usize..=3 {
            // Symmetric: both advance by `look`.
            if oi + look < old_lines.len()
                && ni + look < new_lines.len()
                && old_lines[oi + look] == new_lines[ni + look]
            {
                // Found sync point: hunk covers [oi..oi+look) and [ni..ni+look).
                oi += look;
                ni += look;
                synced = true;
                break 'outer;
            }
            // Old advanced more.
            for od in 1..=look {
                let nd = look - od;
                if oi + od < old_lines.len()
                    && ni + nd < new_lines.len()
                    && old_lines[oi + od] == new_lines[ni + nd]
                {
                    oi += od;
                    ni += nd;
                    synced = true;
                    break 'outer;
                }
            }
        }

        if !synced {
            // No sync within lookahead — consume one line from each.
            oi += 1;
            ni += 1;
        }

        hunks.push((hunk_old_start, oi, hunk_new_start, ni));
    }

    // Trailing mismatch: if one side has leftover lines, that's a final hunk.
    if oi < old_lines.len() || ni < new_lines.len() {
        let hunk_old_start = oi;
        let hunk_new_start = ni;
        oi = old_lines.len();
        ni = new_lines.len();
        hunks.push((hunk_old_start, oi, hunk_new_start, ni));
    }

    // Step 6: fallback — if only one hunk, return single EditRegion.
    if hunks.len() <= 1 {
        return vec![EditRegion {
            offset: base_offset,
            old_len: old_middle.len(),
            inserted: new_middle,
        }];
    }

    // Step 7: convert line-level hunks to byte-level EditRegions.
    // Precompute cumulative byte offsets for each line index.
    let old_line_offsets: Vec<usize> = {
        let mut offsets = Vec::with_capacity(old_lines.len() + 1);
        offsets.push(0);
        for line in &old_lines {
            offsets.push(offsets.last().unwrap() + line.len());
        }
        offsets
    };
    let new_line_offsets: Vec<usize> = {
        let mut offsets = Vec::with_capacity(new_lines.len() + 1);
        offsets.push(0);
        for line in &new_lines {
            offsets.push(offsets.last().unwrap() + line.len());
        }
        offsets
    };

    let regions: Vec<EditRegion<'a>> = hunks
        .iter()
        .map(|&(os, oe, ns, ne)| {
            let old_byte_start = old_line_offsets[os];
            let old_byte_end = old_line_offsets[oe];
            let new_byte_start = new_line_offsets[ns];
            let new_byte_end = new_line_offsets[ne];
            EditRegion {
                offset: base_offset + old_byte_start,
                old_len: old_byte_end - old_byte_start,
                inserted: &new_middle[new_byte_start..new_byte_end],
            }
        })
        .collect();

    regions
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
        engine.set_shadow_text("hello\n");
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
        engine.set_shadow_text("hello\n");
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
        engine.set_shadow_text("abc\n");
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

    // ── decompose_multi_site_diff ──────────────────────────────────────

    #[test]
    fn decompose_find_replace_all() {
        let (before, after) = multi_site_pair();
        let regions = decompose_multi_site_diff(before, after, usize::MAX);
        assert!(
            regions.len() >= 3,
            "expected 3+ regions, got {}",
            regions.len()
        );
    }

    #[test]
    fn decompose_single_edit_returns_one() {
        let before = "hello world";
        let after = "hello brave world";
        let regions = decompose_multi_site_diff(before, after, usize::MAX);
        assert_eq!(regions.len(), 1);
    }

    #[test]
    fn decompose_identical_returns_empty() {
        let before = "hello";
        let after = "hello";
        let regions = decompose_multi_site_diff(before, after, usize::MAX);
        assert!(regions.is_empty());
    }

    #[test]
    fn decompose_small_change_returns_one() {
        // Under 200 bytes → single region fast path.
        let before = "abc";
        let after = "axc";
        let regions = decompose_multi_site_diff(before, after, usize::MAX);
        assert_eq!(regions.len(), 1);
    }

    /// Helper: returns (before, after) text pair large enough to exceed the
    /// 200-byte fast-path, with 3 replacement sites separated by matching
    /// context lines.
    fn multi_site_pair() -> (&'static str, &'static str) {
        (
            "\
hello world\n\
the quick brown fox jumps over the lazy dog near the riverbank\n\
foo bar baz quux corge grault garply waldo fred plugh\n\
hello again\n\
the quick brown fox jumps over the lazy dog near the riverbank\n\
foo bar baz quux corge grault garply waldo fred plugh\n\
hello end\n",
            "\
hi world\n\
the quick brown fox jumps over the lazy dog near the riverbank\n\
foo bar baz quux corge grault garply waldo fred plugh\n\
hi again\n\
the quick brown fox jumps over the lazy dog near the riverbank\n\
foo bar baz quux corge grault garply waldo fred plugh\n\
hi end\n",
        )
    }

    #[test]
    fn decompose_regions_index_into_before() {
        let (before, after) = multi_site_pair();
        let regions = decompose_multi_site_diff(before, after, usize::MAX);

        for (i, region) in regions.iter().enumerate() {
            let end = region.offset + region.old_len;
            assert!(
                end <= before.len(),
                "region {i}: offset {} + old_len {} = {} exceeds before.len() {}",
                region.offset,
                region.old_len,
                end,
                before.len()
            );
            // Verify the slice is valid UTF-8 (indexing into &str panics otherwise).
            let _old_slice = &before[region.offset..end];
        }
    }

    #[test]
    fn decompose_inserted_corresponds_to_after() {
        let (before, after) = multi_site_pair();
        let regions = decompose_multi_site_diff(before, after, usize::MAX);

        // Reconstruct after from before by applying regions in reverse order.
        let mut result = before.to_string();
        for region in regions.iter().rev() {
            let end = region.offset + region.old_len;
            result.replace_range(region.offset..end, region.inserted);
        }
        assert_eq!(result, after);
    }

    #[test]
    fn decompose_regions_sorted_and_non_overlapping() {
        let (before, after) = multi_site_pair();
        let regions = decompose_multi_site_diff(before, after, usize::MAX);

        for i in 1..regions.len() {
            let prev_end = regions[i - 1].offset + regions[i - 1].old_len;
            assert!(
                prev_end <= regions[i].offset,
                "regions overlap: region {} ends at {}, region {} starts at {}",
                i - 1,
                prev_end,
                i,
                regions[i].offset
            );
        }
    }

    #[test]
    fn decompose_empty_old_middle_returns_single() {
        // Pure insertion (no deleted text) → single region.
        // cursor_byte = 5: after "abXYZ" so the suffix "c" is absorbed.
        let before = "abc";
        let after = "abXYZc";
        let regions = decompose_multi_site_diff(before, after, 5);
        assert_eq!(regions.len(), 1);
        assert_eq!(regions[0].offset, 2);
        assert_eq!(regions[0].old_len, 0);
        assert_eq!(regions[0].inserted, "XYZ");
    }

    #[test]
    fn decompose_empty_new_middle_returns_single() {
        // Pure deletion (no inserted text) → single region.
        let before = "abXYZc";
        let after = "abc";
        let regions = decompose_multi_site_diff(before, after, 2);
        assert_eq!(regions.len(), 1);
        assert_eq!(regions[0].old_len, 3);
        assert_eq!(regions[0].inserted, "");
    }
}
