use crate::bridge::types::command::EditorCommand;
use vim_core::domain::external_command::ExternalCommand;

/// Convert vim-core `ExternalCommand` to shell `EditorCommand`.
#[must_use]
pub fn external_command_to_editor_command(cmd: &ExternalCommand) -> EditorCommand {
    match cmd {
        ExternalCommand::Save => EditorCommand::Save,
        ExternalCommand::Quit => EditorCommand::Quit,
        ExternalCommand::SaveQuit => EditorCommand::SaveQuit,
        ExternalCommand::QuitNoSave => EditorCommand::QuitNoSave,
        ExternalCommand::BufferReopen => EditorCommand::BufferReopen,
        ExternalCommand::BufferNext => EditorCommand::BufferNext,
        ExternalCommand::BufferPrev => EditorCommand::BufferPrev,
        ExternalCommand::BufferGoto(idx) => EditorCommand::BufferGoto(*idx),
        ExternalCommand::SearchNext => EditorCommand::SearchNext,
        ExternalCommand::SearchPrev => EditorCommand::SearchPrev,
        ExternalCommand::SearchWordForward => EditorCommand::SearchWordForward,
        ExternalCommand::SearchWordBackward => EditorCommand::SearchWordBackward,
        ExternalCommand::SearchWordPartialForward => EditorCommand::SearchWordPartialForward,
        ExternalCommand::SearchWordPartialBackward => EditorCommand::SearchWordPartialBackward,
        ExternalCommand::FoldOpen => EditorCommand::FoldOpen,
        ExternalCommand::FoldClose => EditorCommand::FoldClose,
        ExternalCommand::FoldToggle => EditorCommand::FoldToggle,
        ExternalCommand::FoldAll => EditorCommand::FoldAll,
        ExternalCommand::UnfoldAll => EditorCommand::UnfoldAll,
        ExternalCommand::OpenLineBelow { count } => EditorCommand::OpenLineBelow { count: *count },
        ExternalCommand::OpenLineAbove { count } => EditorCommand::OpenLineAbove { count: *count },
        ExternalCommand::Backspace => EditorCommand::Backspace,
        ExternalCommand::ReplaceChar(c) => EditorCommand::ReplaceChar(*c),
        ExternalCommand::InsertText(text) => EditorCommand::InsertText(text.clone()),
        ExternalCommand::GotoDefinition => EditorCommand::GotoDefinition,
        ExternalCommand::GoToLine { line } => EditorCommand::GoToLine { line: *line },
        ExternalCommand::ShowDocumentation => EditorCommand::ShowDocumentation,
        ExternalCommand::CompletionNext => EditorCommand::CompletionNext,
        ExternalCommand::CompletionPrev => EditorCommand::CompletionPrev,
        ExternalCommand::CompletionAccept => EditorCommand::CompletionAccept,
        ExternalCommand::CompletionCancel => EditorCommand::CompletionCancel,
        ExternalCommand::IncrementNumber { count } => {
            EditorCommand::IncrementNumber { count: *count }
        }
        ExternalCommand::DecrementNumber { count } => {
            EditorCommand::DecrementNumber { count: *count }
        }
        ExternalCommand::Read { command, path } => EditorCommand::Read {
            command: *command,
            path: path.clone(),
        },
        ExternalCommand::ListRegisters => EditorCommand::ListRegisters,
        ExternalCommand::Source { path } => EditorCommand::Source { path: path.clone() },
        ExternalCommand::ExecuteLastEx => EditorCommand::ExecuteLastEx,
        ExternalCommand::ExecuteRegister { register } => EditorCommand::ExecuteRegister {
            register: *register,
        },
        ExternalCommand::Custom { cmd, args } => EditorCommand::Custom {
            cmd: cmd.clone(),
            args: args.clone(),
        },
        ExternalCommand::ShowExpressionPrompt => EditorCommand::ShowExpressionPrompt,
        ExternalCommand::Message(text) => EditorCommand::Message(text.clone()),
        ExternalCommand::SetClipboard(text) => EditorCommand::ClipboardSet(text.clone()),
        ExternalCommand::Sleep { milliseconds } => EditorCommand::Sleep {
            milliseconds: *milliseconds,
        },
        ExternalCommand::RepeatSubstituteAllLines => EditorCommand::RepeatSubstituteAllLines,
        // Range-based operations and filter stay as Messages until full migration
        ExternalCommand::DeleteRange { .. }
        | ExternalCommand::YankRange { .. }
        | ExternalCommand::WriteRange { .. }
        | ExternalCommand::Substitute { .. }
        | ExternalCommand::Filter { .. }
        | ExternalCommand::IndentLines { .. }
        | ExternalCommand::Global { .. } => {
            EditorCommand::Message(format!("Range command: {:?}", cmd))
        }
    }
}
