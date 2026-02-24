use crate::bridge::components::cmdline::VimCmdLine;
use crate::bridge::components::cursor_visual::VimCursor;
use crate::bridge::components::status_bar;
use crate::bridge::godot::names::{
    callbacks, canvas_item, control, editor_settings, line_edit, object, range, text_edit, theme,
};
use crate::bridge::vim_wrapper::VimController;
use godot::classes::object::ConnectFlags;
use godot::classes::{CodeEdit, ProjectSettings};
use godot::prelude::*;
use vim_core::state::mode::Mode;

/// Extension trait for VimController lifecycle operations.
pub trait LifecycleTrait {
    /// Attach to a CodeEdit editor and set up all signals and components.
    fn attach_to_editor(&mut self, editor: Gd<CodeEdit>);

    /// Disconnect all signals from the currently attached editor.
    fn disconnect_signals(&mut self);

    /// Full detach - disconnect and free all resources.
    fn detach_fully(&mut self);
}

impl LifecycleTrait for VimController {
    fn attach_to_editor(&mut self, mut editor: Gd<CodeEdit>) {
        if let Some(attached) = &self.attached_editor {
            if attached.is_instance_valid() && attached.instance_id() == editor.instance_id() {
                return;
            }
        }

        for child in editor.get_children().iter_shared() {
            let node_name = child.get_name().to_string();
            if node_name.starts_with("VimCursor") {
                log::warn!("Another VimController already attached to this editor, skipping");
                return;
            }
        }

        self.disconnect_signals();

        let cmd_line = if let Some(existing) = self.ui.cmdline.take() {
            if existing.is_instance_valid() {
                existing
            } else {
                VimCmdLine::new_alloc()
            }
        } else {
            VimCmdLine::new_alloc()
        };

        status_bar::inject_cmdline(&editor, &cmd_line);
        status_bar::configure_cmdline_sizing(&cmd_line);

        if let Some(mut input) = cmd_line.bind().get_command_input() {
            let callable = self.base_mut().callable(callbacks::ON_CMD_SUBMITTED);
            if !input.is_connected(line_edit::signals::TEXT_SUBMITTED, &callable) {
                input.connect(line_edit::signals::TEXT_SUBMITTED, &callable);
            }

            let esc_callable = self.base_mut().callable(callbacks::ON_CMD_INPUT_GUI_INPUT);
            if !input.is_connected(control::signals::GUI_INPUT, &esc_callable) {
                input.connect(control::signals::GUI_INPUT, &esc_callable);
            }

            let text_changed_callable = self.base_mut().callable(callbacks::ON_CMD_TEXT_CHANGED);
            if !input.is_connected(line_edit::signals::TEXT_CHANGED, &text_changed_callable) {
                input.connect(line_edit::signals::TEXT_CHANGED, &text_changed_callable);
            }
        } else {
            log::error!("CmdLine LineEdit not found after injection");
        }

        self.ui.cmdline = Some(cmd_line);

        let callable = self.base_mut().callable(callbacks::HANDLE_GUI_INPUT);
        if !editor.is_connected(control::signals::GUI_INPUT, &callable) {
            editor.connect(control::signals::GUI_INPUT, &callable);
        }

        let callable = self.base_mut().callable(callbacks::ON_CARET_MOVED);
        if !editor.is_connected(text_edit::signals::CARET_CHANGED, &callable) {
            // Deferred connection prevents re-entrant borrow: caret changes fire synchronously
            // during text edits, which would re-enter VimController while it is modifying text.
            editor.call(
                object::methods::CONNECT,
                &[
                    text_edit::signals::CARET_CHANGED.to_variant(),
                    callable.to_variant(),
                    ConnectFlags::DEFERRED.ord().to_variant(),
                ],
            );
        }

        // Remove any orphaned VimCursor nodes
        for child in editor.get_children().iter_shared() {
            let node_name = child.get_name().to_string();
            if node_name.starts_with("VimCursor") {
                log::warn!("Cleaning up orphaned cursor node: {}", node_name);
                let mut orphan = child;
                if orphan.get_parent().is_some() {
                    editor.clone().remove_child(&orphan);
                }
                orphan.queue_free();
            }
        }

        let vim_cursor = VimCursor::new_alloc();
        let cursor_node: Gd<godot::classes::Node> = vim_cursor.clone().upcast();
        editor.add_child(&cursor_node);
        self.ui.cursor_visual = Some(vim_cursor);

        editor.add_theme_color_override(theme::CARET_COLOR, Color::from_rgba(0.0, 0.0, 0.0, 0.0));

        let update_callable = self.base_mut().callable(callbacks::ON_CURSOR_VISUAL_UPDATE);
        let scroll_callable = self.base_mut().callable(callbacks::ON_SCROLLBAR_CHANGED);
        if let Some(mut v_scroll) = editor.get_v_scroll_bar() {
            if !v_scroll.is_connected(range::signals::VALUE_CHANGED, &scroll_callable) {
                v_scroll.call(
                    object::methods::CONNECT,
                    &[
                        range::signals::VALUE_CHANGED.to_variant(),
                        scroll_callable.to_variant(),
                        ConnectFlags::DEFERRED.ord().to_variant(),
                    ],
                );
            }
        }
        if let Some(mut h_scroll) = editor.get_h_scroll_bar() {
            if !h_scroll.is_connected(range::signals::VALUE_CHANGED, &scroll_callable) {
                h_scroll.call(
                    object::methods::CONNECT,
                    &[
                        range::signals::VALUE_CHANGED.to_variant(),
                        scroll_callable.to_variant(),
                        ConnectFlags::DEFERRED.ord().to_variant(),
                    ],
                );
            }
        }

        // Deferred connection prevents re-entrancy during draw/resize/visibility changes.
        if !editor.is_connected(canvas_item::signals::DRAW, &update_callable) {
            editor.call(
                object::methods::CONNECT,
                &[
                    canvas_item::signals::DRAW.to_variant(),
                    update_callable.to_variant(),
                    ConnectFlags::DEFERRED.ord().to_variant(),
                ],
            );
        }
        if !editor.is_connected(canvas_item::signals::VISIBILITY_CHANGED, &update_callable) {
            editor.call(
                object::methods::CONNECT,
                &[
                    canvas_item::signals::VISIBILITY_CHANGED.to_variant(),
                    update_callable.to_variant(),
                    ConnectFlags::DEFERRED.ord().to_variant(),
                ],
            );
        }
        if !editor.is_connected(control::signals::MINIMUM_SIZE_CHANGED, &update_callable) {
            editor.call(
                object::methods::CONNECT,
                &[
                    control::signals::MINIMUM_SIZE_CHANGED.to_variant(),
                    update_callable.to_variant(),
                    ConnectFlags::DEFERRED.ord().to_variant(),
                ],
            );
        }

        let mut settings = ProjectSettings::singleton();
        let update_callable = self.base_mut().callable(callbacks::ON_CURSOR_VISUAL_UPDATE);
        if !settings.is_connected(editor_settings::signals::SETTINGS_CHANGED, &update_callable) {
            settings.connect(editor_settings::signals::SETTINGS_CHANGED, &update_callable);
        }

        self.on_cursor_visual_update();
        self.update_cursor_visuals(&Mode::Normal, &mut editor);

        if let Some(mut line_manager) = self.ui.line_number_manager.clone() {
            line_manager.bind_mut().attach(editor.clone());
        }

        self.attached_editor = Some(editor.clone());

        self.refresh_cached_config();

        // Invalidate visual tracker so first action after attach does full update
        self.visuals.visual_tracker.invalidate_all();

        log::debug!("Attached to editor: {}", editor.get_name());
    }

    fn disconnect_signals(&mut self) {
        if let Some(mut editor) = self.attached_editor.take() {
            if !editor.is_instance_valid() {
                return;
            }

            let gui_callable = self.base().callable(callbacks::HANDLE_GUI_INPUT);
            if editor.is_connected(control::signals::GUI_INPUT, &gui_callable) {
                editor.disconnect(control::signals::GUI_INPUT, &gui_callable);
            }
            let caret_callable = self.base().callable(callbacks::ON_CARET_MOVED);
            if editor.is_connected(text_edit::signals::CARET_CHANGED, &caret_callable) {
                editor.disconnect(text_edit::signals::CARET_CHANGED, &caret_callable);
            }

            let update_callable = self.base().callable(callbacks::ON_CURSOR_VISUAL_UPDATE);
            let scroll_callable = self.base().callable(callbacks::ON_SCROLLBAR_CHANGED);
            if let Some(mut v_scroll) = editor.get_v_scroll_bar() {
                if v_scroll.is_connected(range::signals::VALUE_CHANGED, &scroll_callable) {
                    v_scroll.disconnect(range::signals::VALUE_CHANGED, &scroll_callable);
                }
            }
            if let Some(mut h_scroll) = editor.get_h_scroll_bar() {
                if h_scroll.is_connected(range::signals::VALUE_CHANGED, &scroll_callable) {
                    h_scroll.disconnect(range::signals::VALUE_CHANGED, &scroll_callable);
                }
            }

            if editor.is_connected(canvas_item::signals::DRAW, &update_callable) {
                editor.disconnect(canvas_item::signals::DRAW, &update_callable);
            }
            if editor.is_connected(canvas_item::signals::VISIBILITY_CHANGED, &update_callable) {
                editor.disconnect(canvas_item::signals::VISIBILITY_CHANGED, &update_callable);
            }

            editor.remove_theme_color_override(theme::CARET_COLOR);

            if let Some(mut cursor) = self.ui.cursor_visual.take() {
                editor.remove_child(&cursor.clone().upcast::<godot::classes::Node>());
                cursor.queue_free();
            }
        }
    }

    fn detach_fully(&mut self) {
        log::info!("Detaching from editor");
        self.disconnect_signals();

        if let Some(mut cmdline) = self.ui.cmdline.take() {
            if cmdline.is_instance_valid() {
                status_bar::restore_status_bar(&cmdline);

                if let Some(mut parent) = cmdline.get_parent() {
                    parent.remove_child(&cmdline.clone().upcast::<godot::classes::Node>());
                }

                cmdline.queue_free();
            }
        }

        self.base_mut().queue_free();
        log::info!("Detached successfully");
    }
}
