//! Completion-aware key routing for CodeEdit's autocomplete popup.
//!
//! Godot's CodeEdit autocomplete is driven by `_gui_input()`, which never
//! fires when Vim consumes the key via `set_input_as_handled()`. This module
//! intercepts completion-relevant keys *before* the engine so the popup can
//! trigger, filter, navigate, and confirm — all without engine changes.
//!
//! The interception has two phases:
//! - **Pre-engine** ([`try_handle_completion`]): captures Ctrl+N/P, Tab,
//!   Enter, Escape, arrow keys while the popup is visible.
//! - **Post-engine** ([`maybe_retrigger_completion`]): re-triggers the popup
//!   after printable/backspace keystrokes so filtering stays in sync.

use godot::classes::CodeEdit;
use godot::prelude::*;
use vim_core::execution::{ExternalEdit, ExternalEditKind, VimEngine};
use vim_core::keymap::{Key, KeyEvent, Modifiers};
use vim_core::primitives::{Offset, Range};

use crate::bridge;
use crate::bridge::codec::usize_to_i32;

/// Godot returns -1 when no completion popup is visible.
fn is_completion_active(editor: &Gd<CodeEdit>) -> bool {
    editor.get_code_completion_selected_index() >= 0
}

/// Pre-engine interception for completion and trigger keys.
///
/// Returns `Some(consumed)` if the key was handled here (skip engine).
/// Returns `None` if the engine should process the key normally.
pub(crate) fn try_handle_completion(
    engine: &mut VimEngine,
    key: KeyEvent,
    editor: &mut Gd<CodeEdit>,
) -> Option<bool> {
    let mode = engine.mode();
    let in_insert = mode.is_insert() || mode.is_replace();

    // Ctrl+Space: force-trigger completion regardless of popup state.
    if in_insert
        && key.modifiers().contains(Modifiers::CTRL)
        && key.key() == Key::Char(' ')
    {
        if !editor.is_code_completion_enabled() {
            return None;
        }
        editor.request_code_completion_ex().force(true).done();
        return Some(true);
    }

    // Ctrl+N / Ctrl+P: trigger popup when not yet visible.
    // Godot's request_code_completion is synchronous — popup state
    // and selected index are available immediately after the call.
    if in_insert && key.modifiers().contains(Modifiers::CTRL) {
        match key.key() {
            Key::Char('n') if !is_completion_active(editor) => {
                if !editor.is_code_completion_enabled() {
                    return None;
                }
                // Godot auto-selects index 0 — matches Vim's Ctrl+N (forward).
                editor.request_code_completion_ex().force(true).done();
                return Some(true);
            }
            Key::Char('p') if !is_completion_active(editor) => {
                if !editor.is_code_completion_enabled() {
                    return None;
                }
                editor.request_code_completion_ex().force(true).done();
                // Vim's Ctrl+P selects the *last* item (backward search).
                if is_completion_active(editor) {
                    let count = usize_to_i32(editor.get_code_completion_options().len());
                    if count > 0 {
                        editor.set_code_completion_selected_index(count - 1);
                    }
                }
                return Some(true);
            }
            _ => {}
        }
    }

    // Remaining interceptions only apply with a visible popup in insert/replace.
    if !in_insert || !is_completion_active(editor) {
        return None;
    }

    match key.key() {
        // Up/Down: return `Some(false)` = "handled by us, but don't mark consumed"
        // so Godot's CodeEdit processes the arrow key to navigate the list.
        Key::Up | Key::Down => Some(false),

        // Tab / Enter: confirm and reconcile the text delta with the engine
        // so dot-repeat and macro recording capture the completed text.
        Key::Tab | Key::Enter => {
            confirm_and_reconcile_completion(engine, editor);
            Some(true)
        }

        // Escape: dismiss popup, then fall through to engine (which exits insert mode).
        Key::Escape => {
            editor.cancel_code_completion();
            None
        }

        Key::Char('n') if key.modifiers().contains(Modifiers::CTRL) => {
            let current = editor.get_code_completion_selected_index();
            let count = usize_to_i32(editor.get_code_completion_options().len());
            if count > 0 {
                let next = if current + 1 >= count { 0 } else { current + 1 };
                editor.set_code_completion_selected_index(next);
            }
            Some(true)
        }

        Key::Char('p') if key.modifiers().contains(Modifiers::CTRL) => {
            let current = editor.get_code_completion_selected_index();
            let count = usize_to_i32(editor.get_code_completion_options().len());
            if count > 0 {
                let prev = if current <= 0 { count - 1 } else { current - 1 };
                editor.set_code_completion_selected_index(prev);
            }
            Some(true)
        }

        _ => None,
    }
}

/// After the engine processes an insert-mode key, re-trigger CodeEdit's
/// completion heuristic so the popup can appear, filter, or dismiss.
///
/// Gated on `code_complete_enabled` (the user's EditorSetting for auto-trigger).
/// When the user disables auto-completion, typing should not pop up the
/// completion menu; only explicit triggers (Ctrl+Space, Ctrl+N, Ctrl+P) should.
pub(crate) fn maybe_retrigger_completion(
    engine: &VimEngine,
    key: KeyEvent,
    editor: &mut Gd<CodeEdit>,
    code_complete_enabled: bool,
) {
    if !code_complete_enabled {
        return;
    }

    let mode = engine.mode();
    if !mode.is_insert() && !mode.is_replace() {
        return;
    }

    let should_retrigger = match key.key() {
        // Only printable chars and backspace change the prefix that Godot
        // uses to filter the completion list.
        Key::Char(c) if !c.is_control() && key.modifiers() == Modifiers::NONE => true,
        Key::Backspace => true,
        _ => false,
    };

    if should_retrigger {
        editor.request_code_completion_ex().force(false).done();
    }
}

/// Confirm the selected completion and reconcile the text delta with the
/// engine so dot-repeat and macro recording capture the completed text.
///
/// Strategy: snapshot text before/after Godot's confirm, compute a minimal
/// contiguous diff (common-prefix / common-suffix), and feed it to the
/// engine as an `ExternalEdit`. The engine records the net-new text
/// internally for dot-repeat.
fn confirm_and_reconcile_completion(engine: &mut VimEngine, editor: &mut Gd<CodeEdit>) {
    let before_text = editor.get_text().to_string();
    let before_index = bridge::codec::LineIndex::new(&before_text);
    let before_byte = before_index.line_col_to_byte(
        &before_text,
        editor.get_caret_line(),
        editor.get_caret_column(),
    );

    // CodeEdit replaces `code_completion_base` (the typed prefix) with the
    // selected item's `insert_text`. This is the only mutation.
    editor.confirm_code_completion_ex().replace(false).done();

    let after_text = editor.get_text().to_string();
    let after_index = bridge::codec::LineIndex::new(&after_text);
    let after_byte = after_index.line_col_to_byte(
        &after_text,
        editor.get_caret_line(),
        editor.get_caret_column(),
    );

    // Completion edits are always a contiguous replacement at the caret:
    // the typed prefix was deleted and the full completion inserted.
    //
    // Find the common prefix (bytes before the edit region), snapping to
    // a char boundary to avoid slicing mid-UTF-8.
    let raw_prefix = before_text
        .bytes()
        .zip(after_text.bytes())
        .take_while(|(a, b)| a == b)
        .count()
        .min(before_byte);
    let common_prefix = snap_to_char_boundary_down(&before_text, raw_prefix);

    if before_text == after_text {
        log::trace!("completion_reconcile: no text change, skipping");
        return;
    }

    // Common suffix (bytes after the edit region). Clamped so that
    // prefix + suffix never exceeds the shorter text — otherwise
    // overlap produces inverted ranges.
    let max_suffix = before_text.len().saturating_sub(before_byte)
        .min(after_text.len().saturating_sub(common_prefix));
    let raw_suffix = before_text
        .bytes()
        .rev()
        .zip(after_text.bytes().rev())
        .take_while(|(a, b)| a == b)
        .count()
        .min(max_suffix);
    let common_suffix = before_text.len()
        - snap_to_char_boundary_up(&before_text, before_text.len() - raw_suffix);

    let deleted_end = before_text.len().saturating_sub(common_suffix);
    let inserted_end = after_text.len().saturating_sub(common_suffix);

    if common_prefix > deleted_end || common_prefix > inserted_end {
        return;
    }

    let deleted_range = Range::new(
        Offset::new(common_prefix),
        Offset::new(deleted_end),
    );
    let deleted_text = &before_text[common_prefix..deleted_end];
    let inserted_text = &after_text[common_prefix..inserted_end];

    log::debug!(
        "completion_reconcile: deleted={}b inserted={}b",
        deleted_end - common_prefix, inserted_end - common_prefix
    );

    let edit = ExternalEdit::new(
        deleted_range,
        inserted_text,
        Offset::new(after_byte),
        ExternalEditKind::PasteOrIme,
    );
    // Response discarded: CodeEdit already applied the text change.
    // Processing effects here would double-apply them.
    let _response = engine.apply_external_edit_with_recording(edit, deleted_text);
}

/// Snap a byte offset down to the nearest char boundary in `s`.
fn snap_to_char_boundary_down(s: &str, offset: usize) -> usize {
    let mut pos = offset.min(s.len());
    while pos > 0 && !s.is_char_boundary(pos) {
        pos -= 1;
    }
    pos
}

/// Snap a byte offset up to the nearest char boundary in `s`.
fn snap_to_char_boundary_up(s: &str, offset: usize) -> usize {
    let mut pos = offset.min(s.len());
    while pos < s.len() && !s.is_char_boundary(pos) {
        pos += 1;
    }
    pos
}
