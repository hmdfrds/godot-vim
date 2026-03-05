//! External command queue for VimController.
//!
//! Provides deferred execution of external commands to avoid
//! re-entrant borrow panics when commands trigger focus or visibility changes.

use crate::bridge::types::command::EditorCommand;
use crate::bridge::vim_adapter::handlers::external_cmd::{ExternalCmdHandler, FocusBehavior};
use crate::bridge::vim_wrapper::VimController;
use godot::prelude::*;

fn drain_pending_commands(pending: &mut Vec<EditorCommand>) -> Vec<EditorCommand> {
    std::mem::take(pending)
}

fn should_restore_focus(behavior: FocusBehavior, editor_has_focus: bool) -> bool {
    behavior == FocusBehavior::Restore && !editor_has_focus
}

impl VimController {
    /// Executes an external command immediately.
    ///
    /// Dispatches directly to the command handler instead of re-parsing through
    /// the Ex engine, which would create an infinite loop (Custom → Ex parse →
    /// Custom → deferred → Ex parse → …).
    pub(crate) fn execute_command_deferred_impl(&mut self, cmd: String, args: PackedStringArray) {
        let args_vec: Vec<String> = args.as_slice().iter().map(|s| s.to_string()).collect();
        let editor_cmd = EditorCommand::Custom {
            cmd,
            args: args_vec,
        };
        self.perform_editor_command_immediate(editor_cmd);
    }

    /// Process the command queue safely at the end of the frame.
    pub(crate) fn flush_command_queue(&mut self) {
        if self.dock.pending_commands.is_empty() {
            return;
        }

        // Drain commands to avoid borrowing self while iterating
        let commands = drain_pending_commands(&mut self.dock.pending_commands);

        for cmd in commands {
            let behavior = self.perform_editor_command_immediate(cmd);

            if behavior == FocusBehavior::Restore {
                if let Some(mut editor) = self.get_editor() {
                    if should_restore_focus(behavior, editor.has_focus()) {
                        editor.grab_focus();
                    }
                }
            }
        }
    }

    /// Performs the command synchronously.
    /// Called by `flush_command_queue`.
    pub(crate) fn perform_editor_command_immediate(&mut self, cmd: EditorCommand) -> FocusBehavior {
        <Self as ExternalCmdHandler>::handle_external_command(self, cmd)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drain_pending_commands_preserves_fifo_order() {
        let mut pending = vec![
            EditorCommand::Save,
            EditorCommand::Quit,
            EditorCommand::BufferNext,
        ];

        let drained = drain_pending_commands(&mut pending);
        assert_eq!(
            drained,
            vec![
                EditorCommand::Save,
                EditorCommand::Quit,
                EditorCommand::BufferNext,
            ]
        );
        assert!(pending.is_empty());
    }

    #[test]
    fn should_restore_focus_only_when_requested_and_not_focused() {
        assert!(should_restore_focus(FocusBehavior::Restore, false));
        assert!(!should_restore_focus(FocusBehavior::Restore, true));
        assert!(!should_restore_focus(FocusBehavior::Skip, false));
        assert!(!should_restore_focus(FocusBehavior::Skip, true));
    }
}
