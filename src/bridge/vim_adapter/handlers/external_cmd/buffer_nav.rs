//! Buffer navigation: BufferNext, BufferPrev, BufferGoto.

use super::FocusBehavior;
use crate::bridge::godot::code_edit_ext::CodeEditExt;
use crate::bridge::vim_wrapper::VimController;

impl VimController {
    pub(super) fn handle_buffer_nav(&mut self, delta: i32) -> FocusBehavior {
        self.switch_buffer(delta);
        FocusBehavior::Skip
    }

    pub(super) fn handle_buffer_goto(&mut self, index: usize) -> FocusBehavior {
        self.goto_buffer(index);
        FocusBehavior::Skip
    }

    pub(super) fn handle_goto_line(
        &mut self,
        editor: &mut godot::prelude::Gd<godot::classes::CodeEdit>,
        line: usize,
    ) {
        // Use unfold to reveal the target line (e.g., :123 command)
        editor.set_line_unfold(crate::bridge::vim_adapter::core::cast::usize_to_i32(line));
    }
}
