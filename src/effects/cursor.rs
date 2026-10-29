//! Applies cursor and selection effects to CodeEdit (set cursor, set/clear
//! selection in char/line/block mode).

use vim_core::primitives::SelectionShape;

use super::viewport;
use crate::bridge::codec::{i32_to_usize, usize_to_i32, DocumentView};
use crate::bridge::port::{TextEditorPort, ViewportAdjust};
use crate::types::CharLineCol;

/// Move the caret to the given byte `offset` and scroll the viewport to follow,
/// keeping Vim's `scrolloff` margin.
///
/// A target inside a closed fold puts the caret on the first visible line
/// after it (`set_caret_line_unfold`).
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
    enforce_scrolloff(editor, scrolloff);
}

/// Scroll the viewport so the caret keeps at least `scrolloff` screen rows
/// of context above and below, as Vim's `scrolloff` does, by the fewest rows.
pub(crate) fn enforce_scrolloff(editor: &mut impl TextEditorPort, scrolloff: i32) {
    if scrolloff <= 0 {
        return;
    }
    let rows = editor.get_visible_line_count();
    let caret = viewport::caret_row(editor);
    let from_top = caret - viewport::top_row(editor);
    // A view too small for both margins keeps the caret in the middle.
    let wanted = if rows <= scrolloff * 2 {
        (rows / 2).min(caret)
    } else if from_top < scrolloff {
        scrolloff.min(caret)
    } else if from_top > rows - 1 - scrolloff {
        rows - 1 - scrolloff
    } else {
        return;
    };
    if wanted != from_top {
        viewport::put_caret_row(editor, wanted);
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
/// After setting the selection, moves the caret off a folded head line and
/// scrolls the viewport to follow.
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

    // A head line inside a closed fold gives the line after the fold.
    editor.set_caret_line_unfold(head_line, ViewportAdjust::NoAdjust);

    // Viewport scrolling strategy depends on selection shape because Godot's
    // caret after select() may not match the Vim head position.
    match shape {
        SelectionShape::Char => {
            // In char mode, Godot's caret tracks the head — viewport follows.
            editor.adjust_viewport_to_caret();
        }
        _ => {
            // In line/block mode the caret is on the head line but not at the
            // head column. A head line off screen comes back centred.
            let rows = editor.get_visible_line_count();
            let from_top = viewport::caret_row(editor) - viewport::top_row(editor);
            if !(0..rows).contains(&from_top) {
                viewport::put_caret_row(editor, rows / 2);
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
