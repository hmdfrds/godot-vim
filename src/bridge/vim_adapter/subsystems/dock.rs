//! Dock observation subsystem — dock control tracking and deferred command queue.

use crate::bridge::types::command::EditorCommand;
use godot::classes::Control;
use godot::prelude::*;

/// Dock observation and deferred command queue.
pub struct DockSubsystem {
    /// Observed dock control (for direct signal interception)
    pub observed_dock: Option<Gd<Control>>,
    /// Commands queued for deferred processing in `_process` to avoid re-entrant borrow panics.
    pub pending_commands: Vec<EditorCommand>,
}

impl DockSubsystem {
    /// Creates a new DockSubsystem with no observed dock.
    pub fn new() -> Self {
        Self {
            observed_dock: None,
            pending_commands: Vec::new(),
        }
    }
}
