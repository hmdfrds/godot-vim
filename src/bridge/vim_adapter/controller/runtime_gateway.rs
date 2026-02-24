//! Canonical runtime gateway for VimController.
//!
//! All key/action/Ex execution routes through this module.

use crate::bridge::types::cursor::CursorPos;
use crate::bridge::vim_adapter::contracts::{ExecutionContext, InputPolicy};
use crate::bridge::vim_adapter::core::snapshot::LazyGodotSnapshot;
use crate::bridge::vim_wrapper::VimController;
use vim_core::domain::position::Position;
use vim_core::domain::selection::Selection;
use vim_core::inputs::commands::Action;
use vim_core::inputs::VimKey;
use vim_core::state::mode::{Mode, VisualKind};

impl VimController {
    /// Execute one key through the canonical runtime and visual synchronization path.
    pub(crate) fn execute_key_with_visuals(&mut self, key: &VimKey, policy: InputPolicy) {
        let Some(editor) = self.get_editor() else {
            return;
        };

        let prev_mode = self.engine.mode();

        let (cursor, snapshot) = if let Mode::Visual(VisualKind::Block {
            start,
            cursor: vcursor,
        }) = &self.engine.mode()
        {
            let block_selection = Selection::new(Position::new(start.line, start.col), *vcursor);
            (
                *vcursor,
                LazyGodotSnapshot::with_selection(&editor, block_selection),
            )
        } else {
            (self.engine.cursor_pos(), LazyGodotSnapshot::new(&editor))
        };

        let snap = self.engine.visual_snapshot();
        let cursor_pos = CursorPos::new(cursor.line, cursor.col);
        let context = ExecutionContext::from_snapshot(cursor_pos, &snapshot);
        let Some(output) = self
            .engine
            .process_key_with_policy(key, policy, &snapshot, context)
        else {
            return;
        };

        self.apply_output_with_visuals(prev_mode, &snap, output);
    }

    /// Execute one action through the canonical runtime and visual synchronization path.
    pub(crate) fn execute_action_with_visuals(&mut self, action: Action, prev_mode: Mode) {
        let Some(editor) = self.get_editor() else {
            return;
        };

        let (cursor, snapshot) = if let Mode::Visual(VisualKind::Block {
            start,
            cursor: vcursor,
        }) = &self.engine.mode()
        {
            let block_selection = Selection::new(Position::new(start.line, start.col), *vcursor);
            (
                *vcursor,
                LazyGodotSnapshot::with_selection(&editor, block_selection),
            )
        } else {
            (self.engine.cursor_pos(), LazyGodotSnapshot::new(&editor))
        };

        let snap = self.engine.visual_snapshot();
        let cursor_pos = CursorPos::new(cursor.line, cursor.col);
        let context = ExecutionContext::from_snapshot(cursor_pos, &snapshot);
        let output = self
            .engine
            .process_action_with_context(action, &snapshot, context);

        self.apply_output_with_visuals(prev_mode, &snap, output);
    }

    /// Execute one Ex command through the canonical runtime and visual synchronization path.
    pub(crate) fn execute_ex_command_with_visuals(&mut self, command: &str, prev_mode: Mode) {
        let Some(editor) = self.get_editor() else {
            return;
        };

        let (cursor, snapshot) = if let Mode::Visual(VisualKind::Block {
            start,
            cursor: vcursor,
        }) = &self.engine.mode()
        {
            let block_selection = Selection::new(Position::new(start.line, start.col), *vcursor);
            (
                *vcursor,
                LazyGodotSnapshot::with_selection(&editor, block_selection),
            )
        } else {
            (self.engine.cursor_pos(), LazyGodotSnapshot::new(&editor))
        };

        let snap = self.engine.visual_snapshot();
        let cursor_pos = CursorPos::new(cursor.line, cursor.col);
        let context = ExecutionContext::from_snapshot(cursor_pos, &snapshot);
        let output = self
            .engine
            .process_ex_command_with_context(command, &snapshot, context);

        self.apply_output_with_visuals(prev_mode, &snap, output);
    }
}
