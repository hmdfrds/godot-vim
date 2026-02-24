use godot::classes::CodeEdit;
use godot::prelude::*;
use vim_core::inputs::{KeyCode, VimKey};

/// Manages completion state, interception, and synchronization.
#[derive(Debug, Default)]
pub struct CompletionManager;

impl CompletionManager {
    /// Checks if completion menu is actively displaying options.
    /// This is the source of truth for whether completion mode is active.
    ///
    /// Uses `get_code_completion_selected_index() >= 0` as an O(1) check
    /// instead of `get_code_completion_options().is_empty()` which is O(n).
    pub fn is_active(&self, editor: &Gd<CodeEdit>) -> bool {
        editor.get_code_completion_selected_index() >= 0
    }

    /// Determines if a key should be intercepted (Exclusive) by the Vim Controller.
    ///
    /// If true, the key bypasses Godot's normal input processing (Passive) and
    /// is handled by Vim (which generates Actions).
    pub fn should_intercept(&self, key: &VimKey, editor: &Gd<CodeEdit>) -> bool {
        if !self.is_active(editor) {
            return false;
        }

        // Only unmodified Enter is intercepted.
        // - Up/Down: Left to Godot (Passive) for list navigation.
        // - Tab: Left to Godot (Passive) to handle complex replace logic.
        // - Modifiers (Ctrl+Enter): Left to Godot for special behaviors (e.g. "Complete with arguments").
        match key.code {
            KeyCode::Enter => key.modifiers.is_empty(),
            _ => false,
        }
    }
}
