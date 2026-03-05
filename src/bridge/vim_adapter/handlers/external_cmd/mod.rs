//! External command handler - dispatches to domain-specific modules.
//!
//! # Module Structure
//! - `file_ops.rs` - Save, Quit, WriteRange, BufferReopen
//! - `buffer_nav.rs` - BufferNext, BufferPrev, BufferGoto
//! - `search.rs` - SearchNext, SearchPrev, SearchWord*
//! - `fold.rs` - FoldOpen, FoldClose, FoldToggle, FoldAll
//! - `text_editing.rs` - Backspace, ReplaceChar, Delete, Yank, Substitute
//! - `navigation.rs` - GotoDefinition, ShowDocumentation
//! - `debug.rs` - ToggleBreakpoint, DebugContinue, etc.
//! - `number_ops.rs` - IncrementNumber, DecrementNumber
//! - `global_cmd.rs` - Global/Substitute operations
//! - `registers.rs` - ListRegisters

mod buffer_nav;
mod debug;
mod editor;
mod file_ops;
mod fold;
mod navigation;
mod number_ops;
mod registers;
mod scene;
mod search;
mod text_editing;

use crate::bridge::navigation::window::nav::{handle_window_nav, NavDirection, WindowNavResult};
use crate::bridge::types::command::EditorCommand;
use crate::bridge::vim_adapter::core::cast::i32_to_usize;
use crate::bridge::vim_wrapper::VimController;
use godot::classes::{CodeEdit, DisplayServer};
use godot::prelude::*;
use vim_core::inputs::{KeyCode, VimKey, VimModifiers};

/// Defines how focus should be handled after an external command.
#[derive(Debug, Clone, Copy, PartialEq, Eq, strum::Display)]
pub enum FocusBehavior {
    /// The caller should restore focus to the active editor.
    Restore,
    /// The command has handled focus (e.g. context switch); caller should skip restoration.
    Skip,
}

/// Trait for handling external Vim commands.
///
/// Accepts `EditorCommand` directly — no reconversion to vim-core types.
pub trait ExternalCmdHandler {
    /// Handle an external command.
    ///
    /// Returns `FocusBehavior` to instruct the caller on focus restoration.
    fn handle_external_command(&mut self, cmd: EditorCommand) -> FocusBehavior;
}

impl ExternalCmdHandler for VimController {
    fn handle_external_command(&mut self, cmd: EditorCommand) -> FocusBehavior {
        let Some(mut editor) = self.get_editor() else {
            return FocusBehavior::Restore;
        };

        match cmd {
            EditorCommand::Save => self.handle_save(),
            EditorCommand::Quit => self.handle_quit(),
            EditorCommand::SaveQuit => self.handle_save_quit(),
            EditorCommand::QuitNoSave => self.handle_quit_no_save(),
            EditorCommand::BufferReopen => self.handle_buffer_reopen(),

            EditorCommand::BufferNext => return self.handle_buffer_nav(1),
            EditorCommand::BufferPrev => return self.handle_buffer_nav(-1),
            EditorCommand::BufferGoto(idx) => return self.handle_buffer_goto(idx as usize),

            // Search operations
            EditorCommand::SearchNext => self.handle_search_repeat(&mut editor, true),
            EditorCommand::SearchPrev => self.handle_search_repeat(&mut editor, false),
            EditorCommand::SearchWordForward => self.handle_search_word(&mut editor, true),
            EditorCommand::SearchWordBackward => self.handle_search_word(&mut editor, false),
            EditorCommand::SearchWordPartialForward => {
                // g* and g# - partial word search (delegates to whole-word search)
                self.handle_search_word(&mut editor, true);
            }
            EditorCommand::SearchWordPartialBackward => {
                self.handle_search_word(&mut editor, false);
            }

            // Fold operations
            EditorCommand::FoldOpen => fold::handle_fold_open(&mut editor),
            EditorCommand::FoldClose => fold::handle_fold_close(&mut editor),
            EditorCommand::FoldToggle => fold::handle_fold_toggle(&mut editor),
            EditorCommand::FoldAll => editor.fold_all_lines(),
            EditorCommand::UnfoldAll => editor.unfold_all_lines(),

            // Line operations
            EditorCommand::OpenLineBelow { count } => {
                self.handle_open_line(&mut editor, true, count as usize)
            }
            EditorCommand::GoToLine { line } => {
                self.handle_goto_line(&mut editor, line as usize);
            }
            EditorCommand::OpenLineAbove { count } => {
                self.handle_open_line(&mut editor, false, count as usize)
            }

            // Text editing
            EditorCommand::Backspace => editor.backspace(),
            EditorCommand::ReplaceChar(c) => text_editing::handle_replace_char(&mut editor, c),
            EditorCommand::InsertText(text) => editor.insert_text_at_caret(&text),

            // Navigation
            EditorCommand::GotoDefinition => self.handle_goto_definition(&mut editor),
            EditorCommand::ShowDocumentation => navigation::handle_show_documentation(&mut editor),

            // Completion hints
            EditorCommand::CompletionNext => {
                let current = editor.get_code_completion_selected_index();
                if current >= 0 {
                    editor.set_code_completion_selected_index(current + 1);
                }
            }
            EditorCommand::CompletionPrev => {
                let current = editor.get_code_completion_selected_index();
                if current > 0 {
                    editor.set_code_completion_selected_index(current - 1);
                }
            }
            EditorCommand::CompletionAccept => {
                let start_line = editor.get_caret_line();
                let start_col = editor.get_caret_column();

                // Delegate confirmation and state sync to manager
                self.engine.confirm_completion(&mut editor);

                // Capture the completed text for dot-repeat and macro recording
                let end_line = editor.get_caret_line();
                let end_col = editor.get_caret_column();

                if start_line == end_line && end_col > start_col {
                    let line_text = editor.get_line(start_line).to_string();
                    let inserted: String = line_text
                        .chars()
                        .skip(start_col as usize)
                        .take((end_col - start_col) as usize)
                        .collect();

                    log::debug!("Appending completion to repeat buffer: '{}'", inserted);
                    self.engine.record_insert_str(&inserted);

                    // Record completion text to macro buffer as synthetic key presses
                    // Replace the Enter key (already recorded) with actual inserted text
                    if self.engine.recording_register().is_some() {
                        // Remove the Enter key that triggered this completion
                        self.engine.macro_buffer_replace_last_enter();
                        // Add the actual completion text as key presses
                        for c in inserted.chars() {
                            self.engine.record_macro_key(VimKey::new(
                                KeyCode::Char(c),
                                VimModifiers::NONE,
                            ));
                        }
                        log::debug!(
                            "Recorded {} completion chars to macro buffer",
                            inserted.len()
                        );
                    }
                }
            }
            EditorCommand::CompletionCancel => {
                editor.cancel_code_completion();
            }

            // Custom Shell Commands (Godot specific)
            EditorCommand::Custom { cmd, args } => {
                return self.handle_custom_command(&cmd, args, &editor);
            }

            // Number manipulation
            EditorCommand::IncrementNumber { count } => {
                #[allow(clippy::cast_possible_wrap, reason = "count is always small positive")]
                number_ops::modify_number_at_cursor(&mut editor, count as i64);
            }
            EditorCommand::DecrementNumber { count } => {
                #[allow(clippy::cast_possible_wrap, reason = "count is always small positive")]
                number_ops::modify_number_at_cursor(&mut editor, -(count as i64));
            }

            // Read (handled by processor eagerly)
            EditorCommand::Read { .. } => {}

            // Scripting / Global
            EditorCommand::Source { path } => {
                self.handle_source(path);
            }

            EditorCommand::ExecuteRegister { register } => {
                if let Some((text, _mode)) = self.engine.register_get(register) {
                    let content = text.to_string();
                    self.handle_execute_script(content);
                }
            }

            EditorCommand::ExecuteLastEx => {
                log::warn!(":@@ command not fully implemented yet");
            }

            EditorCommand::ListRegisters => {
                self.handle_list_registers();
            }

            EditorCommand::RepeatSubstituteAllLines => {
                let Some(last_line_i32) =
                    crate::bridge::vim_adapter::core::column_codec::last_line_index(&editor)
                else {
                    return FocusBehavior::Restore;
                };
                let last_line = i32_to_usize(last_line_i32);
                let (pat, repl, flags) = self.engine.last_substitute();
                if let (Some(pattern), Some(replacement), Some(flags)) = (pat, repl, flags) {
                    text_editing::execute_substitute_range(
                        &mut editor,
                        0,
                        last_line,
                        pattern,
                        replacement,
                        flags,
                    );
                }
            }

            EditorCommand::Sleep { .. } => {}
            EditorCommand::ShowExpressionPrompt => {
                log::info!("Expression register prompt requested");
            }
            EditorCommand::Message(msg) => {
                self.show_cmdline_message(&msg);
            }
            EditorCommand::ClipboardSet(text) => {
                let mut ds = DisplayServer::singleton();
                ds.clipboard_set(&GString::from(&text));
            }

            // Commands already handled inline by dispatch_editor_command
            // (mode, motion, undo/redo, paste, marks, macros, block ops,
            //  typing, append, search, find-replace, viewport, repeat, etc.)
            // If they reach here, it means dispatch didn't handle them — log a warning.
            other => {
                log::warn!(
                    "ExternalCmdHandler received unexpected command: {:?}",
                    other
                );
            }
        }

        FocusBehavior::Restore
    }
}

impl VimController {
    /// Handle shell-registered custom commands.
    fn handle_custom_command(
        &mut self,
        cmd: &str,
        args: Vec<String>,
        editor: &Gd<CodeEdit>,
    ) -> FocusBehavior {
        match cmd {
            // Buffer navigation
            "bn" | "bnext" => return self.handle_buffer_nav(1),
            "bp" | "bprev" | "bprevious" => return self.handle_buffer_nav(-1),

            // Debug operations
            "GodotBreakpoint" | "toggle_breakpoint" => {
                debug::handle_toggle_breakpoint(&mut editor.clone());
            }
            "GodotContinue" | "debug_continue" => debug::handle_debug_continue(),
            "GodotNext" | "debug_next" => debug::handle_debug_next(),
            "GodotStepIn" | "debug_step_in" => debug::handle_debug_step_in(),
            "GodotStepOut" | "debug_step_out" => debug::handle_debug_step_out(),
            "GodotPause" | "debug_pause" => debug::handle_debug_pause(),

            "Scene" => {
                crate::bridge::vim_adapter::handlers::dock::handle_dock_focus(
                    crate::bridge::vim_adapter::handlers::dock::DockTarget::Scene,
                );
                return FocusBehavior::Skip;
            }
            "FileSystem" => {
                crate::bridge::vim_adapter::handlers::dock::handle_dock_focus(
                    crate::bridge::vim_adapter::handlers::dock::DockTarget::FileSystem,
                );
                return FocusBehavior::Skip;
            }
            "Inspector" => {
                crate::bridge::vim_adapter::handlers::dock::handle_dock_focus(
                    crate::bridge::vim_adapter::handlers::dock::DockTarget::Inspector,
                );
                return FocusBehavior::Skip;
            }
            "Script" => {
                crate::bridge::vim_adapter::handlers::dock::handle_dock_focus(
                    crate::bridge::vim_adapter::handlers::dock::DockTarget::Script,
                );
                return FocusBehavior::Skip;
            }
            "FocusDock" => {
                if let Some(arg) = args.first() {
                    let target = match arg.to_lowercase().as_str() {
                        "scene" => crate::bridge::vim_adapter::handlers::dock::DockTarget::Scene,
                        "filesystem" | "files" => {
                            crate::bridge::vim_adapter::handlers::dock::DockTarget::FileSystem
                        }
                        "inspector" => {
                            crate::bridge::vim_adapter::handlers::dock::DockTarget::Inspector
                        }
                        "script" => crate::bridge::vim_adapter::handlers::dock::DockTarget::Script,
                        "output" => crate::bridge::vim_adapter::handlers::dock::DockTarget::Output,
                        "2d" => crate::bridge::vim_adapter::handlers::dock::DockTarget::Editor2D,
                        "3d" => crate::bridge::vim_adapter::handlers::dock::DockTarget::Editor3D,
                        _ => return FocusBehavior::Restore,
                    };
                    crate::bridge::vim_adapter::handlers::dock::handle_dock_focus(target);
                    return FocusBehavior::Skip;
                }
            }

            "WindowNav" | "window_nav" => {
                if let Some(dir_str) = args.first() {
                    let nav_dir = match dir_str.as_str() {
                        "left" => NavDirection::Left,
                        "right" => NavDirection::Right,
                        "up" => NavDirection::Prev,
                        "down" => NavDirection::Next,
                        _ => return FocusBehavior::Restore,
                    };

                    let control = editor.clone().upcast::<godot::classes::Control>();
                    let result = handle_window_nav(&control, nav_dir);
                    match result {
                        WindowNavResult::Focused(target) => {
                            self.observe_dock_control(target);
                            return FocusBehavior::Skip;
                        }
                        WindowNavResult::Ignored => {}
                    }
                }
            }

            // Scene Control
            "run" | "play" => {
                scene::handle_play_main();
            }
            "runcurrent" | "playcurrent" => {
                scene::handle_play_current();
            }
            "stop" => {
                scene::handle_stop();
            }

            // Script/File Save (syncs buffer state properly)
            "save" => {
                self.handle_save(); // Uses file_ops::handle_save which properly clears the `*` marker
            }
            "saveall" => {
                self.handle_save_all();
            }
            // Scene save (for .tscn files)
            "savescene" => {
                if let Err(e) = scene::handle_save() {
                    self.show_cmdline_message(&e);
                }
            }

            // Editor State
            "zen" => {
                editor::handle_zen(true);
            }
            "unzen" => {
                editor::handle_zen(false);
            }
            "restart" => {
                editor::handle_restart();
            }

            // Buffer goto (b1, b2, ..., b9)
            cmd if cmd.starts_with('b') && cmd.len() > 1 => {
                if let Ok(idx) = cmd[1..].parse::<usize>() {
                    return self.handle_buffer_goto(idx);
                }
                log::warn!("Invalid buffer number: {}", cmd);
            }

            _ => log::warn!("Unhandled custom command: {}", cmd),
        }
        FocusBehavior::Restore
    }
}
