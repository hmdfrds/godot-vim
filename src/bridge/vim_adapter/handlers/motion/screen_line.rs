use crate::bridge::vim_adapter::core::cast::{i32_to_usize, usize_to_i32};
use godot::classes::CodeEdit;
use godot::prelude::*;
use vim_core::runtime::pure::{self as pure_motion, ScreenLineResult, WrapInfo};
use vim_core::inputs::commands::motions::Motion;

/// Handles window positioning motions (H, M, L) via pure core computation.
pub fn execute_window_motion(editor: &mut Gd<CodeEdit>, motion: Motion, count: usize) {
    let first_visible = i32_to_usize(editor.get_v_scroll().floor() as i32);
    let visible_lines = i32_to_usize(editor.get_visible_line_count());
    let line_count = i32_to_usize(editor.get_line_count());

    if let Some(target) =
        pure_motion::compute_window_motion(motion, count, first_visible, visible_lines, line_count)
    {
        editor.set_caret_line(usize_to_i32(target));
    }
}

/// Handles screen-line motions (g0/g^/g$/gm/gk/gj) via pure core computation.
///
/// Reads wrap info from Godot, delegates to canonical pure motion helpers,
/// and applies the computed column/line change.
pub fn execute_screen_line_motion(editor: &mut Gd<CodeEdit>, motion: Motion) {
    let line = editor.get_caret_line();
    let col = editor.get_caret_column();
    let wrap_index = i32_to_usize(editor.get_caret_wrap_index());

    // Read wrap segments from Godot
    let wrapped_parts = editor.get_line_wrapped_text(line);
    let segment_lengths: Vec<usize> = if wrapped_parts.is_empty() {
        vec![editor.get_line(line).len()]
    } else {
        (0..wrapped_parts.len())
            .filter_map(|i| wrapped_parts.get(i).map(|s| s.to_string().len()))
            .collect()
    };

    // Compute first non-blank for the current segment
    let segment_first_nonblank = if let Some(seg) = wrapped_parts.get(wrap_index) {
        let text = seg.to_string();
        text.find(|c: char| !c.is_whitespace()).unwrap_or(0)
    } else {
        let text = editor.get_line(line).to_string();
        text.find(|c: char| !c.is_whitespace()).unwrap_or(0)
    };

    let wrap = WrapInfo {
        segment_lengths,
        wrap_index,
        current_col: i32_to_usize(col),
        current_line: i32_to_usize(line),
        line_count: i32_to_usize(editor.get_line_count()),
        segment_first_nonblank,
    };

    match pure_motion::compute_screen_line_motion(motion, &wrap) {
        ScreenLineResult::SameLineCol(target_col) => {
            editor.set_caret_column(usize_to_i32(target_col));
        }
        ScreenLineResult::DifferentLine(target_line, col_hint) => {
            editor.set_caret_line(usize_to_i32(target_line));
            if let Some(col_offset) = col_hint {
                let target_line_len = editor.get_line(usize_to_i32(target_line)).len();
                let target = col_offset.min(target_line_len.saturating_sub(1));
                editor.set_caret_column(usize_to_i32(target));
            }
        }
        ScreenLineResult::None => {}
    }
}
