//! Multi-cursor keybindings (Godot-native compatible).
//!
//! Intercepted before process_key:
//! - Ctrl+D: add next match
//! - Ctrl+Shift+Up/Down: add cursor above/below (matches Godot's native shortcut)
//! - Ctrl+Shift+L: select all occurrences
//! - Alt+Click: add cursor at mouse position (Godot-native, handled by import sync)
//! - Escape (when multi-cursor active): clear secondary cursors

use godot::classes::InputEventKey;
use godot::global::Key;

/// Actions triggered by multi-cursor keyboard shortcuts.
///
/// Each variant maps to a specific vim-core `MultiCursorCommand` that the
/// caller will execute after this detection layer returns.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MultiCursorAction {
    /// Ctrl+D — add next occurrence of word/selection.
    AddNextMatch,
    /// Ctrl+Shift+Up — add a cursor on the line above.
    AddCursorAbove,
    /// Ctrl+Shift+Down — add a cursor on the line below.
    AddCursorBelow,
    /// Ctrl+Shift+L — select all occurrences of word/selection.
    SelectAllOccurrences,
    /// Escape when multi-cursor active — clear all secondary cursors.
    /// Not detected by `is_multi_cursor_shortcut` — handled by the caller
    /// which checks `cursor_count > 1` before deciding Escape behavior.
    #[allow(dead_code)]
    ClearSecondary,
}

/// Detect whether a key event matches a VS Code-style multi-cursor shortcut.
///
/// Returns `Some(action)` if the key was recognized as a multi-cursor command.
/// The caller should execute the corresponding action and NOT pass the key
/// through to the vim engine.
///
/// Note: Escape/ClearSecondary is handled by the caller separately (it must
/// check `cursor_count > 1` before deciding whether Escape clears cursors
/// or passes through to the vim engine for its normal behavior).
pub(crate) fn is_multi_cursor_shortcut(key_event: &InputEventKey) -> Option<MultiCursorAction> {
    let keycode = key_event.get_keycode();
    let ctrl = key_event.is_ctrl_pressed();
    let alt = key_event.is_alt_pressed();
    let shift = key_event.is_shift_pressed();
    let meta = key_event.is_meta_pressed();

    // Ignore meta combinations — those are OS-level shortcuts.
    if meta {
        return None;
    }

    match keycode {
        // Ctrl+D: add next match
        Key::D if ctrl && !alt && !shift => Some(MultiCursorAction::AddNextMatch),

        // Ctrl+Shift+Up: add cursor above (matches Godot's ui_text_caret_add_above)
        Key::UP if ctrl && !alt && shift => Some(MultiCursorAction::AddCursorAbove),

        // Ctrl+Shift+Down: add cursor below (matches Godot's ui_text_caret_add_below)
        Key::DOWN if ctrl && !alt && shift => Some(MultiCursorAction::AddCursorBelow),

        // Ctrl+Shift+L: select all occurrences
        Key::L if ctrl && !alt && shift => Some(MultiCursorAction::SelectAllOccurrences),

        _ => None,
    }
}

// NOTE: Unit tests for `is_multi_cursor_shortcut` require a running Godot
// engine to instantiate `InputEventKey` objects. These are covered by the
// integration test suite (GUT/Godot test runner), not `cargo test`.
