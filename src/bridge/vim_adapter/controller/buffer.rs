//! Buffer switching methods for VimController.
//!
//! Handles :bn, :bp, :b{n} commands via TabContainer.

use crate::bridge::godot::names::control;
use crate::bridge::vim_wrapper::VimController;
use godot::classes::{CodeEdit, Node, TabContainer};
use godot::prelude::*;

impl VimController {
    /// Recursively find the TabContainer within the ScriptEditor.
    fn find_tab_container(node: Gd<Node>) -> Option<Gd<TabContainer>> {
        if let Ok(tabs) = node.clone().try_cast::<TabContainer>() {
            return Some(tabs);
        }

        for child in node.get_children().iter_shared() {
            if let Some(tabs) = Self::find_tab_container(child) {
                return Some(tabs);
            }
        }
        None
    }

    /// Recursively find the CodeEdit within a node (e.g. active tab).
    fn find_code_edit(node: Gd<Node>) -> Option<Gd<CodeEdit>> {
        if let Ok(edit) = node.clone().try_cast::<CodeEdit>() {
            return Some(edit);
        }

        for child in node.get_children().iter_shared() {
            if let Some(edit) = Self::find_code_edit(child) {
                return Some(edit);
            }
        }
        None
    }

    /// Switch to next/previous buffer.
    ///
    /// `delta`: +1 for next, -1 for previous
    pub(crate) fn switch_buffer(&mut self, delta: i32) {
        let Some(script_editor) = godot::classes::EditorInterface::singleton().get_script_editor()
        else {
            self.show_cmdline_message("No script editor");
            return;
        };

        let Some(mut tabs) = Self::find_tab_container(script_editor.upcast()) else {
            self.show_cmdline_message("No tabs found");
            return;
        };

        let count = tabs.get_tab_count();
        if count <= 1 {
            self.show_cmdline_message("Only one buffer");
            return;
        }

        let current = tabs.get_current_tab();
        let next = ((current + delta) % count + count) % count;

        tabs.set_current_tab(next);

        if let Some(control) = tabs.get_current_tab_control() {
            if let Some(edit) = Self::find_code_edit(control.clone().upcast()) {
                // Attach immediately; gui_focus_changed may not fire for floating
                // window viewports, so the explicit attach is required here.
                self.attach(edit.clone());
                edit.clone()
                    .call_deferred(control::methods::GRAB_FOCUS, &[]);
                log::debug!("switch_buffer: Attached and deferring grab_focus for new editor");
            } else {
                let mut c = control;
                c.call_deferred(control::methods::GRAB_FOCUS, &[]);
            }
        }
    }

    /// Go to specific buffer by 1-indexed number.
    pub(crate) fn goto_buffer(&mut self, index: usize) {
        let Some(script_editor) = godot::classes::EditorInterface::singleton().get_script_editor()
        else {
            self.show_cmdline_message("No script editor");
            return;
        };

        let Some(mut tabs) = Self::find_tab_container(script_editor.upcast()) else {
            self.show_cmdline_message("No tabs found");
            return;
        };

        let count = tabs.get_tab_count();
        if count == 0 {
            self.show_cmdline_message("No buffers open");
            return;
        }

        // Convert 1-indexed to 0-indexed
        if index == 0 || index > count as usize {
            self.show_cmdline_message(&format!("Buffer {} doesn't exist (1-{})", index, count));
            return;
        }

        let target_idx = (index - 1) as i32;

        tabs.set_current_tab(target_idx);

        if let Some(control) = tabs.get_current_tab_control() {
            if let Some(edit) = Self::find_code_edit(control.clone().upcast()) {
                // Attach immediately; gui_focus_changed may not fire for floating
                // window viewports, so the explicit attach is required here.
                self.attach(edit.clone());
                edit.clone()
                    .call_deferred(control::methods::GRAB_FOCUS, &[]);
                log::debug!("goto_buffer: Attached and deferring grab_focus for new editor");
            } else {
                let mut c = control;
                c.call_deferred(control::methods::GRAB_FOCUS, &[]);
            }
        }
    }
}
