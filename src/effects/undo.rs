//! Undo/redo effect handlers and [`UndoDepth`] tracking for Godot's
//! `begin_complex_operation` / `end_complex_operation` nesting.

use crate::bridge::port::TextEditorPort;

/// Depth ceiling before suppressing Godot calls. Typical depth is 1-3;
/// exceeding 64 indicates an engine bug, not normal operation.
const MAX_UNDO_DEPTH: u32 = 64;

/// Tracked nesting depth for Godot's `begin/end_complex_operation`.
///
/// Enforces a ceiling invariant: depths beyond `MAX_UNDO_DEPTH` are counted
/// but not forwarded to Godot, keeping the Godot undo stack balanced even
/// under runaway nesting from engine bugs.
#[derive(Debug, Default)]
pub(crate) struct UndoDepth(u32);

impl UndoDepth {
    pub(crate) const fn new() -> Self {
        Self(0)
    }

    #[must_use]
    pub(crate) fn is_zero(&self) -> bool {
        self.0 == 0
    }

    #[must_use]
    pub(crate) fn depth(&self) -> u32 {
        self.0
    }

    /// Drain all depth, returning the count of Godot-side `end_complex_operation`
    /// calls needed. Depths above `MAX_UNDO_DEPTH` were never sent to Godot.
    pub(crate) fn drain(&mut self) -> u32 {
        let godot_groups = self.0.min(MAX_UNDO_DEPTH);
        self.0 = 0;
        godot_groups
    }
}

/// Begin an undo group. Depths beyond `MAX_UNDO_DEPTH` are tracked but not
/// forwarded to Godot; the matching `handle_end_undo_group` suppresses the
/// corresponding end call, keeping the Godot stack balanced.
pub(crate) fn handle_begin_undo_group(
    editor: &mut impl TextEditorPort,
    undo_depth: &mut UndoDepth,
) {
    undo_depth.0 += 1;
    if undo_depth.0 <= MAX_UNDO_DEPTH {
        editor.begin_complex_operation();
    } else {
        // Controller's ensure_undo_balanced will surface orphaned groups.
        log::error!(
            "BeginUndoGroup exceeded depth ceiling ({MAX_UNDO_DEPTH}), \
             suppressing Godot call (depth={})",
            undo_depth.0
        );
    }
}

/// End the innermost undo group. Above-ceiling depths match suppressed
/// begins — the counter decrements but Godot's `end_complex_operation`
/// is NOT called.
pub(crate) fn handle_end_undo_group(editor: &mut impl TextEditorPort, undo_depth: &mut UndoDepth) {
    if undo_depth.0 > MAX_UNDO_DEPTH {
        undo_depth.0 -= 1;
    } else if undo_depth.0 > 0 {
        editor.end_complex_operation();
        undo_depth.0 -= 1;
    } else {
        log::warn!("EndUndoGroup without matching BeginUndoGroup (depth already 0)");
    }
}

pub(crate) fn handle_undo(editor: &mut impl TextEditorPort, count: u32) {
    log::debug!("undo: count={}", count);
    for _ in 0..count {
        editor.undo();
    }
    // Godot's undo restores the caret snapshot from begin_complex_operation,
    // which may include secondary carets and selections the engine already cleared.
    editor.remove_secondary_carets();
    editor.deselect();
}

pub(crate) fn handle_redo(editor: &mut impl TextEditorPort, count: u32) {
    log::debug!("redo: count={}", count);
    for _ in 0..count {
        editor.redo();
    }
    editor.remove_secondary_carets();
    editor.deselect();
}

/// `U` (per-line undo) — not supported by Godot's CodeEdit.
pub(super) fn handle_undo_line(_count: u32) {
    use std::sync::atomic::{AtomicBool, Ordering};
    static LOGGED: AtomicBool = AtomicBool::new(false);
    if !LOGGED.swap(true, Ordering::Relaxed) {
        log::info!("U (undo line) not supported — CodeEdit provides only global undo");
    }
}
