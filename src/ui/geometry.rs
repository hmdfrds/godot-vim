//! Logical (line, col) to pixel-coordinate conversion for overlay positioning.
//!
//! Centralizes the `get_rect_at_line_column` workarounds so every overlay
//! (inccommand, yank highlight, debug range) uses a consistent mapping.
//! The cursor overlay does NOT use these helpers -- it uses TextServer
//! shaped-text APIs for sub-character precision (see `cursor_shape.rs`).

use godot::prelude::{Rect2, Vector2};

use crate::types::{CharLineCol, PixelPos};

/// Left-edge x of column `col` on `line`, in editor-local pixels.
///
/// Works around a Godot bug: `get_rect_at_line_column(line, col)` returns
/// the rect for `col-1` when `col >= 1`, because `shaped_text_get_grapheme_bounds`
/// uses `>=` instead of `>` in its end-boundary check. The fix: for col >= 1,
/// the right edge (`position.x + size.x`) of the returned rect equals the
/// true left edge of the requested column.
pub(super) fn corrected_col_x(editor: &godot::classes::CodeEdit, line: i32, col: i32) -> Option<PixelPos> {
    let rect = editor.get_rect_at_line_column(line, col);
    // (-1,-1) = off-screen/not-laid-out sentinel.
    if rect.position.x == -1 && rect.position.y == -1 {
        return None;
    }
    // All-zeros = empty document sentinel.
    if rect.position.x == 0 && rect.position.y == 0 && rect.size.x == 0 && rect.size.y == 0 {
        return None;
    }
    let x = if col == 0 {
        rect.position.x
    } else {
        rect.position.x + rect.size.x
    };
    Some(PixelPos::new(x, rect.position.y))
}

/// One `Rect2` per visible line in the `[start, end)` range, suitable for
/// painting colored highlight overlays.
///
/// Multi-line ranges produce: first line from start col to right edge,
/// intermediate lines at full width, last line from col 0 to end col.
/// `max_rects` caps output to prevent runaway draw cost on huge ranges.
/// Off-screen lines (sentinel rects from Godot) are silently skipped.
pub(super) fn compute_highlight_rects(
    editor: &godot::classes::CodeEdit,
    start: &CharLineCol,
    end: &CharLineCol,
    max_rects: usize,
) -> Vec<Rect2> {
    let mut rects = Vec::new();
    let line_height = editor.get_line_height().max(1);

    if start.line == end.line {
        let Some(start_pos) = corrected_col_x(editor, start.line, start.col) else {
            return rects;
        };
        let Some(end_pos) = corrected_col_x(editor, end.line, end.col) else {
            return rects;
        };
        let width = (end_pos.x - start_pos.x).max(1);
        rects.push(Rect2::new(
            Vector2::new(start_pos.x as f32, start_pos.y as f32),
            Vector2::new(width as f32, line_height as f32),
        ));
    } else {
        let editor_width = editor.get_size().x;
        if !editor_width.is_finite() || editor_width < 0.0 {
            return rects;
        }
        for line in start.line..=end.line {
            if rects.len() >= max_rects {
                break;
            }
            let col = if line == start.line { start.col } else { 0 };
            let Some(pos) = corrected_col_x(editor, line, col) else {
                continue;
            };
            let width = if line == end.line {
                let Some(end_pos) = corrected_col_x(editor, end.line, end.col) else {
                    continue;
                };
                (end_pos.x - pos.x).max(1)
            } else {
                (editor_width.min(i32::MAX as f32) as i32 - pos.x).max(1)
            };
            rects.push(Rect2::new(
                Vector2::new(pos.x as f32, pos.y as f32),
                Vector2::new(width as f32, line_height as f32),
            ));
        }
    }

    rects
}
