//! Global input handler for Dock Navigation and Global Mappings.
//!
//! Extracted from `entry.rs` to keep the plugin lifecycle code focused.
//! Contains:
//! - Global mapping resolution (e.g., `<Space>f` → `:FileSystem`)
//! - Focus-based input filtering
//! - Dock input delegation

use crate::bridge::entry::GodotVimPlugin;
use crate::bridge::godot::names::callbacks;
use crate::bridge::types::key::KeyCode;
use crate::bridge::vim_adapter::mapping::{MappedAction, MappingLookup, MappingMode};
use godot::classes::{EditorInterface, InputEvent, InputEventKey};
use godot::prelude::*;

/// Trait for global input handling (mappings, dock navigation).
///
/// Extracted from `GodotVimPlugin::input()` to separate concerns.
pub trait GlobalInputHandler {
    /// Processes a global input event (called from `IEditorPlugin::input`).
    fn handle_global_input(&mut self, event: Gd<InputEvent>);

    /// Determines if global processing should be skipped for the focused control.
    fn should_skip_global_processing(
        &self,
        viewport_opt: &Option<Gd<godot::classes::Viewport>>,
    ) -> bool;
}

impl GlobalInputHandler for GodotVimPlugin {
    fn handle_global_input(&mut self, event: Gd<InputEvent>) {
        if !crate::bridge::settings::VimSettings::enabled() {
            return;
        }

        if !crate::bridge::settings::VimSettings::mapping_enabled() {
            return;
        }

        let Some(key_event) = event.clone().try_cast::<InputEventKey>().ok() else {
            return;
        };

        if !key_event.is_pressed() {
            return;
        }

        let interface = EditorInterface::singleton();
        let viewport_opt = interface.get_base_control().and_then(|c| c.get_viewport());

        if let Some(key_event) = crate::bridge::safety::input::parse_godot_event(&event) {
            if !matches!(
                key_event.code,
                KeyCode::Shift | KeyCode::Control | KeyCode::Alt | KeyCode::Meta
            ) {
                if self.should_skip_global_processing(&viewport_opt) {
                    return;
                }

                // Add key to pending sequence (convert to vim-core key at boundary)
                let vim_key = crate::bridge::vim_adapter::engine::VimEngine::to_vim_key(&key_event);
                self.global_mapping_state.add_key(vim_key);

                // Check mappings in both Global mode and the current Vim mode.
                if let Some(ref controller) = self.vim_controller {
                    let ctrl = controller.bind();
                    let pending = self.global_mapping_state.pending_keys();

                    let global_lookup = ctrl
                        .input
                        .mapping_store
                        .lookup(pending, MappingMode::Global);
                    let current_mode = ctrl.get_mapping_mode().unwrap_or(MappingMode::Normal);
                    let mode_lookup = ctrl.input.mapping_store.lookup(pending, current_mode);

                    // Mode-specific match takes priority over global match.
                    let (winner, is_global) =
                        match (&mode_lookup, &global_lookup) {
                            // Mode has exact match → mode wins (more specific)
                            (MappingLookup::Match(m), _)
                            | (MappingLookup::MatchAndPrefix(m), _) => (Some(m.to.clone()), false),
                            // Only Global has exact match → global wins
                            (_, MappingLookup::Match(g))
                            | (_, MappingLookup::MatchAndPrefix(g)) => (Some(g.to.clone()), true),
                            // Neither has exact match
                            _ => (None, false),
                        };

                    let either_prefix = matches!(
                        global_lookup,
                        MappingLookup::Prefix | MappingLookup::MatchAndPrefix(_)
                    ) || matches!(
                        mode_lookup,
                        MappingLookup::Prefix | MappingLookup::MatchAndPrefix(_)
                    );

                    drop(ctrl); // Release borrow before mutation

                    if let Some(action) = winner {
                        // Found an exact match - execute it
                        self.global_mapping_state.reset();

                        // Stop timeout timer
                        if let Some(timer) = &mut self.global_mapping_timer {
                            timer.stop();
                        }

                        // Execute the mapping
                        self.execute_mapped_action(action, is_global, &viewport_opt);
                        return;
                    } else if either_prefix {
                        // At least one mode has this as a prefix - wait for more keys
                        let timeoutlen = crate::bridge::settings::VimSettings::timeoutlen();
                        if let Some(timer) = &mut self.global_mapping_timer {
                            #[expect(
                                clippy::cast_precision_loss,
                                reason = "milliseconds don't need 64-bit precision"
                            )]
                            timer.set_wait_time(timeoutlen as f64 / 1000.0);
                            timer.start();
                        }
                        // Consume key silently while waiting
                        if let Some(mut viewport) = viewport_opt.clone() {
                            viewport.set_input_as_handled();
                        }
                        return;
                    } else {
                        // Neither matches - reset and continue to normal processing
                        if let Some(timer) = &mut self.global_mapping_timer {
                            timer.stop();
                        }
                        self.global_mapping_state.reset();
                    }
                }
            }
        }

        let Some(focus_owner) = viewport_opt.clone().and_then(|v| v.gui_get_focus_owner()) else {
            return;
        };

        // Allow ESC through for canceling search focus; other editor keys are handled
        // by the editor's own input handler.
        let is_editor = focus_owner.is_class("LineEdit")
            || focus_owner.is_class("TextEdit")
            || focus_owner.is_class("CodeEdit");
        if is_editor && key_event.get_keycode() != godot::global::Key::ESCAPE {
            return;
        }

        let result = crate::bridge::navigation::dock::focus::handle_dock_input(
            focus_owner.clone(),
            event.clone(),
        );

        match result {
            crate::bridge::navigation::dock::focus::DockInputResult::Handled => {
                if let Some(mut vp) = viewport_opt.clone() {
                    vp.set_input_as_handled();
                }
            }
            crate::bridge::navigation::dock::focus::DockInputResult::Focused(target) => {
                if let Some(mut vp) = viewport_opt.clone() {
                    vp.set_input_as_handled();
                }
                if let Some(controller) = &mut self.vim_controller {
                    controller.bind_mut().observe_dock_control(target);
                }
            }
            crate::bridge::navigation::dock::focus::DockInputResult::Ignored => {}
        }
    }

    /// Returns `true` when global mappings must not be attempted.
    ///
    /// Skips when:
    /// - Focus is on a foreign text input (LineEdit/TextEdit not belonging to this editor)
    /// - Focus is on the attached editor but Vim is in Insert/Replace mode
    fn should_skip_global_processing(
        &self,
        viewport_opt: &Option<Gd<godot::classes::Viewport>>,
    ) -> bool {
        let Some(ref vp) = viewport_opt else {
            return false;
        };
        let Some(focus_owner) = vp.gui_get_focus_owner() else {
            return false;
        };

        let is_text_input = focus_owner.is_class("LineEdit") || focus_owner.is_class("TextEdit");

        if is_text_input {
            let is_our_editor = self.vim_controller.as_ref().is_some_and(|controller| {
                let ctrl = controller.bind();
                ctrl.attached_editor
                    .as_ref()
                    .is_some_and(|ed| ed.instance_id() == focus_owner.instance_id())
            });

            if !is_our_editor {
                return true;
            }

            if let Some(ref controller) = self.vim_controller {
                let ctrl = controller.bind();
                if matches!(ctrl.get_mapping_mode(), Some(MappingMode::Insert)) {
                    return true;
                }
            }
        }

        false
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Helper: Execute a resolved mapping action
// ═══════════════════════════════════════════════════════════════════════════

impl GodotVimPlugin {
    /// Executes a resolved mapping action (global or mode-specific).
    pub(crate) fn execute_mapped_action(
        &mut self,
        action: MappedAction,
        is_global: bool,
        viewport_opt: &Option<Gd<godot::classes::Viewport>>,
    ) {
        if let MappedAction::Command(cmd_str) = action {
            if is_global {
                // Global commands are executed deferred to prevent re-entrant borrow panics.
                if let Some(ref mut controller) = self.vim_controller {
                    let cmd_str = cmd_str.trim_start_matches(':');
                    let mut parts = cmd_str.split_whitespace();
                    if let Some(cmd_name) = parts.next() {
                        let mut args_packed = PackedStringArray::new();
                        for arg in parts {
                            args_packed.push(arg);
                        }

                        let mut ctrl = controller.clone();
                        ctrl.call_deferred(
                            callbacks::EXECUTE_COMMAND_DEFERRED,
                            &[
                                GString::from(cmd_name).to_variant(),
                                args_packed.to_variant(),
                            ],
                        );
                    }
                }
            } else {
                if let Some(ref mut controller) = self.vim_controller {
                    controller
                        .bind_mut()
                        .process_mapped_key(MappedAction::Command(cmd_str));
                }
            }
        } else if !is_global {
            if let Some(ref mut controller) = self.vim_controller {
                controller.bind_mut().process_mapped_key(action);
            }
        }

        if let Some(mut viewport) = viewport_opt.clone() {
            viewport.set_input_as_handled();
        }
    }
}
