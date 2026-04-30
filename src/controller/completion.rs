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
use vim_core::execution::VimEngine;
use vim_core::keymap::{Key, KeyEvent, Modifiers};

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

    // Ctrl+Space → Ctrl+@ after bridge translation: force-trigger completion.
    if in_insert && key.modifiers().contains(Modifiers::CTRL) && key.key() == Key::Char('@') {
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

/// After the engine processes an insert-mode key, re-trigger or dismiss
/// CodeEdit's completion popup to match Godot's native behavior.
///
/// Godot natively calls the private `_filter_code_completion_candidates_impl`
/// after each typed character, which re-filters candidates and cancels the
/// popup when the word prefix is empty. We replicate that cancel logic here:
/// word chars and completion-prefix chars (`.`, etc.) retrigger; everything
/// else (`;`, `)`, space) cancels. Prefix chars come from CodeEdit's
/// `code_completion_prefixes` — a per-language set, not a hardcoded list.
///
/// Gated on `code_complete_enabled` so typing doesn't auto-trigger the popup
/// when the user has disabled auto-completion in EditorSettings.
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

    match key.key() {
        Key::Char(c) if !c.is_control() && key.modifiers() == Modifiers::NONE => {
            if c.is_alphanumeric() || c == '_' || is_completion_prefix(editor, c) {
                editor.request_code_completion_ex().force(false).done();
            } else {
                editor.cancel_code_completion();
            }
        }
        Key::Backspace => {
            editor.request_code_completion_ex().force(false).done();
        }
        _ => {}
    }
}

/// Check if `ch` is in CodeEdit's `code_completion_prefixes` (e.g., `.` for
/// member access). These are per-language trigger characters configured by
/// Godot's script language providers.
fn is_completion_prefix(editor: &Gd<CodeEdit>, ch: char) -> bool {
    let prefixes = editor.get_code_completion_prefixes();
    let mut buf = [0u8; 4];
    let ch_str = ch.encode_utf8(&mut buf);
    prefixes.iter_shared().any(|p| *p.to_string() == *ch_str)
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

    super::reconcile::reconcile_external_text_change(engine, &before_text, &after_text, after_byte);
}
