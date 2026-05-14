//! Applies cursor and selection effects to CodeEdit (set cursor, set/clear
//! selection in char/line/block mode).

use vim_core::primitives::SelectionShape;

use crate::bridge::codec::{i32_to_usize, usize_to_i32, DocumentView};
use crate::bridge::port::{TextEditorPort, ViewportAdjust};
use crate::types::CharLineCol;

/// Move the caret to the given byte `offset` and scroll the viewport to follow,
/// enforcing the Vim `scrolloff` margin.
///
/// Uses `set_caret_line_unfold` so that if the target line is inside a
/// folded region, Godot will unfold it to keep the cursor visible.
///
/// After positioning the caret, checks whether the cursor is within
/// `scrolloff` lines of the viewport edge. If so, scrolls the viewport
/// to restore the margin. Godot's `adjust_viewport_to_caret` only ensures
/// the cursor is *somewhere* on screen — it does not enforce a margin.
pub(crate) fn handle_set_cursor(
    editor: &mut impl TextEditorPort,
    doc: &DocumentView,
    offset: usize,
    scrolloff: i32,
) {
    let pos = doc.line_index.byte_to_line_col(doc.text, offset);
    log::trace!(
        "set_cursor: offset={} -> line={} col={}",
        offset,
        pos.line,
        pos.col
    );
    editor.set_caret_line_unfold(pos.line, ViewportAdjust::Adjust);
    editor.set_caret_column(pos.col);

    // adjust_viewport_to_caret only guarantees visibility, not scrolloff margin.
    editor.adjust_viewport_to_caret();
    enforce_scrolloff(editor, pos.line, scrolloff);
}

/// Scroll the viewport so the cursor keeps at least `scrolloff` lines of
/// context above and below, matching Vim's `set scrolloff=N` behavior.
///
/// Only scrolls the minimum amount needed — if the cursor is already within
/// the margin, the viewport stays put.
pub(crate) fn enforce_scrolloff(editor: &mut impl TextEditorPort, cursor_line: i32, scrolloff: i32) {
    if scrolloff <= 0 {
        return;
    }

    let first_visible = editor.get_first_visible_line();
    let visible_count = editor.get_visible_line_count();

    // When the viewport is too small for full top+bottom margins, center instead.
    if visible_count <= scrolloff * 2 {
        let center = cursor_line - visible_count / 2;
        let target = center.max(0);
        if first_visible != target {
            editor.set_v_scroll(target.into());
        }
        return;
    }

    let top_margin = first_visible + scrolloff;
    let bot_margin = first_visible + visible_count - 1 - scrolloff;

    if cursor_line < top_margin {
        let target = (cursor_line - scrolloff).max(0);
        editor.set_v_scroll(target.into());
    } else if cursor_line > bot_margin {
        let target = cursor_line - visible_count + 1 + scrolloff;
        editor.set_v_scroll(target.into());
    }
}

/// Set the visual selection between `anchor` and `head` (byte offsets),
/// adapting the CodeEdit selection to the given `SelectionShape`.
///
/// ## Vim-inclusive vs Godot-exclusive selection model
///
/// Vim's visual selection is **inclusive** on both ends: both the anchor
/// character and the head character are part of the selection. Godot's
/// `CodeEdit::select(origin_line, origin_col, caret_line, caret_col)`
/// treats the selection as the range `[origin, caret)` — the character at
/// the caret column is NOT included in the highlight.
///
/// For `Char` mode, we apply a +1 offset to the exclusive end so the
/// highlight covers the full Vim-inclusive range. This is safe because:
///
/// 1. **Shell-owned canonical selection** — the engine's `(anchor, head)`
///    is stored in `BufferState.visual_selection` and fed back into
///    `InputContext` on the next keystroke, bypassing Godot's lossy
///    round-trip entirely. The +1 is rendering-only.
///
/// 2. **VimCursor overlay via `override_pos`** — the cursor overlay reads
///    the engine's actual head from `visual_head_pos`, not from Godot's
///    caret. The +1 shift of the Godot caret doesn't affect cursor display.
///
/// For `Block` mode, only the head line gets a Godot selection (min_col to
/// max_col+1). The full block highlight is rendered by `BlockVisualOverlay`,
/// driven by `UiCoordinator::update` via `UiSnapshot::block_visual`.
///
/// After setting the selection, ensures the head line is visible via
/// unfold and scrolls the viewport to follow.
pub(crate) fn handle_set_selection(
    editor: &mut impl TextEditorPort,
    doc: &DocumentView,
    anchor: usize,
    head: usize,
    shape: SelectionShape,
) {
    let anchor_pos = doc.line_index.byte_to_line_col(doc.text, anchor);
    let head_pos = doc.line_index.byte_to_line_col(doc.text, head);
    let (anchor_line, anchor_col) = (anchor_pos.line, anchor_pos.col);
    let (head_line, head_col) = (head_pos.line, head_pos.col);

    // Block mode is now rendered by a transparent overlay (BlockVisualOverlay),
    // not secondary carets. All shapes use primary caret only.
    editor.remove_secondary_carets();

    match shape {
        SelectionShape::Char => {
            if head >= anchor {
                let head_line_len = usize_to_i32(
                    doc.line_index
                        .line_char_count(doc.text, i32_to_usize(head_line)),
                );
                editor.select(
                    CharLineCol::new(anchor_line, anchor_col),
                    CharLineCol::new(head_line, (head_col + 1).min(head_line_len)),
                );
            } else {
                let anchor_line_len = usize_to_i32(
                    doc.line_index
                        .line_char_count(doc.text, i32_to_usize(anchor_line)),
                );
                editor.select(
                    CharLineCol::new(anchor_line, (anchor_col + 1).min(anchor_line_len)),
                    CharLineCol::new(head_line, head_col),
                );
            }
        }
        SelectionShape::Line => {
            // Line mode selects full lines. Godot's caret follows the `to`
            // end of select(), so swap origin/caret to preserve direction.
            let top_line = anchor_line.min(head_line);
            let bot_line = anchor_line.max(head_line);
            let bot_end_col = usize_to_i32(
                doc.line_index
                    .line_char_count(doc.text, i32_to_usize(bot_line)),
            );
            if head_line >= anchor_line {
                editor.select(
                    CharLineCol::new(top_line, 0),
                    CharLineCol::new(bot_line, bot_end_col),
                );
            } else {
                editor.select(
                    CharLineCol::new(bot_line, bot_end_col),
                    CharLineCol::new(top_line, 0),
                );
            }
        }
        SelectionShape::Block => {
            // Block visual rendering is handled by BlockVisualOverlay (driven
            // by UiCoordinator::update via UiSnapshot::block_visual). Here we
            // only set the primary caret selection on the head line so Godot's
            // native selection highlight shows the head-line portion of the block.
            let min_col = anchor_col.min(head_col);
            let max_col = anchor_col.max(head_col);
            let head_line_len = usize_to_i32(
                doc.line_index
                    .line_char_count(doc.text, i32_to_usize(head_line)),
            );
            let (render_from, render_to) = if head_col <= anchor_col {
                ((max_col + 1).min(head_line_len), min_col)
            } else {
                (min_col, (max_col + 1).min(head_line_len))
            };
            editor.select(
                CharLineCol::new(head_line, render_from),
                CharLineCol::new(head_line, render_to),
            );
        }
        _ => {
            log::warn!(
                "Unknown SelectionShape variant {:?} — treating as block (best-effort)",
                shape
            );
            let min_col = anchor_col.min(head_col);
            let max_col = anchor_col.max(head_col);
            let head_line_len = usize_to_i32(
                doc.line_index
                    .line_char_count(doc.text, i32_to_usize(head_line)),
            );
            let (render_from, render_to) = if head_col <= anchor_col {
                ((max_col + 1).min(head_line_len), min_col)
            } else {
                (min_col, (max_col + 1).min(head_line_len))
            };
            editor.select(
                CharLineCol::new(head_line, render_from),
                CharLineCol::new(head_line, render_to),
            );
        }
    }

    // Unfold the head line if it's hidden inside a fold region.
    editor.set_caret_line_unfold(head_line, ViewportAdjust::NoAdjust);

    // Viewport scrolling strategy depends on selection shape because Godot's
    // caret after select() may not match the Vim head position.
    match shape {
        SelectionShape::Char => {
            // In char mode, Godot's caret tracks the head — viewport follows.
            editor.adjust_viewport_to_caret();
        }
        _ => {
            // In line/block mode, Godot normalizes the caret to the selection
            // boundary (e.g., always bottom for line mode), which diverges
            // from the Vim head. Manually scroll to keep head_line visible.
            let first_visible = editor.get_first_visible_line();
            let visible_count = editor.get_visible_line_count();
            if head_line < first_visible || head_line >= first_visible + visible_count {
                let target = head_line.saturating_sub(visible_count / 2).max(0);
                editor.set_v_scroll(target.into());
            }
        }
    }
}

/// Deselect all text and remove secondary carets (return to normal caret-only state).
pub(crate) fn handle_clear_selection(editor: &mut impl TextEditorPort) {
    log::trace!("clear_selection");
    editor.remove_secondary_carets();
    editor.deselect();
}

#[cfg(test)]
const HANDLED_SELECTION_SHAPES: &[vim_core::primitives::SelectionShape] = &[
    vim_core::primitives::SelectionShape::Char,
    vim_core::primitives::SelectionShape::Line,
    vim_core::primitives::SelectionShape::Block,
];

#[cfg(test)]
mod selection_shape_coverage_tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn selection_shape_dispatch_covers_all_variants() {
        let handled: HashSet<_> = HANDLED_SELECTION_SHAPES.iter().copied().collect();
        let all: HashSet<_> = SelectionShape::ALL.iter().copied().collect();
        let missing: Vec<_> = all.difference(&handled).collect();
        assert!(
            missing.is_empty(),
            "Unhandled SelectionShape variants: {:?}",
            missing
        );
    }

    #[test]
    fn handled_selection_shapes_has_no_duplicates() {
        let mut seen = HashSet::new();
        for kind in HANDLED_SELECTION_SHAPES {
            assert!(
                seen.insert(kind),
                "Duplicate in HANDLED_SELECTION_SHAPES: {:?}",
                kind
            );
        }
    }
}
