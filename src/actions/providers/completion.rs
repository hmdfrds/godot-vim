//! `editor.completion` — the autocomplete popup's keys, as named verbs.
//!
//! The second half of P9, and a different proof from `debugger.rs`: that one
//! shows a new *panel* costs one file, this one shows a hardcoded key table
//! deep inside the editor pipeline can become data without moving transports.
//!
//! # This surface is an overlay, lent per keystroke rather than probed
//!
//! Every other surface is reached by probing a sampled [`FocusChain`]. This one
//! cannot be, for a reason that is structural rather than incidental: whether
//! the completion popup is visible is a **per-keystroke** fact, and the focus
//! chain is sampled once per *focus change* and cached against
//! `(focus owner, epoch, index generation)`. A probe reading popup visibility
//! would be answering from a cache that is stale by construction.
//!
//! So [`EDITOR_COMPLETION`] declares `probe: |_| None` and an
//! [`OverlaySpec`]: `Forest::overlay` materialises it as an ordinary
//! one-element `SurfacePath` for the one transport that has the popup in
//! hand, `GodotVimCore::handle_gui_input_impl`, after the IME guard and
//! before the vim engine sees anything. The path is ordinary, so `resolve`,
//! flags, params, capabilities, the consumption fold and `:panelmap` explain
//! are the same code every classified surface runs, not a reimplementation.
//!
//! `resolve` and `classify` are called only from `_input`, so classification
//! would deliver these keys into the transport that was rejected three times,
//! which is why the surface is lent rather than probed.
//!
//! # Why it stays on `gui_input`, restated because it keeps being asked
//!
//! Moving these keys onto the `_input` registry with the panel bindings was
//! rejected three times and the reasons have not changed:
//!
//! - `_input` is registered **per viewport**. A script editor floated into its
//!   own `Window` has a different viewport, so `_input` never fires there and
//!   the popup keys would silently die in exactly the layout power users pick.
//! - `_input` runs **outside the IME guard**. `handle_gui_input_impl` cancels
//!   or defers to an active preedit before any key is interpreted; a CJK user
//!   composing a word would have `<CR>` stolen by `godotvim.completion.confirm`
//!   mid-composition.
//! - The third routing state, "engine skipped, event deliberately NOT
//!   consumed", is a `gui_input` fact: it is what Up/Down need so `CodeEdit`
//!   moves its own popup selection. It is `Consumption::Handoff`, declared by
//!   [`NAVIGATE`] and folded to `Disposition::Handoff` by the one consumption
//!   fold; on `_input` the variant is unreachable by audit A9'.
//!
//! # What the user gets
//!
//! Keys that were literals in a `match` are rows in `:panelmap`, rebindable
//! and unmappable like every other binding:
//!
//! ```vim
//! panelunmap editor.completion <Tab>
//! panelmap   editor.completion <C-y>   godotvim.completion.confirm
//! panelmap   editor.completion <C-e>   godotvim.completion.dismiss
//! panelmap   editor.completion <C-j>   godotvim.completion.next
//! ```
//!
//! # One deliberate behaviour change, stated plainly
//!
//! The old table matched `Key::Up | Key::Down`, `Key::Tab | Key::Enter` and
//! `Key::Escape` **ignoring modifiers**, so Ctrl+Enter confirmed a completion
//! and Shift+Up was swallowed by the popup. A binding table cannot express
//! "any modifiers" and should not: `<CR>` here means `<CR>`. Modified variants
//! now reach the vim engine, which is both more correct and — unlike before —
//! visible and reversible from a vimrc (`panelmap <shift> editor.completion
//! <Up> godotvim.completion.navigate` restores the Shift+Up half).

use crate::actions::action::{ActionCtx, ActionSpec, CompletionOps};
use crate::actions::bind::Consumption;
use crate::actions::caps::Caps;
use crate::actions::outcome::Outcome;
use crate::actions::surface::{OverlaySpec, Seal, SurfaceSpec};

use super::Provider;

/// The surface the `gui_input` transport lends as an overlay.
pub(crate) const SURFACE: crate::actions::surface::SurfaceId = "editor.completion";

pub(crate) static EDITOR_COMPLETION: SurfaceSpec = SurfaceSpec {
    id: SURFACE,
    // Rootless BY AUDIT (V-O1), not by habit: membership is stacking, not
    // parenthood, and a parent would invite an upward walk into `panel`'s
    // <void> Ctrl+hjkl rules from inside Insert mode.
    parent: None,
    seal: Seal::Open,
    // The chain-driven grant. An overlay never classifies, so this is never
    // reached; the live grant is on the overlay below.
    grants: |_| Caps::empty(),
    // Still never. An overlay joins the path through `Forest::overlay`, which
    // `_input` does not call, which is the whole parity argument.
    probe: |_| None,
    overlay: Some(OverlaySpec {
        // Deliberately NOT popup-gated. Popup-gating would kill <C-@>, <C-n>
        // and <C-p> with the popup closed, which are exactly the three keys
        // the Ctrl+Space bug fix exists to revive. The popup enters as a
        // CAPABILITY instead, where the gate is declared per verb.
        active: |f| f.at_attached_editor && f.insert_like(),
        grants: |f| {
            if f.popup() {
                Caps::POPUP
            } else {
                Caps::empty()
            }
        },
        when: "while the script editor is in an insert-like mode",
    }),
    on_key: None,
    // WAS `false`, contradicting the prose at the old transport-only lookup
    // ("honouring it here would turn a Dvorak Ctrl+p into a completion key").
    // Inert only while the surface never anchored; live from the first
    // overlay walk, so it is corrected here and audited by V-O3.
    refuses_positional: true,
    yields_to_engine: false,
};

/// The popup, or a declination.
///
/// `None` means this transport lends no popup — `:action
/// godotvim.completion.confirm` from the command line, or a `panelmap panel
/// <C-y> godotvim.completion.confirm` the user wrote by mistake. Declining is
/// the only honest answer: there is nothing to confirm.
fn ops<'c, 'a>(cx: &'c mut ActionCtx<'a>) -> Option<&'c mut (dyn CompletionOps + 'a)> {
    cx.completion()
}

/// Wrap `index` into `0..count`, or `None` when there is nothing to select.
///
/// Extracted and pure so the wrap-around is testable on its own: the old code
/// had the same two expressions written twice with the bounds spelled
/// differently (`current + 1 >= count` vs `current <= 0`), which is exactly the
/// shape an off-by-one hides in.
fn wrap(index: i32, count: i32) -> Option<i32> {
    if count <= 0 {
        return None;
    }
    Some(index.rem_euclid(count))
}

pub(crate) static TRIGGER: ActionSpec = ActionSpec {
    id: "godotvim.completion.trigger",
    desc: "Completion: open the popup",
    // No capability. `Caps` describes what a focused *control* affords and is
    // sampled from the focus chain; this surface never classifies, so its
    // verbs arrive with `Caps::empty()` and anything but `empty` would gate
    // every one of them off. The real precondition is `completion_enabled`,
    // asked of the port.
    requires: Caps::empty(),
    // There is no popup outside the attached editor, and a host request that
    // silently declined would look like a broken keybinding.
    host_invocable: false,
    default_consume: None,
    run: |cx| {
        let Some(ops) = ops(cx) else {
            return Outcome::Declined;
        };
        if !ops.completion_enabled() {
            // The user turned autocompletion off in EditorSettings. Forcing a
            // popup they disabled is worse than doing nothing, and declining
            // lets Ctrl+Space reach the engine as an ordinary chord.
            return Outcome::Declined;
        }
        ops.request(true);
        Outcome::Handled
    },
};

pub(crate) static NEXT: ActionSpec = ActionSpec {
    id: "godotvim.completion.next",
    desc: "Completion: next candidate, opening the popup if closed",
    requires: Caps::empty(),
    host_invocable: false,
    default_consume: None,
    run: |cx| {
        let Some(ops) = ops(cx) else {
            return Outcome::Declined;
        };
        if ops.popup_visible() {
            let Some(next) = wrap(ops.selected_index() + 1, ops.option_count()) else {
                return Outcome::Declined;
            };
            ops.select(next);
            return Outcome::Handled;
        }
        if !ops.completion_enabled() {
            return Outcome::Declined;
        }
        // Godot auto-selects index 0 on a fresh request, which is already
        // Vim's `<C-n>` semantics (forward search from the top). Nothing more
        // to do.
        ops.request(true);
        Outcome::Handled
    },
};

pub(crate) static PREV: ActionSpec = ActionSpec {
    id: "godotvim.completion.prev",
    desc: "Completion: previous candidate, opening the popup if closed",
    requires: Caps::empty(),
    host_invocable: false,
    default_consume: None,
    run: |cx| {
        let Some(ops) = ops(cx) else {
            return Outcome::Declined;
        };
        if ops.popup_visible() {
            let Some(prev) = wrap(ops.selected_index() - 1, ops.option_count()) else {
                return Outcome::Declined;
            };
            ops.select(prev);
            return Outcome::Handled;
        }
        if !ops.completion_enabled() {
            return Outcome::Declined;
        }
        ops.request(true);
        // Vim's `<C-p>` searches BACKWARD, so a fresh popup must land on the
        // last candidate rather than the first. `request` is synchronous, so
        // the list is already there to count.
        if ops.popup_visible() {
            if let Some(last) = wrap(ops.option_count() - 1, ops.option_count()) {
                ops.select(last);
            }
        }
        Outcome::Handled
    },
};

// THE RULE OF THIS FILE, and it decides which of the old popup checks lived
// and which died: a GATE becomes a CAPABILITY; a BRANCH stays in the body.
// `NEXT`/`PREV` branch on the popup because they do different work in each
// state, so their `popup_visible()` reads stay. `CONFIRM`, `DISMISS` and
// `NAVIGATE` refused outright with no popup up, which is a routing
// precondition, so it is `requires: Caps::POPUP` and the pipeline decides
// before the body runs, which is the only place a precondition can be
// decided safely under `<void>`.
//
// There is no third path to a popup-less run, and no defence-in-depth guard
// is kept: a verb bound on a surface that lends no port declines at
// `let Some(ops) = ops(cx) else`, and a verb bound on the overlay is gated
// by `POPUP`. A kept guard would be unreachable code that reads as a
// decision.

pub(crate) static CONFIRM: ActionSpec = ActionSpec {
    id: "godotvim.completion.confirm",
    desc: "Completion: accept the selected candidate",
    // THE load-bearing line of this file, replacing the declination that
    // was: `POPUP` decides BEFORE the body runs, so with no popup up `<CR>`
    // is a `Hit::Miss`, the walk exhausts, and the engine inserts a newline,
    // even under a user's `<void>`.
    requires: Caps::POPUP,
    host_invocable: false,
    default_consume: None,
    run: |cx| {
        let Some(ops) = ops(cx) else {
            return Outcome::Declined;
        };
        ops.confirm();
        Outcome::Handled
    },
};

pub(crate) static DISMISS: ActionSpec = ActionSpec {
    id: "godotvim.completion.dismiss",
    desc: "Completion: close the popup, keeping insert mode",
    requires: Caps::POPUP,
    host_invocable: false,
    default_consume: None,
    // The body reports honestly that it dismissed; `dispose` reads the
    // rule's declared policy downstream of this outcome. The shipped table
    // binds no `<Esc>`, so one press still exits Insert through the engine
    // and `handle_set_mode` cancels the popup on Normal entry; the two-stage
    // Escape is one unflagged vimrc line:
    // `panelmap editor.completion <Esc> godotvim.completion.dismiss`.
    run: |cx| {
        let Some(ops) = ops(cx) else {
            return Outcome::Declined;
        };
        ops.cancel();
        Outcome::Handled
    },
};

pub(crate) static NAVIGATE: ActionSpec = ActionSpec {
    id: "godotvim.completion.navigate",
    desc: "Completion: hand this key to CodeEdit's own popup handling",
    requires: Caps::POPUP,
    host_invocable: false,
    // The third routing state, declared on the verb rather than flagged on a
    // lent port: `CodeEdit::_gui_input` moves the popup selection on Up/Down
    // by itself and does it better than we would (it handles scrolling and
    // page bounds), so acceptance folds to `Disposition::Handoff`: skip the
    // engine, do not consume, let the control have it. Audit A9' holds this
    // declaration to a capability no classified path can satisfy.
    default_consume: Some(Consumption::Handoff),
    run: |cx| {
        if ops(cx).is_none() {
            return Outcome::Declined;
        }
        Outcome::Handled
    },
};

const ACTIONS: &[&ActionSpec] = &[&TRIGGER, &NEXT, &PREV, &CONFIRM, &DISMISS, &NAVIGATE];

/// Today's hardcoded table, as text.
///
/// `<C-@>` and not `<C-Space>`: `bridge::input::translate_key` folds Ctrl+Space
/// into `Char('@') + CTRL` before anything downstream sees it (the same fold
/// every terminal does), so `<C-Space>` would parse to `Char(' ') + CTRL` and
/// never match a real keystroke. `parse_lhs` accepts both spellings, which is
/// exactly why the wrong one is a silent dead key.
///
/// No flags: `confirm`, `dismiss` and `navigate` are gated by `Caps::POPUP`,
/// so with no popup up the rules miss and `<CR>` inserts a newline, `<Tab>`
/// indents and the arrows move the caret, without a single mode check in the
/// binding table.
///
/// No `<Esc>` row. `DISMISS` returns `Handled` now, and an elastic `<Esc>`
/// rule would consume the press that should also leave Insert; the engine's
/// own `SetMode(Normal)` cancels the popup on the way out (`effects/mode.rs`),
/// so one press still does both. The two-stage Escape is one vimrc line.
const DEFAULTS: &str = "\
panelmap editor.completion <C-@> godotvim.completion.trigger
panelmap editor.completion <C-n> godotvim.completion.next
panelmap editor.completion <C-p> godotvim.completion.prev
panelmap editor.completion <Tab> godotvim.completion.confirm
panelmap editor.completion <CR> godotvim.completion.confirm
panelmap editor.completion <Up> godotvim.completion.navigate
panelmap editor.completion <Down> godotvim.completion.navigate
";

pub(crate) const PROVIDER: Provider = Provider {
    tag: "godotvim.completion",
    surfaces: &[&EDITOR_COMPLETION],
    actions: ACTIONS,
    defaults: DEFAULTS,
};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::actions::action::Params;
    use crate::actions::resolve::Disposition;

    /// A popup with no Godot in it.
    ///
    /// This is the characterization harness the design's P9 gate asks for. It
    /// could not be written against the old `try_handle_completion`, which
    /// takes `&mut Gd<CodeEdit>` and `&mut VimSession<GodotHost>` — both
    /// unconstructible in a `cdylib` under `cargo test`. Extracting the
    /// decision behind `CompletionOps` is what made the trichotomy testable at
    /// all, and every row below is transcribed from the shipped match arms.
    #[derive(Debug, Default, PartialEq, Eq)]
    struct FakePopup {
        visible: bool,
        enabled: bool,
        options: i32,
        selected: i32,
        /// Every command, in order. Asserting the LOG rather than the end
        /// state is what catches "confirmed, but also cancelled".
        log: Vec<String>,
    }

    impl FakePopup {
        fn closed() -> Self {
            Self {
                enabled: true,
                selected: -1,
                ..Self::default()
            }
        }

        fn open(options: i32, selected: i32) -> Self {
            Self {
                visible: true,
                enabled: true,
                options,
                selected,
                ..Self::default()
            }
        }

        fn disabled() -> Self {
            Self {
                selected: -1,
                ..Self::default()
            }
        }
    }

    impl CompletionOps for FakePopup {
        fn popup_visible(&self) -> bool {
            self.visible
        }
        fn completion_enabled(&self) -> bool {
            self.enabled
        }
        fn option_count(&self) -> i32 {
            self.options
        }
        fn selected_index(&self) -> i32 {
            self.selected
        }
        fn request(&mut self, force: bool) {
            self.log.push(format!("request(force={force})"));
            // Godot's request is synchronous and auto-selects index 0 when it
            // finds candidates. The fake reproduces that, because `prev`'s
            // "then jump to the last one" depends on it.
            if self.enabled && self.options > 0 {
                self.visible = true;
                self.selected = 0;
            }
        }
        fn select(&mut self, index: i32) {
            self.log.push(format!("select({index})"));
            self.selected = index;
        }
        fn confirm(&mut self) {
            self.log.push("confirm".into());
            self.visible = false;
        }
        fn cancel(&mut self) {
            self.log.push("cancel".into());
            self.visible = false;
            // Godot's `cancel_code_completion` (code_edit.cpp:2711-2719)
            // clears `code_completion_active` and leaves the index and the
            // option list intact. Resetting the index here makes the
            // provenance regression test pass vacuously.
        }
    }

    /// Run one verb against one popup state, returning the outcome.
    ///
    /// Takes `params` rather than hardcoding `Params::new()`: parameter
    /// behaviour is untestable otherwise, and `require_selection` is on its
    /// way here.
    fn run(spec: &ActionSpec, params: Params, popup: &mut FakePopup) -> Outcome {
        let mut cx = ActionCtx::new(None, params).with_completion(popup);
        (spec.run)(&mut cx)
    }

    /// The disposition the transport computes, written out here rather than
    /// called, so the assertion is not a tautology against `resolve::dispose`.
    fn fold(spec: &ActionSpec, params: Params, popup: &mut FakePopup) -> Disposition {
        let outcome = run(spec, params, popup);
        match (
            spec.default_consume.unwrap_or(Consumption::Elastic),
            outcome.is_consumed(),
        ) {
            (Consumption::Void, _) => Disposition::Consume,
            (Consumption::Handoff, true) => Disposition::Handoff,
            (_, true) => Disposition::Consume,
            (_, false) => Disposition::Ignore,
        }
    }

    // ── The trichotomy, one row per shipped match arm ────────────────

    #[test]
    fn trigger_opens_the_popup_and_consumes() {
        let mut popup = FakePopup::closed();
        popup.options = 3;
        assert_eq!(
            fold(&TRIGGER, Params::new(), &mut popup),
            Disposition::Consume
        );
        assert_eq!(popup.log, vec!["request(force=true)"]);
        assert!(popup.visible);
    }

    #[test]
    fn trigger_declines_when_completion_is_disabled() {
        // `editor.is_code_completion_enabled()` false → the old code returned
        // `None` and the chord reached the engine. Same here, via declination.
        let mut popup = FakePopup::disabled();
        assert_eq!(
            fold(&TRIGGER, Params::new(), &mut popup),
            Disposition::Ignore
        );
        assert!(popup.log.is_empty(), "must not force a disabled popup");
    }

    #[test]
    fn next_opens_a_closed_popup_rather_than_moving_nothing() {
        let mut popup = FakePopup::closed();
        popup.options = 4;
        assert_eq!(fold(&NEXT, Params::new(), &mut popup), Disposition::Consume);
        // Godot auto-selects 0, which IS Vim's forward search. No extra
        // select() call, and asserting the log is what proves it.
        assert_eq!(popup.log, vec!["request(force=true)"]);
        assert_eq!(popup.selected, 0);
    }

    #[test]
    fn prev_opens_a_closed_popup_and_lands_on_the_last_candidate() {
        // The asymmetry that makes `<C-p>` `<C-p>` and not "`<C-n>` backwards".
        let mut popup = FakePopup::closed();
        popup.options = 4;
        assert_eq!(fold(&PREV, Params::new(), &mut popup), Disposition::Consume);
        assert_eq!(popup.log, vec!["request(force=true)", "select(3)"]);
        assert_eq!(popup.selected, 3);
    }

    #[test]
    fn next_and_prev_move_and_wrap_on_a_visible_popup() {
        for (spec, from, want) in [(&NEXT, 0, 1), (&NEXT, 2, 0), (&PREV, 1, 0), (&PREV, 0, 2)] {
            let mut popup = FakePopup::open(3, from);
            assert_eq!(
                fold(spec, Params::new(), &mut popup),
                Disposition::Consume,
                "{}",
                spec.id
            );
            assert_eq!(popup.selected, want, "{} from {from}", spec.id);
        }
    }

    #[test]
    fn an_empty_candidate_list_declines_instead_of_dividing_by_zero() {
        // `count == 0` with the popup somehow visible. The old code guarded
        // with `if count > 0 { ... }` and then returned `Some(true)` anyway,
        // consuming the key to do nothing; declining is strictly better and
        // `wrap` makes it structural.
        for spec in [&NEXT, &PREV] {
            let mut popup = FakePopup::open(0, -1);
            assert_eq!(
                fold(spec, Params::new(), &mut popup),
                Disposition::Ignore,
                "{}",
                spec.id
            );
            assert!(popup.log.is_empty());
        }
    }

    #[test]
    fn confirm_accepts_a_visible_popup_and_consumes() {
        let mut popup = FakePopup::open(2, 1);
        assert_eq!(
            fold(&CONFIRM, Params::new(), &mut popup),
            Disposition::Consume
        );
        assert_eq!(popup.log, vec!["confirm"]);
    }

    #[test]
    fn with_no_popup_the_capability_gate_stops_the_key_before_the_verb_runs() {
        // The regression that would be reported as "Enter stopped working",
        // decided at the resolve level now: with no popup the overlay grants
        // nothing, `POPUP` misses, the walk exhausts, and the engine inserts
        // the newline. Deciding BEFORE the body runs is what makes the
        // precondition safe under a user's `<void>`.
        use crate::actions::resolve::{Resolution, Stop};
        let index = crate::actions::bind::builtin_index(&crate::actions::specs::registry());
        let rows = [
            ("<CR>", "godotvim.completion.confirm"),
            ("<Tab>", "godotvim.completion.confirm"),
            ("<Up>", "godotvim.completion.navigate"),
            ("<Down>", "godotvim.completion.navigate"),
        ];
        for (notation, verb) in rows {
            assert_eq!(
                resolve_overlay(&index, notation, &popup_facts(-1)),
                Resolution::None(Stop::Exhausted),
                "{notation} must miss with no popup"
            );
            let Resolution::Run { candidates, .. } =
                resolve_overlay(&index, notation, &popup_facts(0))
            else {
                panic!("{notation} must resolve with the popup open");
            };
            let crate::actions::resolve::CandidateTarget::Action(_, spec) = &candidates[0].target
            else {
                panic!("an action rule");
            };
            assert_eq!(spec.id, verb, "{notation}");
        }
    }

    #[test]
    fn dismiss_cancels_and_reports_that_it_did() {
        // The body reports what happened; consumption is the rule's declared
        // policy, read downstream by `dispose`. No shipped rule binds `<Esc>`
        // any more, so one press still exits Insert through the engine, and
        // `handle_set_mode` cancels the popup on Normal entry.
        let mut popup = FakePopup::open(3, 1);
        assert_eq!(run(&DISMISS, Params::new(), &mut popup), Outcome::Handled);
        assert_eq!(popup.log, vec!["cancel"]);
        assert!(!popup.visible);
        let mut popup = FakePopup::open(3, 1);
        assert_eq!(
            fold(&DISMISS, Params::new(), &mut popup),
            Disposition::Consume
        );
    }

    #[test]
    fn navigate_hands_the_key_to_the_control_without_consuming_it() {
        // THE third routing state, now declared on the verb rather than
        // flagged on the port: "handled by us, engine skipped, event NOT
        // marked handled".
        let mut popup = FakePopup::open(3, 0);
        assert_eq!(run(&NAVIGATE, Params::new(), &mut popup), Outcome::Handled);
        assert!(popup.log.is_empty(), "the control does the moving, not us");
        assert_eq!(
            popup.selected, 0,
            "we must not move the selection ourselves"
        );
        let mut popup = FakePopup::open(3, 0);
        assert_eq!(
            fold(&NAVIGATE, Params::new(), &mut popup),
            Disposition::Handoff
        );
    }

    #[test]
    fn every_verb_declines_on_a_transport_that_lends_no_popup() {
        // `:action godotvim.completion.confirm` from the command line, and any
        // `panelmap panel <C-y> godotvim.completion.confirm` a user writes.
        // There is no popup to act on, so every one of them must decline
        // rather than consume.
        let mut effects = Vec::new();
        for spec in ACTIONS {
            let mut cx = ActionCtx::recording(&mut effects);
            assert_eq!((spec.run)(&mut cx), Outcome::Declined, "{}", spec.id);
        }
        assert!(effects.is_empty());
    }

    // ── Shape ────────────────────────────────────────────────────────

    #[test]
    fn wrap_is_total_over_every_index_and_count() {
        assert_eq!(wrap(0, 0), None);
        assert_eq!(wrap(5, -1), None);
        assert_eq!(wrap(0, 3), Some(0));
        assert_eq!(wrap(3, 3), Some(0));
        assert_eq!(wrap(-1, 3), Some(2), "rem_euclid, not %");
        assert_eq!(wrap(-4, 3), Some(2));
    }

    #[test]
    fn the_overlay_never_probes_and_joins_the_path_only_when_its_predicate_holds() {
        // Asserted rather than commented, because a future edit that "fixes"
        // the probe would put a stale-cache read on the hot path and the
        // symptom would be an intermittently dead `<Tab>`.
        use crate::actions::surface::fixtures::{code_edit, no_focus_owner, plain};
        use crate::actions::surface::FocusChain;
        let chains = [
            no_focus_owner(),
            FocusChain {
                nodes: vec![code_edit(1), plain("CodeTextEditor", 2)],
                ..Default::default()
            },
        ];
        for chain in chains {
            assert_eq!((EDITOR_COMPLETION.probe)(&chain), None);
        }
        assert!(
            EDITOR_COMPLETION.overlay.is_some(),
            "the surface joins the path as an overlay, not through a probe"
        );
    }

    /// Every mode the engine has, plus "no controller".
    fn every_mode() -> Vec<Option<vim_core::primitives::Mode>> {
        use vim_core::primitives::{Mode, Operator, VisualType};
        vec![
            None,
            Some(Mode::Normal),
            Some(Mode::Insert),
            Some(Mode::Replace),
            Some(Mode::VirtualReplace),
            Some(Mode::CommandLine),
            Some(Mode::Visual(VisualType::Char)),
            Some(Mode::Visual(VisualType::Line)),
            Some(Mode::Visual(VisualType::Block)),
            Some(Mode::Select(VisualType::Char)),
            Some(Mode::OperatorPending(Operator::Delete)),
        ]
    }

    #[test]
    fn the_activation_table_is_exhaustive_over_attachment_and_mode() {
        use crate::actions::surface::OverlayFacts;
        use vim_core::primitives::Mode;
        let overlay = EDITOR_COMPLETION.overlay.as_ref().expect("declared");
        for at_attached_editor in [false, true] {
            for mode in every_mode() {
                let facts = OverlayFacts {
                    at_attached_editor,
                    mode,
                    selected_index: -1,
                };
                let want = at_attached_editor
                    && matches!(
                        mode,
                        Some(Mode::Insert | Mode::Replace | Mode::VirtualReplace)
                    );
                assert_eq!(
                    (overlay.active)(&facts),
                    want,
                    "at_attached_editor={at_attached_editor} mode={mode:?}"
                );
            }
        }
    }

    #[test]
    fn the_grant_is_the_popup_and_nothing_else() {
        use crate::actions::surface::OverlayFacts;
        let overlay = EDITOR_COMPLETION.overlay.as_ref().expect("declared");
        for (selected_index, want) in [(-1, Caps::empty()), (0, Caps::POPUP), (3, Caps::POPUP)] {
            let facts = OverlayFacts {
                at_attached_editor: true,
                mode: Some(vim_core::primitives::Mode::Insert),
                selected_index,
            };
            assert_eq!((overlay.grants)(&facts), want, "index {selected_index}");
        }
    }

    #[test]
    fn overlay_facts_default_is_the_absent_popup() {
        // Pins the hand-written `Default`: a derived one gives
        // `selected_index: 0`, which reads as "a popup is up with row 0
        // selected".
        use crate::actions::surface::OverlayFacts;
        assert!(!OverlayFacts::default().popup());
        assert!(!OverlayFacts::default().insert_like());
    }

    fn popup_facts(selected_index: i32) -> crate::actions::surface::OverlayFacts {
        crate::actions::surface::OverlayFacts {
            at_attached_editor: true,
            mode: Some(vim_core::primitives::Mode::Insert),
            selected_index,
        }
    }

    #[test]
    fn the_overlay_path_is_one_rootless_element_with_grants_only_caps() {
        use crate::actions::surface::{Anchor, Seal};
        let forest = crate::actions::providers::forest();
        let path = forest.overlay(&popup_facts(0)).expect("active");
        assert_eq!(path.ids, vec!["editor.completion"]);
        assert_eq!(path.anchor, Anchor::Rootless);
        assert_eq!(path.caps, Caps::POPUP);
        assert_eq!(path.seal, Seal::Open);
        assert!(!path.anchor_yields_to_engine);
        assert!(path.anchor_refuses_positional);
    }

    #[test]
    fn no_widget_capability_reaches_the_overlay() {
        // Grants-only caps are what keep `godotvim.search.accept`
        // (requires: Caps::TEXTENTRY) permanently gated on this surface,
        // rather than merely unlikely: the control under the caret IS a
        // TextEdit, and an anchored path would grant TEXTENTRY.
        let forest = crate::actions::providers::forest();
        let path = forest.overlay(&popup_facts(0)).expect("active");
        assert!(!path.caps.satisfies(Caps::TEXTENTRY));
    }

    /// The shipped defaults plus `lines`, applied as a user vimrc.
    fn index_with(lines: &str) -> crate::actions::bind::BindingIndex {
        let reg = crate::actions::specs::registry();
        let mut index = crate::actions::bind::builtin_index(&reg);
        let mut diagnostics = Vec::new();
        crate::actions::bind::apply_text(
            &mut index,
            &reg,
            lines,
            &vim_core::keymap::MappingOwner::User,
            "test",
            crate::actions::bind::Provenance::User,
            &mut diagnostics,
        );
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        index
    }

    /// Resolve `notation` against the overlay path for `facts`.
    fn resolve_overlay(
        index: &crate::actions::bind::BindingIndex,
        notation: &str,
        facts: &crate::actions::surface::OverlayFacts,
    ) -> crate::actions::resolve::Resolution {
        use crate::actions::resolve::{resolve, ResolveInput};
        let path = index.forest().overlay(facts).expect("overlay active");
        let key = crate::actions::keys::parse_lhs(notation).expect("parses")[0];
        let probes = crate::actions::keys::Probes::from_key(key);
        let reg = crate::actions::specs::registry();
        let claims = |_: vim_core::keymap::KeyEvent| false;
        resolve(&ResolveInput {
            probes: &probes,
            path: &path,
            index,
            registry: &reg,
            vim_claims: &claims,
        })
    }

    #[test]
    fn native_on_the_overlay_is_indistinguishable_from_no_rule() {
        // A4' pinned, and the test that stops the sandbox hole reopening:
        // `native` carries no `requires` and cannot be gated, so it MUST mean
        // exactly what no rule means. The transport maps every
        // `Resolution::None` to an empty plan, and an empty plan folds to
        // `Ignore`, so the engine gets the key either way. Three `native`
        // lines in a committed project vimrc therefore cannot suppress the
        // engine on <Esc>, <C-c> and <C-[>.
        use crate::actions::resolve::{dispose, Resolution, Stop};
        let index = index_with("panelmap editor.completion <C-y> native");
        let resolution = resolve_overlay(&index, "<C-y>", &popup_facts(0));
        assert_eq!(
            resolution,
            Resolution::None(Stop::Native("editor.completion"))
        );
        // The transport's mapping: every stop yields empty candidates.
        assert_eq!(
            dispose(&[], false, |_| unreachable!("nothing to run")),
            Disposition::Ignore
        );
    }

    #[test]
    fn a_void_rule_changes_the_disposition_on_the_overlay() {
        // One of the three capability tests that fail under a patch and pass
        // only under one pipeline: `<void>` used to parse, register, echo in
        // `:panelmap`, and do nothing here.
        use crate::actions::resolve::{dispose, Resolution};
        let index =
            index_with("panelmap <void> editor.completion <Esc> godotvim.completion.dismiss");
        let Resolution::Run { candidates, .. } = resolve_overlay(&index, "<Esc>", &popup_facts(1))
        else {
            panic!("the void rule must resolve with the popup open");
        };
        assert_eq!(candidates[0].consume, Consumption::Void);
        let mut popup = FakePopup::open(3, 1);
        let d = dispose(&candidates, false, |candidate| {
            let crate::actions::resolve::CandidateTarget::Action(_, spec) = &candidate.target
            else {
                panic!("an action rule");
            };
            run(spec, candidate.params.clone(), &mut popup)
        });
        assert_eq!(d, Disposition::Consume, "void consumes regardless");
    }

    #[test]
    fn a_norepeat_rule_consumes_an_echo_without_running() {
        use crate::actions::resolve::{dispose, Resolution};
        let index =
            index_with("panelmap <norepeat> editor.completion <C-n> godotvim.completion.next");
        let Resolution::Run { candidates, .. } = resolve_overlay(&index, "<C-n>", &popup_facts(0))
        else {
            panic!("the norepeat rule must resolve");
        };
        let mut popup = FakePopup::open(3, 0);
        let d = dispose(&candidates, true, |candidate| {
            let crate::actions::resolve::CandidateTarget::Action(_, spec) = &candidate.target
            else {
                panic!("an action rule");
            };
            run(spec, candidate.params.clone(), &mut popup)
        });
        assert_eq!(d, Disposition::Consume, "an echo must not leak to Godot");
        assert!(popup.log.is_empty(), "and must not run the verb");
    }

    #[test]
    fn a_panelmap_parameter_reaches_the_verb() {
        // The plumbing the deleted `ActionCtx::new(None, Params::new())`
        // dropped: the resolved candidate carries the rule's parameters, and
        // `dispatch_overlay` clones them into the ctx.
        use crate::actions::resolve::Resolution;
        let index = index_with(
            "panelmap editor.completion <C-y> godotvim.completion.confirm require_selection=0",
        );
        let Resolution::Run { candidates, .. } = resolve_overlay(&index, "<C-y>", &popup_facts(0))
        else {
            panic!("the parameterised rule must resolve");
        };
        assert_eq!(candidates[0].params.int("require_selection", 1), 0);
    }

    #[test]
    fn the_overlay_is_absent_outside_an_insert_like_mode() {
        use crate::actions::surface::OverlayFacts;
        use vim_core::primitives::{Mode, VisualType};
        let forest = crate::actions::providers::forest();
        for mode in [
            None,
            Some(Mode::Normal),
            Some(Mode::Visual(VisualType::Char)),
            Some(Mode::CommandLine),
        ] {
            let facts = OverlayFacts {
                at_attached_editor: true,
                mode,
                selected_index: 0,
            };
            assert!(forest.overlay(&facts).is_none(), "{mode:?}");
        }
    }

    #[test]
    fn the_popup_is_a_capability_and_the_rest_is_a_branch() {
        // The rule, written where it is enforced: a gate becomes a
        // capability, a branch stays in the body. This test's predecessor
        // claimed a `requires` bit here "would gate every completion key off
        // permanently, silently" — the sentence this refactor falsifies: the
        // overlay's grants are real capabilities now, decided by the same
        // `hit_from` gate every classified surface gets.
        for spec in [&CONFIRM, &DISMISS, &NAVIGATE] {
            assert_eq!(
                spec.requires,
                Caps::POPUP,
                "{} refuses outright with no popup, which is a routing \
                 precondition and therefore a capability",
                spec.id
            );
        }
        for spec in [&TRIGGER, &NEXT, &PREV] {
            assert_eq!(
                spec.requires,
                Caps::empty(),
                "{} does different work in each popup state, which is a \
                 branch and stays in the body",
                spec.id
            );
        }
        for spec in ACTIONS {
            assert!(!spec.host_invocable, "{}", spec.id);
            assert!(
                spec.id.starts_with("godotvim.completion."),
                "{} escapes this provider's namespace",
                spec.id
            );
        }
    }

    #[test]
    fn the_defaults_cover_every_key_the_old_table_matched() {
        // Seven rows for seven of the eight old literals. `<Esc>` is
        // deliberately absent: DISMISS consumes now, and a shipped `<Esc>`
        // rule would trap the user in Insert; the engine's own
        // `SetMode(Normal)` cancels the popup instead. `Backspace` is
        // deliberately absent too — it was never a routing decision, it is
        // the post-engine re-filter in `maybe_retrigger_completion`, which
        // runs AFTER the key was already handled and so has no binding to be.
        let lines: Vec<&str> = DEFAULTS.lines().filter(|l| !l.is_empty()).collect();
        assert_eq!(lines.len(), 7);
        for notation in ["<C-@>", "<C-n>", "<C-p>", "<Tab>", "<CR>", "<Up>", "<Down>"] {
            assert!(
                lines.iter().any(|l| l.contains(&format!(" {notation} "))),
                "{notation} is no longer bound"
            );
        }
        for line in lines {
            // Through the real parser, not `rsplit(' ')`: a row carrying
            // `require_selection=0` would otherwise make the split read the
            // parameter as the action id.
            let parsed = crate::config::panelmap::parse_panel_line(line);
            let Ok(Some(crate::config::panelmap::PanelLine::Map(map))) = parsed else {
                panic!("'{line}' is not a panelmap line: {parsed:?}");
            };
            let crate::config::panelmap::TargetSpec::Action(ref id) = map.target else {
                panic!("'{line}' does not target an action");
            };
            assert!(
                ACTIONS.iter().any(|s| s.id == id.as_str()),
                "'{id}' is not declared by this provider"
            );
        }
    }

    #[test]
    fn ctrl_space_is_spelled_the_way_the_runtime_produces_it() {
        // `translate_key` folds Ctrl+Space to Char('@') + CTRL. `<C-Space>`
        // also parses — to Char(' ') + CTRL — so the wrong spelling loads
        // cleanly and never fires. Pinned against the parser itself.
        use vim_core::keymap::{Key, KeyEvent, Modifiers};
        assert_eq!(
            crate::actions::keys::parse_lhs("<C-@>").expect("parses"),
            vec![KeyEvent::new(Key::Char('@'), Modifiers::CTRL)]
        );
        assert!(DEFAULTS.contains("<C-@>"));
        assert!(!DEFAULTS.contains("<C-Space>"));
    }
}
