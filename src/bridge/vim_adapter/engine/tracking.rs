use godot::classes::CodeEdit;
use godot::prelude::Gd;
use vim_core::domain::position::Position;
use vim_core::inputs::{KeyCode, VimKey, VimModifiers};
use vim_core::state::mode::Mode;
use vim_core::state::registry::CommandRegistry;

use crate::bridge::vim_adapter::core::cursor::CursorMoveType;

use super::VimEngine;

impl VimEngine {
    #[inline]
    pub(crate) fn set_cursor(&mut self, line: usize, col: usize) {
        self.state.set_cursor_pos(Position::new(line, col));
    }

    #[inline]
    pub(crate) fn set_mode(&mut self, mode: Mode) {
        self.state.set_mode(mode);
    }

    #[inline]
    #[allow(dead_code)]
    pub(crate) fn take_count(&mut self) -> usize {
        vim_core::runtime::transition::take_count(&mut self.state)
    }

    #[inline]
    pub(crate) fn accumulate_digit(&mut self, digit: char) -> bool {
        vim_core::runtime::transition::accumulate_digit(&mut self.state, digit)
    }

    #[inline]
    #[must_use]
    #[allow(dead_code)]
    pub(crate) fn has_count(&self) -> bool {
        vim_core::runtime::transition::has_count(&self.state)
    }

    #[inline]
    pub(crate) fn push_cmd_history(&mut self, cmd: &str) {
        vim_core::runtime::transition::push_cmd_history(&mut self.state, cmd);
    }

    #[inline]
    pub(crate) fn reset_history_nav(&mut self) {
        vim_core::runtime::transition::reset_history_navigation(&mut self.state);
    }

    pub(crate) fn history_up<'a>(&'a mut self, current_buffer: &str) -> Option<&'a str> {
        vim_core::runtime::transition::history_up(&mut self.state, current_buffer)
    }

    pub(crate) fn history_down(&mut self) -> Option<&str> {
        vim_core::runtime::transition::history_down(&mut self.state)
    }

    #[inline]
    pub(crate) fn sync_cursor(&mut self, pos: Position) {
        self.state.set_cursor_pos(pos);
    }

    #[inline]
    pub(crate) fn set_preferred_column(&mut self, col: usize) {
        self.state.cursor_state.preferred_column = Some(col);
    }

    pub(crate) fn move_cursor_tracked(&mut self, target: Position, move_type: CursorMoveType) {
        let current_pos = self.state.cursor_state.position;
        match move_type {
            CursorMoveType::Jump => {
                self.state.history.jumps.push(current_pos);
                self.state.visual.set_last_jump(current_pos);
            }
            CursorMoveType::JumpRestoration => {
                self.state.visual.set_last_jump(current_pos);
            }
            CursorMoveType::Step => {}
        }
        self.state.set_cursor_pos(target);
    }

    #[inline]
    pub(crate) fn record_jump_at(&mut self, pos: Position) {
        self.state.history.jumps.push(pos);
        self.state.visual.set_last_jump(pos);
    }

    pub(crate) fn record_insert_char(&mut self, c: char) {
        self.state.insert.current_text.push(c);
        self.state.insert.quantum.insert(c);
        self.state.set_cursor_pos(self.state.insert.quantum.cursor());
    }

    #[inline]
    pub(crate) fn record_insert_str(&mut self, s: &str) {
        self.state.insert.current_text.push_str(s);
    }

    pub(crate) fn init_quantum_buffer(&mut self, cursor: Position) {
        self.state.insert.quantum.text.clear();
        self.state.insert.quantum.start = cursor;
        self.state.insert.quantum.deleted_before = 0;
        self.state.set_cursor_pos(cursor);
    }

    pub(crate) fn reset_insert_session(&mut self, cursor: Position) {
        if let Some(session) = self.state.insert.session.as_mut() {
            session.start_cursor = cursor;
        }
    }

    pub(crate) fn track_insert_session_char(&mut self, c: char) {
        if let Some(session) = self.state.insert.session.as_mut() {
            session.insert_char(c);
        }
    }

    #[inline]
    pub(crate) fn record_insert_newline(&mut self) {
        self.state.insert.current_text.push('\n');
    }

    #[inline]
    pub(crate) fn sync_completion_visible(&mut self, visible: bool) {
        self.state.completion_visible = visible;
    }

    pub(crate) fn confirm_completion(&mut self, editor: &mut Gd<CodeEdit>) {
        editor.confirm_code_completion();
        let line = editor.get_caret_line();
        let col = editor.get_caret_column();
        self.state.set_cursor_pos(Position::new(
            crate::bridge::vim_adapter::core::cast::i32_to_usize(line),
            crate::bridge::vim_adapter::core::cast::i32_to_usize(col),
        ));
    }

    #[inline]
    #[allow(dead_code)]
    pub(crate) fn set_completion_visible(&mut self, visible: bool) {
        self.state.completion_visible = visible;
    }

    #[inline]
    pub(crate) fn record_macro_key(&mut self, key: VimKey) {
        self.state.macros.buffer.push(key);
    }

    #[allow(dead_code)]
    pub(crate) fn record_macro_keys_for_cmdline(&mut self, text: &str) {
        for c in text.chars() {
            self.state
                .macros
                .buffer
                .push(VimKey::new(KeyCode::Char(c), VimModifiers::NONE));
        }
        self.state
            .macros
            .buffer
            .push(VimKey::new(KeyCode::Enter, VimModifiers::NONE));
    }

    pub(crate) fn macro_buffer_replace_last_enter(&mut self) {
        if let Some(last) = self.state.macros.buffer.last() {
            if last.code == KeyCode::Enter {
                self.state.macros.buffer.pop();
            }
        }
    }

    pub(crate) fn set_search(&mut self, pattern: String, forward: bool) {
        self.state.search.set_search(pattern, forward);
    }

    #[inline]
    pub(crate) fn set_last_change(&mut self, pos: Position) {
        self.state.visual.set_last_change(pos);
    }

    #[inline]
    pub(crate) fn set_last_insert(&mut self, pos: Position) {
        self.state.visual.set_last_insert(pos);
    }

    #[inline]
    pub(crate) fn set_mark(&mut self, name: char, pos: Position) {
        self.state.history.marks.set(name, pos);
    }

    #[inline]
    pub(crate) fn update_line_snapshot(&mut self, line: usize, text: String) {
        self.state.insert.line_snapshot = Some((line, text));
    }

    pub(crate) fn take_pending_visual_mode(&mut self) -> Option<vim_core::state::mode::Mode> {
        self.state.visual.take_pending_mode()
    }

    #[inline]
    pub(crate) fn registry_mut(&mut self) -> &mut CommandRegistry {
        &mut self.state.registry
    }
}
