//! Completion-aware key routing for CodeEdit's autocomplete popup.
//!
//! Godot's CodeEdit autocomplete is driven by `_gui_input()`, which never
//! fires when Vim consumes the key via `set_input_as_handled()`. This module
//! dispatches completion-relevant keys *before* the engine so the popup can
//! trigger, navigate, and confirm, all without engine changes:
//! [`dispatch_overlay`] runs whatever the `editor.completion` overlay
//! resolved for this keystroke, folded by the SAME `resolve::dispose` every
//! other surface uses, and [`dispatch_host_key`] does what Godot's own
//! shortcut actions do for a key no row claimed. Keeping the popup in step
//! with engine edits is `effects::completion`, at the end of every engine
//! pass.
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
use vim_core::keymap::{KeyEvent, MappingMode, TrieLookup};

use crate::actions::action::{ActionCtx, CompletionOps};
use crate::actions::outcome::Outcome;
use crate::actions::resolve::{self, CandidateTarget, Disposition};
use crate::actions::surface::HostKey;
use crate::bridge;
use crate::bridge::code_edit_ext::CodeEditExt;
use crate::bridge::codec::usize_to_i32;
use crate::bridge::godot_host::GodotHost;

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
        self.editor.completion_popup_open()
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

    fn confirm(&mut self, replace: bool) {
        confirm_and_reconcile_completion(self.session, self.editor, replace);
    }

    fn cancel(&mut self) {
        self.editor.cancel_code_completion();
    }
}

/// Godot's own popup keys, for a key no `panelmap` row claimed: what
/// `CodeEdit::gui_input` does for each action (code_edit.cpp:768-845),
/// through the same public calls, except navigation, which is handed to
/// `CodeEdit` so it moves its own highlight. `Ignore` lets the engine have
/// the key.
pub(crate) fn host_key_disposition(ops: &mut dyn CompletionOps, key: HostKey) -> Disposition {
    match key {
        HostKey::None => Disposition::Ignore,
        // What the script editor's Completion Query item does
        // (script_text_editor.cpp:1700). Never gated on
        // `is_code_completion_enabled()`: Godot's editors never set it (#25).
        HostKey::Query => {
            ops.request(true);
            Disposition::Consume
        }
        _ if !ops.popup_visible() => Disposition::Ignore,
        HostKey::Navigate => Disposition::Handoff,
        HostKey::Close => {
            ops.cancel();
            Disposition::Ignore
        }
        HostKey::Accept | HostKey::Replace => {
            ops.confirm(key == HostKey::Replace);
            Disposition::Consume
        }
    }
}

/// Whether `key` extends or completes the pending mapping prefix, so the
/// mapping owns it rather than the popup (`inoremap j<CR>` after `j`). A key
/// that breaks the prefix is the popup's once the prefix is flushed.
/// vim-core v0.8.0 shows the pending keys only as notation, so they are read
/// back from it.
pub(crate) fn continues_pending_mapping(engine: &VimEngine, key: KeyEvent) -> bool {
    let Some(mode) = MappingMode::from_mode(engine.mode()) else {
        return false;
    };
    let mut keys = vim_core::execution::parse_keys_from_string(&engine.pending_mapping_display());
    if keys.is_empty() {
        return false;
    }
    keys.push(key);
    !matches!(engine.keymap().lookup(mode, &keys), TrieLookup::NoMatch)
}

pub(crate) fn dispatch_host_key(
    session: &mut VimSession<GodotHost>,
    editor: &mut Gd<CodeEdit>,
    key: HostKey,
) -> Disposition {
    if key == HostKey::None {
        return Disposition::Ignore;
    }
    host_key_disposition(&mut CompletionPort { session, editor }, key)
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

/// Confirm the selected completion and reconcile the text delta with the
/// engine so dot-repeat and macro recording capture the completed text.
///
/// One caret: the edit is cut at the caret it started from
/// (`reconcile::reconcile_completion`), because a replace, or a merge over
/// matching text, also deletes text after it. Several carets keep the
/// whole-text diff.
fn confirm_and_reconcile_completion(
    session: &mut VimSession<GodotHost>,
    editor: &mut Gd<CodeEdit>,
    replace: bool,
) {
    let before_text = editor.get_text().to_string();
    let single_caret = editor.get_caret_count() == 1;
    let caret_before = bridge::codec::LineIndex::new(&before_text).line_col_to_byte(
        &before_text,
        editor.get_caret_line(),
        editor.get_caret_column(),
    );

    // CodeEdit replaces `code_completion_base` (the typed prefix) with the
    // selected item's `insert_text`. This is the only mutation.
    editor.confirm_code_completion_ex().replace(replace).done();

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

    if single_caret {
        super::reconcile::reconcile_completion(
            session.engine_mut(),
            &before_text,
            &after_text,
            caret_before,
            after_byte,
        );
    } else {
        super::reconcile::reconcile_external_text_change(
            session.engine_mut(),
            &before_text,
            &after_text,
            after_byte,
            vim_core::execution::ExternalEditKind::Completion,
        );
    }

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

    /// A popup with no Godot in it, logging every command.
    #[derive(Default)]
    struct Popup {
        open: bool,
        log: Vec<&'static str>,
    }

    impl CompletionOps for Popup {
        fn popup_visible(&self) -> bool {
            self.open
        }
        fn option_count(&self) -> i32 {
            3
        }
        fn selected_index(&self) -> i32 {
            if self.open {
                0
            } else {
                -1
            }
        }
        fn request(&mut self, force: bool) {
            self.log
                .push(if force { "request(force)" } else { "request" });
            self.open = true;
        }
        fn select(&mut self, _index: i32) {
            self.log.push("select");
        }
        fn confirm(&mut self, replace: bool) {
            self.log.push(if replace { "replace" } else { "accept" });
            self.open = false;
        }
        fn cancel(&mut self) {
            self.log.push("cancel");
            self.open = false;
        }
    }

    /// An Insert session with `lhs` mapped to `<Esc>` and `typed` sent.
    fn pending(lhs: &str, typed: &str) -> vim_core::execution::HostSession {
        use vim_core::execution::{parse_keys_from_string, HostSession};
        use vim_core::keymap::{Key, MappingFlags, MappingKind};
        let mut s = HostSession::new("x\n");
        s.set_shadow_text("x\n");
        s.engine_mut().map(
            MappingMode::Insert,
            &parse_keys_from_string(lhs),
            vec![KeyEvent::new(
                Key::Escape,
                vim_core::keymap::Modifiers::NONE,
            )],
            MappingKind::NonRecursive,
            MappingFlags::default(),
        );
        for key in parse_keys_from_string(&format!("i{typed}")) {
            s.process_key_host(key);
        }
        s
    }

    #[test]
    fn a_popup_key_belongs_to_a_mapping_only_if_it_continues_it() {
        use vim_core::execution::parse_keys_from_string;
        let key = |k: &str| parse_keys_from_string(k)[0];
        // (what, mapping, typed, key, continues)
        let rows = [
            ("inoremap jk: Enter breaks it", "jk", "j", "<CR>", false),
            ("inoremap jk: Down breaks it", "jk", "j", "<Down>", false),
            ("inoremap jk: k completes it", "jk", "j", "k", true),
            (
                "inoremap j<CR>: Enter completes it",
                "j<CR>",
                "j",
                "<CR>",
                true,
            ),
            (
                "inoremap ;<Tab>x: Tab extends it",
                ";<Tab>x",
                ";",
                "<Tab>",
                true,
            ),
            ("nothing pending", "jk", "", "<CR>", false),
        ];
        for (what, lhs, typed, k, want) in rows {
            let s = pending(lhs, typed);
            assert_eq!(
                s.engine().has_pending_mapping(),
                !typed.is_empty(),
                "{what}: pending state"
            );
            assert_eq!(
                continues_pending_mapping(s.engine(), key(k)),
                want,
                "{what}"
            );
        }
    }

    #[test]
    fn host_keys_do_what_codeedit_does_and_nothing_else() {
        use Disposition::{Consume, Handoff, Ignore};
        // (key, popup open, disposition, commands)
        let rows: &[(HostKey, bool, Disposition, &[&str])] = &[
            (HostKey::None, true, Ignore, &[]),
            // Whether the row was chosen does not matter: Godot preselects
            // row 0 and accepts it (code_edit.cpp:2582, 822-823).
            (HostKey::Accept, true, Consume, &["accept"]),
            (HostKey::Replace, true, Consume, &["replace"]),
            (HostKey::Navigate, true, Handoff, &[]),
            (HostKey::Close, true, Ignore, &["cancel"]),
            (HostKey::Query, true, Consume, &["request(force)"]),
            // #25: Ctrl+Space opens the popup, and never reaches i_CTRL-@.
            (HostKey::Query, false, Consume, &["request(force)"]),
            (HostKey::Accept, false, Ignore, &[]),
            (HostKey::Replace, false, Ignore, &[]),
            (HostKey::Navigate, false, Ignore, &[]),
            (HostKey::Close, false, Ignore, &[]),
        ];
        for (key, open, want, log) in rows {
            let mut popup = Popup {
                open: *open,
                ..Popup::default()
            };
            assert_eq!(
                host_key_disposition(&mut popup, *key),
                *want,
                "{key:?} open={open}"
            );
            assert_eq!(popup.log, *log, "{key:?} open={open}");
        }
    }
}
