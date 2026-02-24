//! Editor state handlers for Ex commands.
//!
//! Provides handlers for `:zen`, `:restart`, etc.

use godot::classes::EditorInterface;
use godot::obj::Singleton;

/// Enables or disables distraction-free (zen) mode.
pub fn handle_zen(enable: bool) {
    EditorInterface::singleton().set_distraction_free_mode(enable);
}

/// Restarts the Godot editor.
/// Saves all work before restarting.
pub fn handle_restart() {
    EditorInterface::singleton()
        .restart_editor_ex()
        .save(true)
        .done();
}
