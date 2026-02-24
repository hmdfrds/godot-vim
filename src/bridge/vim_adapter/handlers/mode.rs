//! Mode handler trait for `VimController`.
//!
//! Handles mode change transitions and their side effects.

use crate::bridge::vim_adapter::core::cast::{i32_to_usize, usize_to_i32};
use crate::bridge::vim_wrapper::VimController;
use vim_core::state::mode::{CmdType, InsertMode, Mode, ReplaceMode, VisualKind};

/// Trait for handling mode transitions.
pub trait ModeHandler {
    /// Handle a mode change request with optional previous mode for `save_visual_selection`.
    fn handle_mode_change(&mut self, new_mode: Mode, previous_mode: Option<Mode>);
}

impl ModeHandler for VimController {
    fn handle_mode_change(&mut self, mut new_mode: Mode, previous_mode: Option<Mode>) {
        if let Some(cmdline) = self.ui.cmdline.as_mut().filter(|c| c.is_instance_valid()) {
            let editor_mode = crate::bridge::vim_adapter::convert::mode_to_editor_mode(&new_mode);
            cmdline
                .bind_mut()
                .update_mode(editor_mode, self.engine.recording_register());
            if let Mode::CmdLine(cmd_type) = new_mode {
                let prompt = match cmd_type {
                    CmdType::Ex => ":",
                    CmdType::ExVisualRange => ":'<,'>",
                    CmdType::SearchForward => "/",
                    CmdType::SearchBackward => "?",
                    CmdType::Filter => "!",
                };
                cmdline.bind_mut().start_input(prompt);
            }
        }

        if let Some(mut editor) = self.get_editor() {
            // Secondary carets remain visible when leaving VisualBlock; remove them explicitly.
            if matches!(self.engine.mode(), Mode::Visual(VisualKind::Block { .. }))
                && !matches!(new_mode, Mode::Visual(VisualKind::Block { .. }))
            {
                editor.remove_secondary_carets();
                // Re-enable caret blink that was disabled in visual block
                editor.set_caret_blink_enabled(true);
            }

            // vim-core initializes visual start positions as (0,0) when the anchor is not yet known.
            // Resolve to the actual caret position before the selection is rendered.
            // `previous_mode` is checked (not `vim_state.mode`) because vim-core has already
            // transitioned to the new mode by the time this handler runs.
            let was_visual = previous_mode.as_ref().is_some_and(|m| m.is_visual());
            if !was_visual {
                // Entering visual mode from non-visual - use current cursor as anchor
                let cursor = Self::cursor_from_editor(&editor);
                match &mut new_mode {
                    Mode::Visual(VisualKind::Char { start }) if start.line == 0 && start.col == 0 => {
                        *start = cursor;
                        self.engine.sync_cursor(cursor);
                        log::debug!("Hydrated Visual start from cursor: {:?}", cursor);
                    }
                    Mode::Visual(VisualKind::Line { start_line }) if *start_line == 0 => {
                        *start_line = cursor.line;
                        self.engine.sync_cursor(cursor);
                        log::debug!(
                            "Hydrated VisualLine start_line from cursor: {}",
                            cursor.line
                        );
                    }
                    Mode::Visual(VisualKind::Block {
                        start,
                        cursor: block_cursor,
                    }) if start.line == 0 && start.col == 0 => {
                        *start = cursor;
                        *block_cursor = cursor;
                        self.engine.sync_cursor(cursor);
                        log::debug!("Hydrated VisualBlock start from cursor: {:?}", cursor);
                    }
                    _ => {}
                }
            }

            match new_mode {
                Mode::Visual(_) => {
                    self.engine.set_mode(new_mode);
                }
                Mode::Normal => {
                    // Disable overtype mode when leaving Replace mode
                    editor.set_overtype_mode_enabled(false);
                    // Dismiss code completion popup and parameter hint tooltip
                    editor.cancel_code_completion();
                    editor.set_code_hint(""); // Dismiss parameter tooltip
                                              // Save visual selection for 'gv' command using PREVIOUS mode
                                              // (vim_state.mode is already Normal at this point from processor)
                    if let Some(ref prev) = previous_mode {
                        // Save insert position for 'gi' command when leaving Insert mode
                        if matches!(
                            prev,
                            Mode::Insert(..) | Mode::Replace(ReplaceMode::Overwrite) | Mode::Replace(ReplaceMode::Virtual)
                        ) {
                            let pos = vim_core::domain::position::Position::new(
                                i32_to_usize(editor.get_caret_line()),
                                i32_to_usize(editor.get_caret_column()),
                            );
                            self.engine.set_last_insert(pos);
                            log::debug!("Saved last insert position: {:?}", pos);
                        }
                    }
                    editor.remove_secondary_carets();
                    editor.deselect();
                    // Search highlights (/pattern, *, #) persist until ESC is pressed;
                    // they are not cleared on Normal mode entry.
                    self.engine.set_mode(new_mode);
                }
                Mode::Insert(InsertMode::Standard { .. }) => {
                    editor.set_overtype_mode_enabled(false);
                    self.engine.set_mode(new_mode);
                }
                Mode::Insert(InsertMode::BlockInsert { lines, col, .. }) => {
                    editor.set_overtype_mode_enabled(false);
                    // BlockInsert originates either from a transaction (block c/s, lines/col
                    // already correct) or from ShellRequest::BeginBlockInsert (I in visual block,
                    // handled by block_ops). In both cases only the editor state needs setup.
                    log::debug!(
                        "Block insert: lines ({}, {}), col {}",
                        lines.0,
                        lines.1,
                        col
                    );
                    editor.remove_secondary_carets();
                    editor.deselect();
                    editor.set_caret_line(usize_to_i32(lines.0));
                    editor.set_caret_column(usize_to_i32(col));
                    self.engine.set_mode(new_mode);
                }
                Mode::Insert(InsertMode::BlockAppend { .. }) => {
                    editor.set_overtype_mode_enabled(false);
                    self.engine.set_mode(new_mode);
                }
                Mode::Insert(..) => {
                    // Other insert sub-modes (Register, Literal, etc.)
                    editor.set_overtype_mode_enabled(false);
                    self.engine.set_mode(new_mode);
                }
                Mode::Replace(ReplaceMode::Overwrite) | Mode::Replace(ReplaceMode::Virtual) => {
                    // Enable Godot's native overtype mode for usage.
                    // For VirtualReplace (gR), this overwrites existing chars.
                    // True "Virtual" behavior regarding tabs requires specific input handling,
                    // but overtype is the baseline.
                    editor.set_overtype_mode_enabled(true);
                    self.engine.set_mode(new_mode);
                }
                _ => self.engine.set_mode(new_mode),
            }
            self.update_cursor_visuals(&new_mode, &mut editor);
        }
        log::debug!("Mode changed to {new_mode:?}");
    }
}
