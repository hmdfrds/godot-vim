//! UI component subsystem — cmdline, cursor overlay, line numbers.

use crate::bridge::components::cmdline::VimCmdLine;
use crate::bridge::components::cursor_visual::VimCursor;
use crate::bridge::components::line_numbers::LineNumberManager;
use godot::prelude::*;

/// UI component references: cmdline, cursor visual, line numbers.
pub struct UiSubsystem {
    /// The command line overlay component.
    pub cmdline: Option<Gd<VimCmdLine>>,
    /// The custom cursor overlay.
    pub cursor_visual: Option<Gd<VimCursor>>,
    /// Line number manager.
    pub line_number_manager: Option<Gd<LineNumberManager>>,
}

impl UiSubsystem {
    /// Creates a new UiSubsystem with no components attached.
    pub fn new() -> Self {
        Self {
            cmdline: None,
            cursor_visual: None,
            line_number_manager: None,
        }
    }
}
