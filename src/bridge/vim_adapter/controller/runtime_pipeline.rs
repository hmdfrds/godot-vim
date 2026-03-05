//! Unified key execution pipeline for controller -> engine runtime.

use crate::bridge::types::command::EditorCommand;
use crate::bridge::types::cursor::CursorPos;
use crate::bridge::vim_adapter::contracts::{ExecutionContext, InputPolicy};
use crate::bridge::vim_adapter::core::column_codec;
use crate::bridge::vim_adapter::core::snapshot::LazyGodotSnapshot;
use crate::bridge::vim_wrapper::VimController;
use vim_core::domain::position::Position;
use vim_core::domain::selection::Selection;
use vim_core::inputs::{KeyCode, VimKey, VimModifiers};
use vim_core::state::mode::{Mode, ReplaceMode, VisualKind};

/// Key execution outcome summary.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PipelineOutcome {
    pub executed: bool,
    pub consumed: bool,
    pub had_transaction: bool,
}

impl VimController {
    /// Canonical key runtime pipeline used by both Exclusive and Passive input policies.
    pub(crate) fn run_key_pipeline(
        &mut self,
        vim_key: &VimKey,
        policy: InputPolicy,
    ) -> PipelineOutcome {
        let Some(editor) = self.get_editor() else {
            return PipelineOutcome::default();
        };

        let prev_mode = self.engine.mode();
        let (cursor, snapshot) = match self.engine.mode() {
            Mode::Visual(VisualKind::Block { start, cursor }) => {
                let block_selection = Selection::new(
                    Position::from_byte(start.line, start.col.as_usize()),
                    cursor,
                );
                (
                    cursor,
                    LazyGodotSnapshot::with_selection(&editor, block_selection),
                )
            }
            _ => {
                let cursor = if policy == InputPolicy::Passive {
                    column_codec::read_caret_core_position(&editor)
                } else {
                    self.engine.cursor_pos()
                };
                (cursor, LazyGodotSnapshot::new(&editor))
            }
        };

        let snap = self.engine.visual_snapshot();
        let cursor_pos = CursorPos::new(cursor.line, cursor.col.as_usize());
        let context = ExecutionContext::from_snapshot(cursor_pos, &snapshot);
        let Some(mut output) = self
            .engine
            .process_key_with_policy(vim_key, policy, &snapshot, context)
        else {
            return PipelineOutcome::default();
        };

        let had_transaction = output.has_transaction();
        if policy == InputPolicy::Passive {
            let is_backspace_key = matches!(vim_key.code, KeyCode::Backspace)
                || (matches!(vim_key.code, KeyCode::Char('h'))
                    && vim_key.modifiers.contains(VimModifiers::CTRL));
            let is_replace_backspace = is_backspace_key
                && matches!(
                    self.engine.mode(),
                    Mode::Replace(ReplaceMode::Overwrite) | Mode::Replace(ReplaceMode::Virtual)
                );

            if is_replace_backspace {
                self.set_input_handled();
            }

            // Passive mode: Godot keeps rendering authority for command side effects.
            for cmd in &output.commands {
                match cmd {
                    EditorCommand::TypeChar(_) | EditorCommand::Backspace => {}
                    _ => {
                        log::debug!("Passive mode ignored command: {cmd:?}");
                    }
                }
            }
            output.commands.clear();
            output.pending_keys.clear();

            self.apply_output_with_visuals(prev_mode, &snap, output);
            if is_replace_backspace || had_transaction {
                self.set_input_handled();
            }
            return PipelineOutcome {
                executed: true,
                consumed: is_replace_backspace || had_transaction,
                had_transaction,
            };
        }

        self.apply_output_with_visuals(prev_mode, &snap, output);
        PipelineOutcome {
            executed: true,
            consumed: true,
            had_transaction,
        }
    }
}
