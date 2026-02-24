//! Cursor and mode visual methods for VimController.
//!
//! Handles cursor appearance, scroll offset, and mode-based styling.

use crate::bridge::settings;
use crate::bridge::vim_adapter::convert::mode_to_editor_mode;
use crate::bridge::vim_adapter::handlers::visual;
use crate::bridge::vim_adapter::managers::mode;
use crate::bridge::vim_wrapper::VimController;
use godot::classes::CodeEdit;
use godot::prelude::*;
use vim_core::state::mode::Mode;

impl VimController {
    /// Updates command line and cursor visuals if mode has changed.
    pub(crate) fn update_mode_visuals_if_changed(&mut self, prev_mode: Mode) {
        if self.engine.mode() == prev_mode {
            return;
        }

        if let Some(cmdline) = self.ui.cmdline.as_mut().filter(|c| c.is_instance_valid()) {
            cmdline.bind_mut().update_mode(
                mode_to_editor_mode(&self.engine.mode()),
                self.engine.recording_register(),
            );
        }

        if let Some(mut editor) = self.get_editor() {
            let mode = self.engine.mode();
            self.update_cursor_visuals(&mode, &mut editor);
        }
    }

    /// Updates cursor visuals based on current mode.
    pub(crate) fn update_cursor_visuals(&mut self, mode: &Mode, editor: &mut Gd<CodeEdit>) {
        visual::update_cursor_visuals(mode, editor, &mut self.ui.cmdline);

        // If the custom cursor component is active, sync it and hide the native caret.
        if let Some(mut cursor) = self.ui.cursor_visual.clone() {
            cursor.bind_mut().set_mode(mode_to_editor_mode(mode));
            editor.add_theme_color_override("caret_color", Color::from_rgba(0.0, 0.0, 0.0, 0.0));
            Self::sync_highlight_current_line(editor);
            return;
        }

        // Apply mode-based cursor colors via ModeManager
        if let Some(color) = mode::get_mode_cursor_color(&mode_to_editor_mode(mode)) {
            editor.add_theme_color_override("caret_color", color);
        } else {
            // Mode colors disabled - use default theme color
            editor.remove_theme_color_override("caret_color");
        }

        // Sync highlight_current_line setting
        Self::sync_highlight_current_line(editor);
    }

    /// Applies scroll offset (scrolloff) - keeps N lines visible above/below cursor.
    ///
    /// This implements Vim's `scrolloff` behavior:
    /// - When cursor moves near top/bottom of viewport, adjust scroll to maintain margin
    /// - Does not affect the cursor position, only the viewport scroll
    #[allow(
        clippy::cast_possible_truncation,
        reason = "Line counts are always small positive integers"
    )]
    pub(crate) fn apply_scroll_offset(editor: &mut Gd<CodeEdit>) {
        let offset = settings::VimSettings::scroll_offset();
        if offset == 0 {
            return; // No offset configured
        }

        let caret_line = editor.get_caret_line();
        let first_visible = editor.get_first_visible_line();
        let last_visible = editor.get_last_full_visible_line();
        let visible_count = last_visible - first_visible;

        // Skip if viewport is too small for offset
        if visible_count <= offset * 2 {
            return;
        }

        // Check if cursor is too close to top
        if caret_line < first_visible + offset {
            // Scroll up: make cursor's line minus offset the first visible
            let new_first = (caret_line - offset).max(0);
            editor.set_line_as_first_visible(new_first);
        }
        // Check if cursor is too close to bottom
        else if caret_line > last_visible - offset {
            // Scroll down: make cursor's line plus offset the last visible
            let new_first = (caret_line - visible_count + offset).max(0);
            editor.set_line_as_first_visible(new_first);
        }
    }

    /// Syncs the `highlight_current_line` setting to `CodeEdit`'s property.
    pub(crate) fn sync_highlight_current_line(editor: &mut Gd<CodeEdit>) {
        let enabled = settings::VimSettings::highlight_current_line();
        if editor.is_highlight_current_line_enabled() != enabled {
            editor.set_highlight_current_line(enabled);
        }
    }

    /// Syncs search highlighting with Godot's CodeEdit.
    ///
    /// Delegates to [`SearchManager`] which handles caching and pattern filtering.
    pub(crate) fn sync_search_highlight(&mut self, editor: &mut Gd<CodeEdit>) {
        self.visuals
            .search_manager
            .sync_highlight(self.engine.last_search(), editor);
    }
}
