use crate::bridge::types::command::EditorCommand;
use vim_core::prelude::ShellRequest;

use super::external::external_command_to_editor_command;
use super::mode::mode_to_editor_mode;
use super::position::position_to_cursor;

/// Convert vim-core `ShellRequest` to shell `EditorCommand`.
///
/// This is the main translation point — every `ShellRequest` from vim-core
/// becomes an `EditorCommand` that the shell can dispatch.
#[must_use]
pub fn shell_request_to_command(req: &ShellRequest) -> EditorCommand {
    if let Some(command) = convert_mode_request(req) {
        return command;
    }
    if let Some(command) = convert_undo_redo_request(req) {
        return command;
    }
    if let Some(command) = convert_insert_paste_request(req) {
        return command;
    }
    if let Some(command) = convert_navigation_mark_request(req) {
        return command;
    }
    if let Some(command) = convert_search_replace_request(req) {
        return command;
    }
    if let Some(command) = convert_block_insert_request(req) {
        return command;
    }

    match req {
        ShellRequest::MacroStarted(register) => EditorCommand::MacroStarted(*register),
        ShellRequest::MacroStopped => EditorCommand::MacroStopped,
        ShellRequest::External(cmd) => external_command_to_editor_command(cmd),
        ShellRequest::ShowExpressionPrompt => EditorCommand::ShowExpressionPrompt,
        ShellRequest::ClipboardSet(text) => EditorCommand::ClipboardSet(text.clone()),
        _ => unreachable!("all ShellRequest variants should be handled"),
    }
}

fn convert_mode_request(req: &ShellRequest) -> Option<EditorCommand> {
    match req {
        ShellRequest::ModeChange {
            new_mode,
            previous_mode,
        } => Some(EditorCommand::ModeChange {
            new_mode: mode_to_editor_mode(new_mode),
            previous_mode: previous_mode.as_ref().map(mode_to_editor_mode),
        }),
        _ => None,
    }
}

fn convert_undo_redo_request(req: &ShellRequest) -> Option<EditorCommand> {
    match req {
        ShellRequest::Undo { count } => Some(EditorCommand::Undo { count: *count }),
        ShellRequest::Redo { count } => Some(EditorCommand::Redo { count: *count }),
        ShellRequest::Repeat { count } => Some(EditorCommand::Repeat { count: *count }),
        ShellRequest::UndoSync => Some(EditorCommand::UndoSync),
        ShellRequest::UndoNoSync => Some(EditorCommand::UndoNoSync),
        _ => None,
    }
}

fn convert_insert_paste_request(req: &ShellRequest) -> Option<EditorCommand> {
    match req {
        ShellRequest::Paste {
            after,
            register,
            count,
            adjust_indent,
            move_cursor_to_end,
        } => Some(EditorCommand::Paste {
            after: *after,
            register: *register,
            count: *count,
            adjust_indent: *adjust_indent,
            move_cursor_to_end: *move_cursor_to_end,
        }),
        ShellRequest::InsertRegister { name, literally } => Some(EditorCommand::InsertRegister {
            name: *name,
            literally: *literally,
        }),
        ShellRequest::TypeChar(c) => Some(EditorCommand::TypeChar(*c)),
        ShellRequest::Append { at_eol } => Some(EditorCommand::Append { at_eol: *at_eol }),
        ShellRequest::InsertAtFirstNonBlank => Some(EditorCommand::InsertAtFirstNonBlank),
        ShellRequest::InsertAtLastPosition => Some(EditorCommand::InsertAtLastPosition),
        ShellRequest::ExitInsertMode { text, count } => Some(EditorCommand::ExitInsertMode {
            text: text.clone(),
            count: *count,
        }),
        _ => None,
    }
}

fn convert_navigation_mark_request(req: &ShellRequest) -> Option<EditorCommand> {
    match req {
        ShellRequest::SetMark(c) => Some(EditorCommand::SetMark(*c)),
        ShellRequest::JumpToMark { name, exact } => Some(EditorCommand::JumpToMark {
            name: *name,
            exact: *exact,
        }),
        ShellRequest::JumpTo(pos) => Some(EditorCommand::JumpTo(position_to_cursor(pos))),
        ShellRequest::ScrollWindow { up } => Some(EditorCommand::ScrollWindow { up: *up }),
        ShellRequest::ViewportUpdate { top_line } => Some(EditorCommand::ViewportUpdate {
            top_line: *top_line,
        }),
        ShellRequest::Motion { motion, count } => Some(EditorCommand::Motion {
            motion: *motion,
            count: *count,
        }),
        _ => None,
    }
}

fn convert_search_replace_request(req: &ShellRequest) -> Option<EditorCommand> {
    match req {
        ShellRequest::Search(pattern, forward) => Some(EditorCommand::Search {
            pattern: pattern.clone(),
            forward: *forward,
        }),
        ShellRequest::FindAndReplace {
            pattern,
            replacement,
            flags,
        } => Some(EditorCommand::FindAndReplace {
            pattern: pattern.clone(),
            replacement: replacement.clone(),
            flags: flags.clone(),
        }),
        _ => None,
    }
}

fn convert_block_insert_request(req: &ShellRequest) -> Option<EditorCommand> {
    match req {
        ShellRequest::BeginBlockInsert { lines, col, origin } => {
            Some(EditorCommand::BeginBlockInsert {
                lines: *lines,
                col: *col,
                origin: position_to_cursor(origin),
            })
        }
        ShellRequest::BeginBlockAppend {
            lines,
            end_col,
            origin,
        } => Some(EditorCommand::BeginBlockAppend {
            lines: *lines,
            end_col: *end_col,
            origin: position_to_cursor(origin),
        }),
        ShellRequest::FinishBlockInsert {
            lines,
            col,
            text,
            origin,
        } => Some(EditorCommand::FinishBlockInsert {
            lines: *lines,
            col: *col,
            text: text.clone(),
            origin: position_to_cursor(origin),
        }),
        ShellRequest::FinishBlockAppend {
            lines,
            col,
            text,
            origin,
        } => Some(EditorCommand::FinishBlockAppend {
            lines: *lines,
            col: *col,
            text: text.clone(),
            origin: position_to_cursor(origin),
        }),
        ShellRequest::BlockInsertPreview { lines, col, text } => {
            Some(EditorCommand::BlockInsertPreview {
                lines: *lines,
                col: *col,
                text: text.clone(),
            })
        }
        ShellRequest::BlockInsertBackspace { lines, col, text } => {
            Some(EditorCommand::BlockInsertBackspace {
                lines: *lines,
                col: *col,
                text: text.clone(),
            })
        }
        _ => None,
    }
}
