//! Signal handlers for VimController.
//!
//! This module extracts visual update and cursor synchronization logic
//! from the main VimController.

use crate::bridge::components::cursor_visual::VimCursor;
use crate::bridge::components::status_bar;
use crate::bridge::godot::names::theme;
use crate::bridge::settings;
use godot::classes::{CanvasItem, CodeEdit};
use godot::prelude::*;

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
            Some((p.line, p.col))
        } else {
            None
        };

        let (target_pos, target_height, target_width) =
            match calculate_cursor_geometry(&editor, cursor_pos) {
                Some(geom) => geom,
                None => return, // Layout not yet ready.
            };

        cursor
            .bind_mut()
            .set_target(target_pos, target_height, target_width);

        let mode = self.engine.mode();
        cursor.bind_mut().set_mode(mode_to_editor_mode(&mode));
    }
}

/// Pure function to calculate cursor geometry from editor state.
///
/// `override_pos` - If provided, use this position instead of reading from editor.
///                  Used in visual mode where Godot's select() corrupts the caret position.
///
/// Returns `None` if the layout is not ready (e.g., during file switch).
/// Returns `Some((position, height, width))` on success.
fn calculate_cursor_geometry(
    editor: &Gd<CodeEdit>,
    override_pos: Option<(usize, usize)>,
) -> Option<(Vector2, f32, f32)> {
    let line_height = editor.get_line_height() as f32;
    let font = editor.get_theme_font(theme::FONT)?;
    let font_size = editor.get_theme_font_size(theme::FONT_SIZE);
    let char_width = font.get_char_size('m' as u32, font_size).x;

    // Use override position if provided (visual mode), otherwise read from editor
    let (line, col) = if let Some((l, c)) = override_pos {
        (l as i32, c as i32)
    } else {
        (editor.get_caret_line(), editor.get_caret_column())
    };

    let rect = editor.get_rect_at_line_column(line, col);

    // Godot's get_rect_at_line_column has an off-by-one issue where columns 0 and 1
    // return the same X position. Detection: compare rects for columns 0 and 1.
    // When the bug is present, add one character width to compensate. When the first
    // character is a tab and col == 1, the offset equals the tab width instead.
    let target_x = if col >= 1 {
        let rect_0 = editor.get_rect_at_line_column(line, 0);
        let rect_1 = editor.get_rect_at_line_column(line, 1);
        if rect_0.position.x == rect_1.position.x && rect_0.position.x > 0 {
            let line_text = editor.get_line(line).to_string();
            let first_char = line_text.chars().next();

            let offset_width = if let Some('\t') = first_char {
                if col == 1 {
                    let tab_size = editor.get_indent_size();
                    (tab_size as f32) * char_width
                } else {
                    char_width
                }
            } else {
                char_width
            };

            (rect.position.x as f32) + offset_width
        } else {
            rect.position.x as f32
        }
    } else {
        rect.position.x as f32
    };

    let target_y = rect.position.y as f32;
    let target_pos = Vector2::new(target_x, target_y);

    let mut target_height = rect.size.y as f32;
    let mut target_width = rect.size.x as f32;

    // Use actual character width for non-tab characters (avoids oversized cursor on tabs).
    let text = editor.get_line(line).to_string();
    let char_at_col = text.chars().nth(col as usize).unwrap_or('?');
    let is_tab = char_at_col == '\t';

    if !is_tab {
        // Force standard width for normal chars (and EOL)
        target_width = char_width;
    }

    // If height is zero, fall back to the line height.
    if target_height < 0.1 {
        target_height = line_height;
    }

    // If Godot reports Y=0.0 for a line > 0, the layout is not ready.
    if target_pos.y.abs() < f32::EPSILON && editor.get_caret_line() > 0 {
        return None;
    }

    Some((target_pos, target_height, target_width))
}
