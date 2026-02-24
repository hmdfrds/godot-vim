//! Dock observation and deferred command implementations for VimController.
//!
//! Extracted from `vim_wrapper.rs`. The `#[func]` stubs remain there.

use crate::bridge::godot::names::{callbacks, control};
use crate::bridge::navigation::dock::focus::DockInputResult;
use crate::bridge::vim_wrapper::VimController;

use godot::classes::{Control, InputEvent};
use godot::prelude::*;

impl VimController {
    pub(crate) fn observe_dock_control_impl(&mut self, control: Gd<Control>) {
        if !control.is_instance_valid() {
            return;
        }

        // If already observing this instance with the signal connected, skip.
        if let Some(prev) = &self.dock.observed_dock {
            if prev.is_instance_valid() && prev.instance_id() == control.instance_id() {
                let callable = self.base().callable(callbacks::ON_DOCK_GUI_INPUT);
                if prev.is_connected(control::signals::GUI_INPUT, &callable) {
                    return;
                }
            }
        }

        // Disconnect previous
        if let Some(mut prev) = self.dock.observed_dock.take() {
            if prev.is_instance_valid() {
                let callable = self.base().callable(callbacks::ON_DOCK_GUI_INPUT);
                if prev.is_connected(control::signals::GUI_INPUT, &callable) {
                    prev.disconnect(control::signals::GUI_INPUT, &callable);
                }
            }
        }

        // Connect new
        let mut control_mut = control.clone();
        let callable = self.base().callable(callbacks::ON_DOCK_GUI_INPUT);
        if !control_mut.is_connected(control::signals::GUI_INPUT, &callable) {
            control_mut.connect(control::signals::GUI_INPUT, &callable);
        }

        self.dock.observed_dock = Some(control);
    }

    pub(crate) fn on_dock_gui_input_impl(&mut self, event: Gd<InputEvent>) {
        crate::bridge::safety::guard(
            || {
                let Some(control) = self.dock.observed_dock.clone() else {
                    return;
                };
                if !control.is_instance_valid() {
                    return;
                }

                match crate::bridge::navigation::dock::focus::handle_dock_input(
                    control.clone(),
                    event,
                ) {
                    DockInputResult::Handled => {
                        self.set_input_handled();
                    }
                    DockInputResult::Focused(new_target) => {
                        self.observe_dock_control_impl(new_target);
                        self.set_input_handled();
                    }
                    DockInputResult::Ignored => {}
                }
            },
            (),
        );
    }

    pub(crate) fn execute_command_deferred_body(&mut self, cmd: String, args: PackedStringArray) {
        self.execute_command_deferred_impl(cmd, args);
    }
}
