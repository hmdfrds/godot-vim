//! Signal handlers for VimController.
//!
//! This module extracts visual update and cursor synchronization logic
//! from the main VimController.

use crate::bridge::components::cursor_visual::VimCursor;
use crate::bridge::components::status_bar;
use crate::bridge::godot::names::theme;
use crate::bridge::settings;
use crate::bridge::vim_adapter::core::column_codec::{self, CoreByteCol};
use godot::classes::CanvasItem;
use godot::prelude::*;

use crate::bridge::vim_adapter::controller::cursor_geometry::compute_cursor_geometry;
use crate::bridge::vim_adapter::convert::mode_to_editor_mode;

/// Extension trait for signal handler implementations.
pub trait SignalHandlersTrait {
    /// Updates the visual cursor position and style.
    ///
    /// This handles:
    /// - Cmdline layout synchronization
    /// - Cursor creation/destruction based on settings
    /// - Visual mode cursor position correction
    /// - Tab width handling
    /// - Layout race protection
    fn update_cursor_visual(&mut self);
}

impl SignalHandlersTrait for crate::bridge::vim_wrapper::VimController {
    fn update_cursor_visual(&mut self) {
        // Re-apply cmdline sizing on every update to recover from layout changes
        // (e.g. sidebar appearing) that shift the floating overlay position.
        if let Some(cmdline) = self.ui.cmdline.as_ref().filter(|c| c.is_instance_valid()) {
            let cmdline = cmdline.clone();
            status_bar::configure_cmdline_sizing(&cmdline);

            // In CmdLine mode the HUD must always be visible; otherwise honor the user setting.
            if !self.engine.is_cmdline() {
                let mut base = cmdline.upcast::<CanvasItem>();
                base.set_visible(settings::VimSettings::cmdline_enabled());
            } else {
                let mut base = cmdline.upcast::<CanvasItem>();
                base.set_visible(true);
            }
        }

        let enabled = settings::VimSettings::enabled();

        if enabled {
            if self.ui.cursor_visual.is_none() {
                if let Some(mut editor) = self.get_editor() {
                    let vim_cursor = VimCursor::new_alloc();
                    let cursor_node: Gd<Node> = vim_cursor.clone().upcast();
                    editor.add_child(&cursor_node);
                    self.ui.cursor_visual = Some(vim_cursor);
                    editor.add_theme_color_override(
                        theme::CARET_COLOR,
                        Color::from_rgba(0.0, 0.0, 0.0, 0.0),
                    );
                } else {
                    return;
                }
            }
        } else {
            if let Some(mut cursor) = self.ui.cursor_visual.take() {
                cursor.queue_free();
            }
            // Restore the default line caret when the custom cursor is removed.
            if let Some(mut editor) = self.get_editor() {
                editor.remove_theme_color_override(theme::CARET_COLOR);
                editor.set_caret_type(godot::classes::text_edit::CaretType::LINE);
            }
            return;
        }

        let Some(mut cursor) = self.ui.cursor_visual.clone() else {
            return;
        };
        let Some(editor) = self.get_editor() else {
            return;
        };

        // In visual mode, use vim-state position as source of truth because
        // Godot's select() moves the caret to the selection endpoint.
        let cursor_pos = if self.engine.is_visual() {
            let p = self.engine.cursor_pos();
            let editor_col = column_codec::core_byte_to_editor_col_in_editor(
                &editor,
                p.line,
                CoreByteCol::new(p.col.as_usize()),
            );
            Some((p.line, editor_col))
        } else {
            None
        };

        let geometry = match compute_cursor_geometry(&editor, cursor_pos) {
            Some(geom) => geom,
            None => return, // Layout not yet ready.
        };

        cursor
            .bind_mut()
            .set_target(geometry.pos, geometry.height, geometry.width);

        let mode = self.engine.mode();
        cursor.bind_mut().set_mode(mode_to_editor_mode(&mode));
    }
}
