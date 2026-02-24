use crate::bridge::godot::names::callbacks;
use crate::bridge::types::command::EditorCommand;
use crate::bridge::vim_wrapper::VimController;
use godot::obj::WithBaseField;
use godot::prelude::*;

impl VimController {
    /// Handles buffer navigation and custom commands via deferred call.
    pub(super) fn handle_deferred_command(&mut self, cmd: EditorCommand) {
        let (cmd_str, args_vec) = match &cmd {
            EditorCommand::BufferNext => ("bnext".to_string(), vec![]),
            EditorCommand::BufferPrev => ("bprev".to_string(), vec![]),
            EditorCommand::BufferGoto(idx) => (format!("b{}", idx), vec![]),
            EditorCommand::Custom { cmd, args } => (cmd.clone(), args.clone()),
            _ => unreachable!(),
        };

        let mut args_packed = PackedStringArray::new();
        for arg in &args_vec {
            args_packed.push(arg);
        }

        self.base_mut().call_deferred(
            callbacks::EXECUTE_COMMAND_DEFERRED,
            &[
                GString::from(&cmd_str).to_variant(),
                args_packed.to_variant(),
            ],
        );
    }
}
