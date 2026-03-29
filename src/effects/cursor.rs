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
    log::trace!("set_cursor: offset={} -> line={} col={}", offset, pos.line, pos.col);
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
fn enforce_scrolloff(editor: &mut impl TextEditorPort, cursor_line: i32, scrolloff: i32) {
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
/// For `Block` mode, the +1 is applied to `max_col` as before (rendering
/// the exclusive-end rectangle per line).
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

    match shape {
        SelectionShape::Char => {
            if head >= anchor {
                let head_line_len = usize_to_i32(doc.line_index.line_char_count(doc.text, i32_to_usize(head_line)));
                editor.select(
                    CharLineCol::new(anchor_line, anchor_col),
                    CharLineCol::new(head_line, (head_col + 1).min(head_line_len)),
                );
            } else {
                let anchor_line_len = usize_to_i32(doc.line_index.line_char_count(doc.text, i32_to_usize(anchor_line)));
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
            let bot_end_col = usize_to_i32(doc.line_index.line_char_count(doc.text, i32_to_usize(bot_line)));
            if head_line >= anchor_line {
                editor.select(CharLineCol::new(top_line, 0), CharLineCol::new(bot_line, bot_end_col));
            } else {
                editor.select(CharLineCol::new(bot_line, bot_end_col), CharLineCol::new(top_line, 0));
            }
        }
        SelectionShape::Block => {
            render_block_selection(editor, doc, anchor_line, anchor_col, head_line, head_col);
        }
        _ => {
            log::warn!("Unknown SelectionShape variant {:?} — rendering as Block (best-effort)", shape);
            render_block_selection(editor, doc, anchor_line, anchor_col, head_line, head_col);
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

/// Render a block (rectangular) visual selection using Godot's multi-caret system.
///
/// Places the primary caret on `head_line` with a selection spanning the block
/// columns, then adds secondary carets on all other lines in the range.
fn render_block_selection(
    editor: &mut impl TextEditorPort,
    doc: &DocumentView,
    anchor_line: i32,
    anchor_col: i32,
    head_line: i32,
    head_col: i32,
) {
    let min_line = anchor_line.min(head_line);
    let max_line = anchor_line.max(head_line);
    let min_col = anchor_col.min(head_col);
    let max_col = anchor_col.max(head_col);

    editor.remove_secondary_carets();

    // Godot's select() places the caret at the `to` end, so swap
    // anchor/head to keep the caret at the Vim-head side of the block.
    // The +1 on max_col converts Vim's inclusive end to Godot's exclusive end.
    let (render_anchor, render_head) = if head_col <= anchor_col {
        (max_col + 1, min_col)
    } else {
        (min_col, max_col + 1)
    };

    let head_line_len = usize_to_i32(doc.line_index.line_char_count(doc.text, i32_to_usize(head_line)));
    editor.select(
        CharLineCol::new(head_line, render_anchor.min(head_line_len)),
        CharLineCol::new(head_line, render_head.min(head_line_len)),
    );

    // One secondary caret per remaining line. Track indices for rollback on failure.
    let mut added_carets: Vec<i32> = Vec::with_capacity((max_line - min_line) as usize);
    let mut any_failed = false;

    for line in min_line..=max_line {
        if line == head_line {
            continue;
        }
        let line_len = usize_to_i32(doc.line_index.line_char_count(doc.text, i32_to_usize(line)));
        let clamped_anchor = render_anchor.min(line_len);
        let clamped_head = render_head.min(line_len);
        let caret_idx = editor.add_caret(line, clamped_head);
        if caret_idx >= 0 {
            editor.select_for_caret(CharLineCol::new(line, clamped_anchor), CharLineCol::new(line, clamped_head), caret_idx);
            added_carets.push(caret_idx);
        } else {
            log::error!(
                "set_selection: add_caret({}, {}) failed — rolling back {} secondary carets",
                line, clamped_head, added_carets.len()
            );
            any_failed = true;
            break;
        }
    }

    if any_failed {
        editor.remove_secondary_carets();
        log::error!("Block visual selection rolled back due to caret failure");
    }
}

/// Deselect all text and remove secondary carets (return to normal caret-only state).
pub(crate) fn handle_clear_selection(editor: &mut impl TextEditorPort) {
    log::trace!("clear_selection");
    editor.remove_secondary_carets();
    editor.deselect();
}
