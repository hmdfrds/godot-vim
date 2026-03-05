use crate::bridge::vim_wrapper::VimController;
use vim_core::inputs::commands::action::MacroAction;
use vim_core::inputs::commands::Action;
use vim_core::inputs::{KeyCode, VimKey};

impl VimController {
    /// Handles macro recording stop logic.
    /// Returns `true` if 'q' stopped recording.
    ///
    /// Recording of keys happens in `handle_gui_input` before mapping expansion.
    pub(crate) fn try_handle_macro_recording(&mut self, vim_key: &VimKey) -> bool {
        if self.engine.recording_register().is_none() {
            return false;
        }

        if !matches!(vim_key.code, KeyCode::Char('q')) {
            return false;
        }

        if self.get_editor().is_none() {
            return true;
        }

        let prev_mode = self.engine.mode();
        self.execute_action_with_visuals(Action::Macro(MacroAction::StopRecording), prev_mode);
        true
    }

    /// Commit the Quantum Insert buffer.
    pub(crate) fn commit_quantum_buffer(&mut self) {
        if self.input.quantum_buffer.is_empty() {
            return;
        }

        let text = std::mem::take(&mut self.input.quantum_buffer);

        log::trace!("Committing quantum buffer: '{}'", text);

        // Sync vim-state cursor to editor cursor on slow-path transition.
        if let Some(editor) = self.get_editor() {
            let cursor = Self::cursor_from_editor(&editor);
            self.engine.init_quantum_buffer(cursor);
            self.engine.reset_insert_session(cursor);
        }

        // Group the typing session into one undo step.
        if let Some(mut editor) = self.get_editor() {
            editor.end_complex_operation();
        }
    }

    /// Try to handle key via Quantum Insert fast path.
    ///
    /// Returns `true` if handled.
    /// If `from_user_input` is true, the key is passed through to Godot.
    /// If false (replay), text is inserted manually.
    pub(crate) fn try_handle_quantum_insert(
        &mut self,
        vim_key: &VimKey,
        from_user_input: bool,
    ) -> bool {
        if !self.engine.is_insert() {
            return false;
        }

        if self.engine.is_completion_visible() {
            return false;
        }

        let c = match vim_key.code {
            KeyCode::Char(c) if vim_key.modifiers.is_empty() => c,
            _ => return false,
        };

        if self.input.quantum_buffer.is_empty() {
            if let Some(mut editor) = self.get_editor() {
                editor.begin_complex_operation();
            }
        }

        self.input.quantum_buffer.push(c);
        self.engine.record_insert_char(c);
        self.engine.track_insert_session_char(c);

        if !from_user_input {
            if let Some(mut editor) = self.get_editor() {
                let mut buf = [0u8; 4];
                let text: &str = c.encode_utf8(&mut buf);
                editor.insert_text_at_caret(text);
            }
        }

        true
    }
}
