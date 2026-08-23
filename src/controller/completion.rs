//! Completion-aware key routing for CodeEdit's autocomplete popup.
//!
//! Godot's CodeEdit autocomplete is driven by `_gui_input()`, which never
//! fires when Vim consumes the key via `set_input_as_handled()`. This module
//! dispatches completion-relevant keys *before* the engine so the popup can
//! trigger, filter, navigate, and confirm, all without engine changes.
//!
//! Two phases:
//! - **Pre-engine** ([`dispatch_overlay`]): runs whatever the
//!   `editor.completion` overlay resolved for this keystroke, folded by the
//!   SAME `resolve::dispose` every other surface uses, so flags, params and
//!   the decline-and-fall-through rule mean the same thing here.
//! - **Post-engine** ([`maybe_retrigger_completion`]): re-triggers the popup
//!   after printable/backspace keystrokes so filtering stays in sync.
//!
//! # One pipeline, and still on this transport
//!
//! The key table is `panelmap` lines in `actions::providers::completion`,
//! resolved by `GodotVimCore::handle_gui_input_impl` against the overlay path
//! `Forest::overlay` builds, and handed down as an owned
//! [`crate::actions::resolve::OverlayPlan`].
//!
//! What did **not** move is the transport. These keys stay on `gui_input`, for
//! three reasons that were each load-bearing and none of which the overlay
//! changes: `_input` is registered per viewport and never fires for a
//! floated script editor; `_input` runs outside the IME guard above this call,
//! so a CJK preedit would lose `<CR>`; and the third routing state, "engine
//! skipped, event deliberately NOT consumed", is a `gui_input` fact.
//! It is now `Disposition::Handoff`, declared by the verb through
//! `Consumption::Handoff` and folded by `dispose`, rather than a flag on a
//! lent port folded by a second, transport-private verdict.

use godot::classes::CodeEdit;
use godot::prelude::*;
use vim_core::execution::{VimEngine, VimSession};
use vim_core::keymap::{Key, KeyEvent, Modifiers};

use crate::actions::action::{ActionCtx, CompletionOps};
use crate::actions::outcome::Outcome;
use crate::actions::resolve::{self, CandidateTarget, Disposition};
use crate::bridge;
use crate::bridge::codec::usize_to_i32;
use crate::bridge::godot_host::GodotHost;

/// Godot returns -1 when no completion popup is visible.
fn is_completion_active(editor: &Gd<CodeEdit>) -> bool {
    editor.get_code_completion_selected_index() >= 0
}

/// The one real [`CompletionOps`], holding the two things no test can build.
///
/// Everything a completion verb decides is decided against this trait; the
/// verbs themselves live in `actions::providers::completion` and are tested
/// against a plain-data fake. That split is the only reason the `Some(true)` /
/// `Some(false)` / `None` trichotomy has a headless characterization suite at
/// all — `Gd<CodeEdit>` and `VimSession<GodotHost>` cannot be constructed under
/// `cargo test` in a `cdylib`.
struct CompletionPort<'a> {
    session: &'a mut VimSession<GodotHost>,
    editor: &'a mut Gd<CodeEdit>,
}

impl CompletionOps for CompletionPort<'_> {
    fn popup_visible(&self) -> bool {
        is_completion_active(self.editor)
    }

    fn completion_enabled(&self) -> bool {
        self.editor.is_code_completion_enabled()
    }

    fn option_count(&self) -> i32 {
        usize_to_i32(self.editor.get_code_completion_options().len())
    }

    fn selected_index(&self) -> i32 {
        self.editor.get_code_completion_selected_index()
    }

    fn request(&mut self, force: bool) {
        self.editor.request_code_completion_ex().force(force).done();
    }

    fn select(&mut self, index: i32) {
        self.editor.set_code_completion_selected_index(index);
    }

    fn confirm(&mut self) {
        confirm_and_reconcile_completion(self.session, self.editor);
    }

    fn cancel(&mut self) {
        self.editor.cancel_code_completion();
    }
}

/// Pre-engine dispatch: run the overlay's plan through the one consumption
/// fold.
///
/// The plan is resolved by the transport against the overlay path
/// `Forest::overlay` builds, so the mode gate the old interception asked
/// imperatively lives in the overlay's `active` predicate now: an empty plan
/// is the not-insert-like case and the not-bound case alike, and the caller
/// skips the call entirely.
pub(crate) fn dispatch_overlay(
    session: &mut VimSession<GodotHost>,
    editor: &mut Gd<CodeEdit>,
    plan: &crate::actions::resolve::OverlayPlan,
) -> Disposition {
    let mut port = CompletionPort { session, editor };
    resolve::dispose(&plan.candidates, plan.is_echo, |candidate| {
        let CandidateTarget::Action(_, spec) = &candidate.target else {
            // Unreachable from config: `<Shortcut>` is refused at
            // registration on every surface (`bind.rs`). Kept as the seam,
            // and it declines rather than consuming.
            log::warn!("completion: <Shortcut> targets are not dispatched yet");
            return Outcome::Declined;
        };
        // `target: None` is CONSISTENT with the overlay's grants-only caps
        // rather than a lie the gate believes, and it is byte-parity with the
        // `ActionCtx::new(None, ...)` this replaces. `candidate.params` is
        // the parameter plumbing the deleted `Params::new()` dropped.
        let mut cx = ActionCtx::new(None, candidate.params.clone()).with_completion(&mut port);
        let outcome = (spec.run)(&mut cx);
        log::trace!("completion: {} -> {outcome:?}", spec.id);
        outcome
    })
}

/// After the engine processes an insert-mode key, re-trigger or dismiss
/// CodeEdit's completion popup to match Godot's native behavior.
///
/// Godot natively calls the private `_filter_code_completion_candidates_impl`
/// after each typed character, which re-filters candidates and cancels the
/// popup when the word prefix is empty. We replicate that cancel logic here:
/// word chars and completion-prefix chars (`.`, etc.) retrigger; everything
/// else (`;`, `)`, space) cancels. Prefix chars come from CodeEdit's
/// `code_completion_prefixes`, which for the script editor is the hardcoded
/// per-editor set `{".", ",", "(", "=", "$", "@", quote, apostrophe}` written
/// by `CodeTextEditor` (godot editor/gui/code_editor.cpp), not a per-language
/// list.
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
/// member access). A per-editor set: the script editor's is hardcoded by
/// `CodeTextEditor`, not configured per language.
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
fn confirm_and_reconcile_completion(
    session: &mut VimSession<GodotHost>,
    editor: &mut Gd<CodeEdit>,
) {
    let before_text = editor.get_text().to_string();

    // CodeEdit replaces `code_completion_base` (the typed prefix) with the
    // selected item's `insert_text`. This is the only mutation.
    editor.confirm_code_completion_ex().replace(false).done();

    // Fix 4B: Invalidate cache IMMEDIATELY after confirm so that
    // host.text() reflects post-completion state for undo node sync.
    session.host_mut().invalidate_cache();

    let after_text = editor.get_text().to_string();
    let after_index = bridge::codec::LineIndex::new(&after_text);
    let after_byte = after_index.line_col_to_byte(
        &after_text,
        editor.get_caret_line(),
        editor.get_caret_column(),
    );

    super::reconcile::reconcile_external_text_change(
        session.engine_mut(),
        &before_text,
        &after_text,
        after_byte,
        vim_core::execution::ExternalEditKind::Completion,
    );

    // Fix 4A: Sync undo nodes so pressing `u` past a completion doesn't
    // silently skip it. The engine created an undo node during
    // reconciliation; we must create a matching UndoStore snapshot.
    super::sync_undo_nodes_after_external_edit(session, &before_text);
}

// The consumption fold that used to be tested here as `verdict` lives in
// `resolve::dispose` now, pinned by the truth-table rows in
// `actions::resolve` and by the `Disposition` -> `PipelineOutcome` table in
// `controller::pipeline_outcome`.
