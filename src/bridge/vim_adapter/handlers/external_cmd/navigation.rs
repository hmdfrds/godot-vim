//! Navigation operations: GotoDefinition, ShowDocumentation.

use crate::bridge::vim_adapter::core::cast::i32_to_usize;
use crate::bridge::vim_wrapper::VimController;
use crate::bridge::vim_wrapper_util::extract_word_at_col;
use godot::classes::CodeEdit;
use godot::prelude::*;
use vim_core::domain::position::Position;

impl VimController {
    pub(super) fn handle_goto_definition(&mut self, editor: &mut Gd<CodeEdit>) {
        let line = editor.get_caret_line();
        let col = editor.get_caret_column();

        // Save current position to jumplist using centralized tracker
        let current = Position::new(i32_to_usize(line), i32_to_usize(col));

        self.engine.record_jump_at(current);

        let line_text = editor.get_line(line).to_string();
        if let Some(word) = extract_word_at_col(&line_text, i32_to_usize(col)) {
            editor.emit_signal(
                "symbol_lookup",
                &[word.to_variant(), line.to_variant(), col.to_variant()],
            );
            log::debug!("Goto definition word={} line={} col={}", word, line, col);
        } else {
            log::debug!("Goto definition: no symbol under cursor");
        }
    }
}

pub fn handle_show_documentation(editor: &mut Gd<CodeEdit>) {
    let line = editor.get_caret_line();
    let col = editor.get_caret_column();
    let line_text = editor.get_line(line).to_string();

    if let Some(word) = extract_word_at_col(&line_text, i32_to_usize(col)) {
        // Calculate the local position of the character
        let rect_local = editor.get_rect_at_line_column(line, col);
        let pos_local = godot::prelude::Vector2::new(
            rect_local.position.x as f32,
            rect_local.position.y as f32,
        );

        // Convert to window-relative coordinates
        let transform = editor.get_global_transform();
        let pos_global = transform * pos_local;

        // Warp mouse to the symbol's location
        let mut display_server = godot::classes::DisplayServer::singleton();
        display_server.warp_mouse(godot::prelude::Vector2i::new(
            pos_global.x as i32,
            pos_global.y as i32 + (rect_local.size.y / 2), // Center vertically on the line
        ));

        // Emit signal to trigger the tooltip
        editor.emit_signal(
            "symbol_hovered",
            &[word.to_variant(), line.to_variant(), col.to_variant()],
        );
        log::debug!(
            "Show documentation: warped mouse to {:?} and emitted symbol_hovered for '{}'",
            pos_global,
            word
        );
    } else {
        log::debug!("Show documentation: no symbol under cursor");
    }
}
