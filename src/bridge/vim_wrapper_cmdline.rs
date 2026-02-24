//! Cmdline callback implementations for VimController.
//!
//! Extracted from `vim_wrapper.rs`. The `#[func]` stubs remain there.

use crate::bridge::vim_adapter::handlers::cmdline::{CmdLineHandler, IncsearchHandler};
use crate::bridge::vim_wrapper::VimController;

use godot::classes::{InputEvent, InputEventKey};
use godot::global::Key;
use godot::prelude::*;

impl VimController {
    pub(crate) fn on_cmd_submitted_impl(&mut self, text: GString) {
        self.handle_cmd_submitted(&text.to_string());
    }

    pub(crate) fn on_cmd_text_changed_impl(&mut self, new_text: GString) {
        self.update_incsearch_highlights(&new_text.to_string());
    }

    pub(crate) fn on_cmd_input_gui_input_impl(&mut self, event: Gd<InputEvent>) {
        let Some(key_event) = event.try_cast::<InputEventKey>().ok() else {
            return;
        };

        if !key_event.is_pressed() {
            return;
        }

        match key_event.get_keycode() {
            Key::ESCAPE => self.handle_cmdline_escape(),
            Key::UP => self.handle_cmdline_history_up(),
            Key::DOWN => self.handle_cmdline_history_down(),
            _ => {}
        }
    }
}
