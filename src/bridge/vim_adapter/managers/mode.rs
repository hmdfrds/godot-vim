//! ModeManager - Encapsulates mode-based visual styling logic.
//!
//! This module provides pure functions for determining visual styling
//! based on the current Vim mode, keeping the logic separate from
//! Godot-specific application code.
//!
//! ## Separation of Concerns
//! - **Pure Logic**: Mode categorization and color selection
//! - **Application**: Applying colors to Godot components (remains in visuals.rs)

use crate::bridge::settings;
use crate::bridge::types::mode::EditorMode;
use godot::prelude::Color;
use strum::Display;

/// Categories of modes for visual styling purposes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Display)]
pub enum ModeCategory {
    /// Normal mode, CmdLine, and all pending states
    Normal,
    /// Insert mode variants (Insert, BlockInsert, BlockAppend, Replace, VirtualReplace)
    Insert,
    /// Visual mode variants (Visual, VisualLine, VisualBlock)
    Visual,
}

impl ModeCategory {
    /// Categorize an EditorMode for visual styling purposes.
    #[must_use]
    pub fn from_editor_mode(mode: &EditorMode) -> Self {
        match mode {
            EditorMode::Insert | EditorMode::Replace => Self::Insert,
            EditorMode::Visual | EditorMode::VisualLine | EditorMode::VisualBlock => Self::Visual,
            _ => Self::Normal,
        }
    }
}

/// Get the cursor color for a given editor mode.
///
/// Returns `Some(color)` if mode colors are enabled, `None` otherwise
/// (indicating the default theme color should be used).
#[must_use]
pub fn get_mode_cursor_color(mode: &EditorMode) -> Option<Color> {
    if !settings::VimSettings::mode_colors_enabled() {
        return None;
    }

    let color = match ModeCategory::from_editor_mode(mode) {
        ModeCategory::Insert => settings::VimSettings::insert_mode_color(),
        ModeCategory::Visual => settings::VimSettings::visual_mode_color(),
        ModeCategory::Normal => settings::VimSettings::normal_mode_color(),
    };

    Some(color)
}
