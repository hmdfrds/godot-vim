use crate::bridge::types::command::EditorCommand;
use crate::bridge::vim_adapter::convert::cursor_to_position;
use crate::bridge::vim_adapter::handlers::block_ops::BlockOpsHandler;
use crate::bridge::vim_adapter::handlers::external_cmd::ExternalCmdHandler;
use crate::bridge::vim_adapter::handlers::marks::MarksHandler;
use crate::bridge::vim_adapter::handlers::mode::ModeHandler;
use crate::bridge::vim_wrapper::VimController;
use godot::obj::Singleton;
use vim_core::state::mode::Mode;

impl VimController {
    fn handle_clipboard(&mut self, text: String) {
        godot::classes::DisplayServer::singleton()
            .clipboard_set(&godot::prelude::GString::from(&text));
    }

    /// Dispatch a single `EditorCommand` to the appropriate handler.
    pub(super) fn dispatch_editor_command(&mut self, cmd: EditorCommand) {
        match cmd {
            // Mode
            EditorCommand::ModeChange { .. } => {
                let current_mode = self.engine.mode();
                self.handle_mode_change(current_mode, None);
            }

            // Undo/Redo
            EditorCommand::Undo { count } => self.handle_undo(count),
            EditorCommand::Redo { count } => self.handle_redo(count),
            EditorCommand::UndoSync => self.handle_undo_sync(),
            EditorCommand::UndoNoSync => log::debug!("UndoNoSync requested (Ctrl-G U)"),

            // Paste / Registers
            EditorCommand::Paste {
                after,
                register,
                count,
                adjust_indent,
                move_cursor_to_end,
            } => {
                self.handle_paste(after, register, count, adjust_indent, move_cursor_to_end);
            }
            EditorCommand::InsertRegister { name, literally } => {
                self.handle_paste(false, Some(name), 1, false, !literally);
            }

            // Marks
            EditorCommand::SetMark(c) => self.handle_set_mark(c),
            EditorCommand::JumpToMark { name, exact } => self.handle_jump_to_mark(name, exact),
            EditorCommand::JumpTo(pos) => self.handle_jump_to(cursor_to_position(&pos)),

            // Macros
            EditorCommand::MacroStarted(register) => {
                log::debug!("Started recording macro @{}", register);
                self.handle_mode_change(Mode::Recording { register }, None);
            }
            EditorCommand::MacroStopped => {
                log::debug!("Stopped recording macro");
                self.handle_mode_change(Mode::Normal, None);
            }

            // Motion
            EditorCommand::Motion { motion, count } => {
                self.handle_motion_with_count(motion, count);
            }

            // Block operations
            EditorCommand::BeginBlockInsert { lines, col, origin } => {
                self.handle_begin_block_insert(lines, col, cursor_to_position(&origin));
            }
            EditorCommand::BeginBlockAppend {
                lines,
                end_col,
                origin,
            } => {
                self.handle_begin_block_append(lines, end_col, cursor_to_position(&origin));
            }
            EditorCommand::FinishBlockInsert {
                lines,
                col,
                text,
                origin,
            } => {
                self.with_editor(|s, e| {
                    s.handle_finish_block_insert(e, lines, col, &text, cursor_to_position(&origin))
                });
            }
            EditorCommand::FinishBlockAppend {
                lines,
                col,
                text,
                origin,
            } => {
                self.with_editor(|s, e| {
                    s.handle_finish_block_append(e, lines, col, &text, cursor_to_position(&origin))
                });
            }
            EditorCommand::BlockInsertPreview { lines, col, text } => {
                self.with_editor(|s, e| s.handle_block_insert_preview(e, lines, col, &text));
            }
            EditorCommand::BlockInsertBackspace { lines, col, text } => {
                self.with_editor(|s, e| s.handle_block_insert_backspace(e, lines, col, &text));
            }

            // Text editing
            EditorCommand::TypeChar(c) => self.handle_type_char(c),
            EditorCommand::Append { at_eol } => {
                self.with_editor(|s, e| s.handle_append(e, at_eol));
            }
            EditorCommand::InsertAtFirstNonBlank => {
                self.with_editor(|s, e| s.handle_insert_first_nonblank(e));
            }
            EditorCommand::InsertAtLastPosition => {
                self.with_editor(|s, e| s.handle_insert_at_last_position(e));
            }
            EditorCommand::ExitInsertMode { text, count } => {
                self.handle_exit_insert_mode(text, count)
            }
            EditorCommand::Repeat { count } => self.handle_repeat(count),

            // Clipboard
            EditorCommand::ClipboardSet(text) => self.handle_clipboard(text),

            // Viewport
            EditorCommand::ScrollWindow { up } => self.handle_scroll_window(up),
            EditorCommand::ViewportUpdate { top_line } => self.handle_viewport_update(top_line),

            // Search
            EditorCommand::Search { pattern, forward } => self.dispatch_search(pattern, forward),
            EditorCommand::FindAndReplace {
                pattern,
                replacement,
                flags,
            } => {
                self.dispatch_find_and_replace(pattern, replacement, flags);
            }

            // Deferred commands
            EditorCommand::BufferNext
            | EditorCommand::BufferPrev
            | EditorCommand::BufferGoto(_)
            | EditorCommand::Custom { .. } => {
                self.handle_deferred_command(cmd);
            }

            // External commands
            cmd @ (EditorCommand::Save
            | EditorCommand::Quit
            | EditorCommand::SaveQuit
            | EditorCommand::QuitNoSave
            | EditorCommand::BufferReopen
            | EditorCommand::SearchNext
            | EditorCommand::SearchPrev
            | EditorCommand::SearchWordForward
            | EditorCommand::SearchWordBackward
            | EditorCommand::SearchWordPartialForward
            | EditorCommand::SearchWordPartialBackward
            | EditorCommand::FoldOpen
            | EditorCommand::FoldClose
            | EditorCommand::FoldToggle
            | EditorCommand::FoldAll
            | EditorCommand::UnfoldAll
            | EditorCommand::OpenLineBelow { .. }
            | EditorCommand::OpenLineAbove { .. }
            | EditorCommand::Backspace
            | EditorCommand::ReplaceChar(_)
            | EditorCommand::InsertText(_)
            | EditorCommand::GotoDefinition
            | EditorCommand::GoToLine { .. }
            | EditorCommand::ShowDocumentation
            | EditorCommand::CompletionNext
            | EditorCommand::CompletionPrev
            | EditorCommand::CompletionAccept
            | EditorCommand::CompletionCancel
            | EditorCommand::IncrementNumber { .. }
            | EditorCommand::DecrementNumber { .. }
            | EditorCommand::Read { .. }
            | EditorCommand::ListRegisters
            | EditorCommand::Source { .. }
            | EditorCommand::ExecuteLastEx
            | EditorCommand::ExecuteRegister { .. }
            | EditorCommand::Sleep { .. }
            | EditorCommand::RepeatSubstituteAllLines
            | EditorCommand::Message(_)
            | EditorCommand::ShowExpressionPrompt) => {
                self.handle_external_command(cmd);
            }
        }

        if let Some(editor) = self.get_editor() {
            self.engine.sync_cursor(Self::cursor_from_editor(&editor));
        }
    }
}
