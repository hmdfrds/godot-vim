//! Completion-aware key routing for CodeEdit's autocomplete popup.
//!
//! Godot's CodeEdit autocomplete is driven by `_gui_input()`, which never
//! fires when Vim consumes the key via `set_input_as_handled()`. This module
//! dispatches completion-relevant keys *before* the engine so the popup can
//! trigger, navigate, and confirm, all without engine changes:
//! [`dispatch_overlay`] runs whatever the `editor.completion` overlay
//! resolved for this keystroke, folded by the SAME `resolve::dispose` every
//! other surface uses. Keeping the popup in step with engine edits is
//! `effects::completion`, at the end of every engine pass.
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
use vim_core::execution::VimSession;

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

/// Whether the current completion selection was chosen by the user.
///
/// Rules, each citing Godot: no popup resets; a moved caret resets, because
/// typing moves the caret and steering does not, which makes the caret the
/// episode key; an index EDGE to a NON-ZERO value adopts, because Godot's
/// only automatic writes are resets to 0 (`code_edit.cpp:3873, 4097`) while
/// every non-zero write is human-originated; `last_index == None` leaves
/// `explicit` standing, which is what lets the port's own write survive the
/// keystroke that opened the popup.
///
/// A navigation key is a choice even when the index cannot move (one row, or
/// a wrap to row 0), so the port marks it through [`Provenance::chosen`].
///
/// One documented residual, failing toward a newline rather than an unwanted
/// insert: clicking an already-selected row.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct Provenance {
    last_index: Option<i32>,
    last_caret: (i32, i32),
    pub(crate) explicit: bool,
}

impl Provenance {
    /// A user-initiated port write or navigation key.
    pub(crate) fn chosen(self) -> Self {
        Self {
            explicit: true,
            ..self
        }
    }
}

/// Pure. The Godot reads happen once in `handle_gui_input_impl`; this is the
/// rule, and it is a `fn` over plain data so it is table-tested headlessly.
pub(crate) fn advance(
    prev: Provenance,
    facts: &crate::actions::surface::OverlayFacts,
) -> Provenance {
    if !facts.popup() {
        return Provenance::default();
    }
    if prev.last_index.is_some() && facts.caret != prev.last_caret {
        return Provenance {
            last_index: Some(facts.selected_index),
            last_caret: facts.caret,
            explicit: false,
        };
    }
    let adopted = matches!(
        prev.last_index,
        Some(previous) if previous != facts.selected_index && facts.selected_index != 0
    );
    Provenance {
        last_index: Some(facts.selected_index),
        last_caret: facts.caret,
        explicit: prev.explicit || adopted,
    }
}

/// The one real [`CompletionOps`], holding the two things no test can build.
///
/// Everything a completion verb decides is decided against this trait; the
/// verbs themselves live in `actions::providers::completion` and are tested
/// against a plain-data fake. That split is the only reason the `Some(true)` /
/// `Some(false)` / `None` trichotomy has a headless characterization suite at
/// all — `Gd<CodeEdit>` and `VimSession<GodotHost>` cannot be constructed under
/// `cargo test` in a `cdylib`.
///
/// INVARIANT the provenance write-through depends on: `effects::completion`
/// cancels and re-requests on the raw editor and never touches this port, so
/// a port `request`, `select` or `navigated` is reachable only from a
/// user-initiated verb, and marking the selection explicit there is sound.
struct CompletionPort<'a> {
    session: &'a mut VimSession<GodotHost>,
    editor: &'a mut Gd<CodeEdit>,
    provenance: &'a mut Provenance,
}

impl CompletionOps for CompletionPort<'_> {
    fn popup_visible(&self) -> bool {
        is_completion_active(self.editor)
    }

    fn option_count(&self) -> i32 {
        usize_to_i32(self.editor.get_code_completion_options().len())
    }

    fn selected_index(&self) -> i32 {
        self.editor.get_code_completion_selected_index()
    }

    fn request(&mut self, force: bool) {
        // User-initiated by the port invariant above, so the selection Godot
        // preselects on this request was asked for by name.
        *self.provenance = self.provenance.chosen();
        self.editor.request_code_completion_ex().force(force).done();
    }

    fn select(&mut self, index: i32) {
        *self.provenance = self.provenance.chosen();
        self.editor.set_code_completion_selected_index(index);
    }

    fn navigated(&mut self) {
        *self.provenance = self.provenance.chosen();
    }

    fn confirm(&mut self) {
        confirm_and_reconcile_completion(self.session, self.editor);
    }

    fn cancel(&mut self) {
        self.editor.cancel_code_completion();
    }

    fn selection_is_explicit(&self) -> bool {
        self.provenance.explicit
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
    provenance: &mut Provenance,
    plan: &crate::actions::resolve::OverlayPlan,
) -> Disposition {
    let mut port = CompletionPort {
        session,
        editor,
        provenance,
    };
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::actions::surface::OverlayFacts;

    fn facts(selected_index: i32, caret: (i32, i32)) -> OverlayFacts {
        OverlayFacts {
            at_attached_editor: true,
            mode: Some(vim_core::primitives::Mode::Insert),
            selected_index,
            caret,
        }
    }

    fn prov(last_index: Option<i32>, last_caret: (i32, i32), explicit: bool) -> Provenance {
        Provenance {
            last_index,
            last_caret,
            explicit,
        }
    }

    #[test]
    fn advance_is_a_total_table() {
        // One row per rule, each citing the fact it turns on.
        let rows: &[(&str, Provenance, OverlayFacts, Provenance)] = &[
            (
                "popup absent resets everything",
                prov(Some(3), (1, 4), true),
                facts(-1, (1, 4)),
                Provenance::default(),
            ),
            (
                "a moved caret resets: typing moves the caret, steering does not",
                prov(Some(3), (1, 4), true),
                facts(3, (1, 5)),
                prov(Some(3), (1, 5), false),
            ),
            (
                "an index edge to a non-zero value adopts: Godot's only \
                 automatic writes are resets to 0",
                prov(Some(0), (1, 4), false),
                facts(2, (1, 4)),
                prov(Some(2), (1, 4), true),
            ),
            (
                "an index edge to zero does not adopt: that is the machine's \
                 own reset",
                prov(Some(3), (1, 4), false),
                facts(0, (1, 4)),
                prov(Some(0), (1, 4), false),
            ),
            (
                "last_index None leaves explicit standing, so the port's own \
                 write survives the keystroke that opened the popup",
                prov(None, (1, 4), true),
                facts(0, (1, 4)),
                prov(Some(0), (1, 4), true),
            ),
            (
                "a repeated identical index preserves",
                prov(Some(2), (1, 4), true),
                facts(2, (1, 4)),
                prov(Some(2), (1, 4), true),
            ),
            (
                "a navigation key on a one-row list is a choice",
                prov(Some(0), (1, 4), false).chosen(),
                facts(0, (1, 4)),
                prov(Some(0), (1, 4), true),
            ),
            (
                "a navigation key that wraps to row 0 is a choice",
                prov(Some(2), (1, 4), false).chosen(),
                facts(0, (1, 4)),
                prov(Some(0), (1, 4), true),
            ),
            (
                "typing after a navigation key resets it",
                prov(Some(0), (1, 4), false).chosen(),
                facts(0, (1, 5)),
                prov(Some(0), (1, 5), false),
            ),
        ];
        for (what, prev, f, want) in rows {
            assert_eq!(advance(*prev, f), *want, "{what}");
        }
    }

    #[test]
    fn the_regression_this_rule_exists_for() {
        // `cancel_code_completion` leaves the index and the option list
        // intact (code_edit.cpp:2711-2719), so a reopened popup can carry a
        // stale non-zero index the user never touched. Preservation is a
        // NON-WRITE: no edge, same caret, and explicit must stay false so
        // Enter gives a newline rather than an insert nobody asked for.
        let prev = prov(Some(3), (1, 4), false);
        let next = advance(prev, &facts(3, (1, 4)));
        assert!(
            !next.explicit,
            "a preserved stale selection is not a choice"
        );
    }
}
