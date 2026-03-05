//! Cursor geometry mapping for the custom overlay cursor.
//!
//! This module centralizes cursor placement math and avoids tab-specific
//! heuristics by calibrating to Godot's native caret draw position.

use crate::bridge::godot::names::theme;
use crate::bridge::vim_adapter::core::column_codec::EditorCol;
use godot::classes::CodeEdit;
use godot::prelude::*;

const ALIGNMENT_EPSILON: f32 = 1.25;
const MIN_CURSOR_WIDTH: f32 = 0.5;

/// Overlay cursor geometry in editor-local coordinates.
#[derive(Debug, Clone, Copy)]
pub(crate) struct CursorGeometry {
    pub(crate) pos: Vector2,
    pub(crate) height: f32,
    pub(crate) width: f32,
}

/// How to interpret X returned from `get_rect_at_line_column`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RectAlignmentMode {
    /// Rect X already maps to caret X.
    DirectRectX,
    /// Caret X maps to the right edge of the reported rect.
    RectPlusWidth,
}

/// Computes target position and size for the custom cursor overlay.
///
/// `override_pos` is used for visual mode where the logical Vim cursor may
/// differ from CodeEdit's live caret due to selection rendering behavior.
pub(crate) fn compute_cursor_geometry(
    editor: &Gd<CodeEdit>,
    override_pos: Option<(usize, EditorCol)>,
) -> Option<CursorGeometry> {
    let line_height = editor.get_line_height() as f32;
    let font = editor.get_theme_font(theme::FONT)?;
    let font_size = editor.get_theme_font_size(theme::FONT_SIZE);
    let fallback_char_width = font.get_char_size('m' as u32, font_size).x.max(1.0);

    let current_line = editor.get_caret_line();
    let current_col = editor.get_caret_column();
    let native_caret_pos = editor.get_caret_draw_pos();

    let (line, col, using_override) = if let Some((l, c)) = override_pos {
        (l as i32, c.as_usize() as i32, true)
    } else {
        (current_line, current_col, false)
    };

    let target_rect = editor.get_rect_at_line_column(line, col);
    if is_invalid_rect(target_rect) {
        // Godot reports (-1, -1) when the line drawing cache is stale.
        return None;
    }

    let alignment = resolve_editor_alignment(editor, current_line, current_col, native_caret_pos.x);
    let mapped_target_x = map_rect_x(
        alignment,
        target_rect.position.x as f32,
        target_rect.size.x as f32,
    );

    // Use native caret draw X only when target and editor caret coincide.
    // Y comes from rect geometry because `get_caret_draw_pos().y` is caret draw-baseline,
    // while the overlay cursor expects top-left line coordinates.
    let use_native_pos = !using_override && line == current_line && col == current_col;
    let target_x = if use_native_pos {
        native_caret_pos.x
    } else {
        mapped_target_x
    };
    let target_pos = Vector2::new(target_x, target_rect.position.y as f32);

    let mut target_height = target_rect.size.y as f32;
    if target_height < 0.1 {
        target_height = line_height;
    }

    // Keep previous layout readiness guard to avoid transient jumps.
    if target_pos.y.abs() < f32::EPSILON && line > 0 {
        return None;
    }

    let line_len = editor.get_line(line).len();
    let target_width = derive_cursor_width(
        editor,
        line,
        col,
        line_len,
        alignment,
        target_x,
        fallback_char_width,
    );

    Some(CursorGeometry {
        pos: target_pos,
        height: target_height,
        width: target_width,
    })
}

fn resolve_editor_alignment(
    editor: &Gd<CodeEdit>,
    current_line: i32,
    current_col: i32,
    native_x: f32,
) -> RectAlignmentMode {
    let current_rect = editor.get_rect_at_line_column(current_line, current_col);
    if is_invalid_rect(current_rect) {
        return RectAlignmentMode::DirectRectX;
    }

    resolve_alignment_mode(
        native_x,
        current_rect.position.x as f32,
        current_rect.size.x as f32,
        ALIGNMENT_EPSILON,
    )
}

fn derive_cursor_width(
    editor: &Gd<CodeEdit>,
    line: i32,
    col: i32,
    line_len: usize,
    alignment: RectAlignmentMode,
    mapped_current_x: f32,
    fallback_char_width: f32,
) -> f32 {
    let Ok(col_usize) = usize::try_from(col) else {
        return fallback_char_width;
    };

    if col_usize >= line_len {
        return fallback_char_width;
    }

    let next_rect = editor.get_rect_at_line_column(line, col + 1);
    if is_invalid_rect(next_rect) {
        return fallback_char_width;
    }

    let mapped_next_x = map_rect_x(
        alignment,
        next_rect.position.x as f32,
        next_rect.size.x as f32,
    );

    width_from_delta(mapped_current_x, Some(mapped_next_x), fallback_char_width)
}

fn is_invalid_rect(rect: Rect2i) -> bool {
    rect.position.x == -1 && rect.position.y == -1
}

fn map_rect_x(mode: RectAlignmentMode, rect_x: f32, rect_width: f32) -> f32 {
    match mode {
        RectAlignmentMode::DirectRectX => rect_x,
        RectAlignmentMode::RectPlusWidth => rect_x + rect_width,
    }
}

fn width_from_delta(current_x: f32, next_x: Option<f32>, fallback_width: f32) -> f32 {
    let fallback = fallback_width.max(1.0);
    let Some(next_x) = next_x else {
        return fallback;
    };

    if !next_x.is_finite() || !current_x.is_finite() {
        return fallback;
    }

    let width = (next_x - current_x).abs();
    if width < MIN_CURSOR_WIDTH {
        fallback
    } else {
        width
    }
}

fn resolve_alignment_mode(
    native_x: f32,
    rect_x: f32,
    rect_width: f32,
    epsilon: f32,
) -> RectAlignmentMode {
    let direct_delta = (native_x - rect_x).abs();
    if direct_delta <= epsilon {
        return RectAlignmentMode::DirectRectX;
    }

    let plus_width_delta = (native_x - (rect_x + rect_width)).abs();
    if plus_width_delta <= epsilon {
        return RectAlignmentMode::RectPlusWidth;
    }

    RectAlignmentMode::DirectRectX
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map_samples(mode: RectAlignmentMode, samples: &[(f32, f32)]) -> Vec<f32> {
        samples
            .iter()
            .map(|(x, w)| map_rect_x(mode, *x, *w))
            .collect()
    }

    #[test]
    fn alignment_mode_prefers_direct_when_close_to_rect_x() {
        let mode = resolve_alignment_mode(40.2, 40.0, 8.0, 1.0);
        assert_eq!(mode, RectAlignmentMode::DirectRectX);
    }

    #[test]
    fn alignment_mode_uses_plus_width_when_close_to_right_edge() {
        let mode = resolve_alignment_mode(48.1, 40.0, 8.0, 1.0);
        assert_eq!(mode, RectAlignmentMode::RectPlusWidth);
    }

    #[test]
    fn alignment_mode_defaults_to_direct_when_no_match() {
        let mode = resolve_alignment_mode(100.0, 40.0, 8.0, 1.0);
        assert_eq!(mode, RectAlignmentMode::DirectRectX);
    }

    #[test]
    fn mapped_x_progression_handles_tab_then_spaces() {
        // Synthetic sequence where rect-based columns need right-edge mapping:
        // line: "\t\tpass"
        // expected caret X progression: [0, 16, 32, 40, 48].
        let samples = [
            (0.0, 0.0),
            (0.0, 16.0),
            (16.0, 16.0),
            (32.0, 8.0),
            (40.0, 8.0),
        ];
        let mapped = map_samples(RectAlignmentMode::RectPlusWidth, &samples);
        assert_eq!(mapped, vec![0.0, 16.0, 32.0, 40.0, 48.0]);
    }

    #[test]
    fn width_from_delta_supports_tabs_and_spaces() {
        assert_eq!(width_from_delta(0.0, Some(16.0), 8.0), 16.0);
        assert_eq!(width_from_delta(32.0, Some(40.0), 8.0), 8.0);
    }

    #[test]
    fn width_from_delta_falls_back_for_invalid_or_eol() {
        assert_eq!(width_from_delta(10.0, None, 8.0), 8.0);
        assert_eq!(width_from_delta(10.0, Some(10.2), 8.0), 8.0);
        assert_eq!(width_from_delta(10.0, Some(f32::NAN), 8.0), 8.0);
    }
}
