//! Scene control handlers for Ex commands.
//!
//! Provides handlers for `:run`, `:stop`, `:save`, etc.

use godot::classes::EditorInterface;
use godot::obj::Singleton;

/// Plays the main scene (F5 equivalent).
pub fn handle_play_main() {
    EditorInterface::singleton().play_main_scene();
}

/// Plays the current scene (F6 equivalent).
pub fn handle_play_current() {
    EditorInterface::singleton().play_current_scene();
}

/// Stops the currently playing scene (F7 equivalent).
pub fn handle_stop() {
    EditorInterface::singleton().stop_playing_scene();
}

/// Saves the current scene.
/// Returns Ok(()) on success, Err with message on failure.
pub fn handle_save() -> Result<(), String> {
    match EditorInterface::singleton().save_scene() {
        godot::global::Error::OK => Ok(()),
        e => Err(format!("Save failed: {:?}", e)),
    }
}

/// Closes the current scene.
/// Returns Ok(()) on success, Err with message on failure.
#[allow(dead_code)]
pub fn handle_close() -> Result<(), String> {
    match EditorInterface::singleton().close_scene() {
        godot::global::Error::OK => Ok(()),
        e => Err(format!("Close failed: {:?}", e)),
    }
}
