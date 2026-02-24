use crate::bridge::godot::names::theme;
use crate::bridge::vim_adapter::core::cast::{i32_to_usize, usize_to_i32};
use godot::classes::CodeEdit;
use godot::prelude::*;
use vim_core::runtime::pure::{self as pure_motion};
use vim_core::inputs::commands::motions::Motion;

/// Handles scroll/viewport motions via pure core computation + thin shell adapter.
///
/// Reads viewport info from Godot, delegates to canonical pure motion helpers,
/// and applies the computed result.
pub fn execute_scroll_motion(editor: &mut Gd<CodeEdit>, motion: Motion, count: usize) {
    let vp = read_viewport_info(editor);
    let result = pure_motion::compute_scroll(motion, count, &vp);

    if let Some(target) = result.target_line {
        editor
            .set_caret_line_ex(usize_to_i32(target))
            .can_be_hidden(false)
            .done();
    }
    if let Some(scroll) = result.target_v_scroll {
        editor.set_v_scroll(scroll);
    }
    if let Some(scroll) = result.target_h_scroll {
        editor.set_h_scroll(scroll);
    }
}

/// Reads viewport parameters from the editor for pure scroll computation.
fn read_viewport_info(editor: &Gd<CodeEdit>) -> pure_motion::ViewportInfo {
    let char_width = editor
        .get_theme_font(theme::FONT)
        .map(|font| font.get_char_size(' ' as u32, 0).x as f64);

    pure_motion::ViewportInfo {
        caret_line: i32_to_usize(editor.get_caret_line()),
        caret_col: i32_to_usize(editor.get_caret_column()),
        visible_lines: i32_to_usize(editor.get_visible_line_count()),
        line_count: i32_to_usize(editor.get_line_count()),
        line_height: editor.get_line_height() as f64,
        v_scroll: editor.get_v_scroll(),
        viewport_height: editor.get_size().y as f64,
        h_scroll: editor.get_h_scroll(),
        char_width,
        viewport_width: editor.get_size().x as f64,
    }
}
