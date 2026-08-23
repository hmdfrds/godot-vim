//! `editor.completion` — the autocomplete popup's keys, as named verbs.
//!
//! The second half of P9, and a different proof from `debugger.rs`: that one
//! shows a new *panel* costs one file, this one shows a hardcoded key table
//! deep inside the editor pipeline can become data without moving transports.
//!
//! # This surface is never classified, and that is the whole design
//!
//! Every other surface is reached by probing a sampled [`FocusChain`]. This one
//! cannot be, for a reason that is structural rather than incidental: whether
//! the completion popup is visible is a **per-keystroke** fact, and the focus
//! chain is sampled once per *focus change* and cached against
//! `(focus owner, epoch, index generation)`. A probe reading popup visibility
//! would be answering from a cache that is stale by construction.
//!
//! So [`EDITOR_COMPLETION`] declares `probe: |_| None` and is reached by name,
//! by the one transport that has the popup in hand:
//! `GodotVimCore::handle_gui_input_impl` looks the key up on this surface
//! directly, after the IME guard and before the vim engine sees anything. It is
//! the same "reached without probing" arrangement `panel` has — `panel` is
//! reached by following parent links, this one by an explicit lookup — which is
//! why both are excluded from the golden-table coverage audit.
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
//! - `_input` has **no way to express `Some(false)`** — "we made the routing
//!   decision, the engine must not see this key, and the event must NOT be
//!   consumed" — which is precisely what Up/Down need so `CodeEdit` moves its
//!   own popup selection. `Outcome` has three states and none of them is that
//!   one; [`CompletionOps::hand_to_editor`] is.
//!
//! # What the user gets
//!
//! Eight keys that were literals in a `match` are now rows in `:panelmap`,
//! rebindable and unmappable like every other binding:
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
use crate::actions::caps::Caps;
use crate::actions::outcome::Outcome;
use crate::actions::surface::{Seal, SurfaceSpec};

use super::Provider;

/// The surface the `gui_input` transport looks up by name.
pub(crate) const SURFACE: crate::actions::surface::SurfaceId = "editor.completion";

pub(crate) static EDITOR_COMPLETION: SurfaceSpec = SurfaceSpec {
    id: SURFACE,
    // A root. Naming `editor.nav` as parent would be wrong twice over: the
    // popup is live in Insert, where the active surface is `editor.insert`
    // (a `Barrier`), and this surface is never on a classified path at all, so
    // a parent link would only invite an upward walk that cannot happen.
    parent: None,
    // Inert, and documented as inert: `Seal` decides how the forest WALK
    // terminates, and nothing ever walks here. `Open` rather than `Barrier`
    // because `BindingIndex::try_insert` refuses rules on a `Barrier`, and
    // rules are the entire point of this surface.
    seal: Seal::Open,
    grants: |_| Caps::empty(),
    // Never. See the module docs: popup visibility is per-keystroke and the
    // chain is per-focus-change, so a probe here would read a stale cache.
    probe: |_| None,
    on_key: None,
    refuses_positional: false,
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

pub(crate) static CONFIRM: ActionSpec = ActionSpec {
    id: "godotvim.completion.confirm",
    desc: "Completion: accept the selected candidate",
    requires: Caps::empty(),
    host_invocable: false,
    default_consume: None,
    run: |cx| {
        let Some(ops) = ops(cx) else {
            return Outcome::Declined;
        };
        if !ops.popup_visible() {
            // THE load-bearing declination of this file. With no popup up,
            // `<CR>` must reach the engine and insert a newline and `<Tab>`
            // must indent. Consuming here would make Enter stop working in
            // insert mode, which is as bad as this plugin gets.
            return Outcome::Declined;
        }
        ops.confirm();
        Outcome::Handled
    },
};

pub(crate) static DISMISS: ActionSpec = ActionSpec {
    id: "godotvim.completion.dismiss",
    desc: "Completion: close the popup, letting the key through",
    requires: Caps::empty(),
    host_invocable: false,
    default_consume: None,
    run: |cx| {
        let Some(ops) = ops(cx) else {
            return Outcome::Declined;
        };
        if !ops.popup_visible() {
            return Outcome::Declined;
        }
        ops.cancel();
        // Declines ON PURPOSE, having already acted. `<Esc>` must close the
        // popup *and* leave insert mode in one press — two effects from one
        // key — so the popup is cancelled here and the keystroke still travels
        // on to the engine. A `Handled` would trap the user in insert mode
        // with the popup gone, needing a second Esc.
        Outcome::Declined
    },
};

pub(crate) static NAVIGATE: ActionSpec = ActionSpec {
    id: "godotvim.completion.navigate",
    desc: "Completion: let the editor's own popup handling move the selection",
    requires: Caps::empty(),
    host_invocable: false,
    default_consume: None,
    run: |cx| {
        let Some(ops) = ops(cx) else {
            return Outcome::Declined;
        };
        if !ops.popup_visible() {
            // No popup: Up/Down are ordinary cursor movement and belong to the
            // engine.
            return Outcome::Declined;
        }
        // The `Some(false)` leg. `CodeEdit::_gui_input` moves the popup
        // selection on Up/Down by itself and does it better than we would
        // (it handles scrolling and page bounds), so the routing decision is
        // "skip the engine, do not consume, let the control have it".
        ops.hand_to_editor();
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
/// Every rule is elastic: a verb that declines does not consume, and the key
/// continues to the vim engine. That is what keeps `<CR>` inserting a newline
/// and `<Tab>` indenting when no popup is up, without a single mode check in
/// the binding table.
const DEFAULTS: &str = "\
panelmap editor.completion <C-@> godotvim.completion.trigger
panelmap editor.completion <C-n> godotvim.completion.next
panelmap editor.completion <C-p> godotvim.completion.prev
panelmap editor.completion <Tab> godotvim.completion.confirm
panelmap editor.completion <CR> godotvim.completion.confirm
panelmap editor.completion <Esc> godotvim.completion.dismiss
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
        handed_off: bool,
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
        fn hand_to_editor(&mut self) {
            self.handed_off = true;
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

    /// The verdict the transport computes: `Some(true)` consume,
    /// `Some(false)` hand to the control, `None` let the engine have it.
    ///
    /// Duplicated from `controller::completion::verdict` on purpose — this is
    /// the assertion, and asserting through the implementation would make it
    /// a tautology.
    fn verdict(spec: &ActionSpec, popup: &mut FakePopup) -> Option<bool> {
        let outcome = run(spec, Params::new(), popup);
        if popup.handed_off {
            Some(false)
        } else {
            outcome.is_consumed().then_some(true)
        }
    }

    // ── The trichotomy, one row per shipped match arm ────────────────

    #[test]
    fn trigger_opens_the_popup_and_consumes() {
        let mut popup = FakePopup::closed();
        popup.options = 3;
        assert_eq!(verdict(&TRIGGER, &mut popup), Some(true));
        assert_eq!(popup.log, vec!["request(force=true)"]);
        assert!(popup.visible);
    }

    #[test]
    fn trigger_declines_when_completion_is_disabled() {
        // `editor.is_code_completion_enabled()` false → the old code returned
        // `None` and the chord reached the engine. Same here, via declination.
        let mut popup = FakePopup::disabled();
        assert_eq!(verdict(&TRIGGER, &mut popup), None);
        assert!(popup.log.is_empty(), "must not force a disabled popup");
    }

    #[test]
    fn next_opens_a_closed_popup_rather_than_moving_nothing() {
        let mut popup = FakePopup::closed();
        popup.options = 4;
        assert_eq!(verdict(&NEXT, &mut popup), Some(true));
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
        assert_eq!(verdict(&PREV, &mut popup), Some(true));
        assert_eq!(popup.log, vec!["request(force=true)", "select(3)"]);
        assert_eq!(popup.selected, 3);
    }

    #[test]
    fn next_and_prev_move_and_wrap_on_a_visible_popup() {
        for (spec, from, want) in [(&NEXT, 0, 1), (&NEXT, 2, 0), (&PREV, 1, 0), (&PREV, 0, 2)] {
            let mut popup = FakePopup::open(3, from);
            assert_eq!(verdict(spec, &mut popup), Some(true), "{}", spec.id);
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
            assert_eq!(verdict(spec, &mut popup), None, "{}", spec.id);
            assert!(popup.log.is_empty());
        }
    }

    #[test]
    fn confirm_accepts_a_visible_popup_and_consumes() {
        let mut popup = FakePopup::open(2, 1);
        assert_eq!(verdict(&CONFIRM, &mut popup), Some(true));
        assert_eq!(popup.log, vec!["confirm"]);
    }

    #[test]
    fn confirm_declines_with_no_popup_so_enter_still_inserts_a_newline() {
        // The regression that would be reported as "Enter stopped working".
        let mut popup = FakePopup::closed();
        assert_eq!(verdict(&CONFIRM, &mut popup), None);
        assert!(popup.log.is_empty());
    }

    #[test]
    fn dismiss_cancels_the_popup_and_still_lets_escape_reach_the_engine() {
        // Two effects, one key: the popup closes AND insert mode exits. This
        // is the one verb that acts and declines in the same breath, and it is
        // the reason `Outcome::Declined` had to keep meaning "the engine gets
        // it" rather than "nothing happened".
        let mut popup = FakePopup::open(3, 1);
        assert_eq!(verdict(&DISMISS, &mut popup), None);
        assert_eq!(popup.log, vec!["cancel"]);
        assert!(!popup.visible);
    }

    #[test]
    fn dismiss_with_no_popup_does_nothing_at_all() {
        let mut popup = FakePopup::closed();
        assert_eq!(verdict(&DISMISS, &mut popup), None);
        assert!(popup.log.is_empty());
    }

    #[test]
    fn navigate_hands_the_key_to_the_control_without_consuming_it() {
        // THE `Some(false)` case, and the whole reason this could not move to
        // the `_input` registry: "handled by us, engine skipped, event NOT
        // marked handled" is not expressible as an `Outcome`.
        let mut popup = FakePopup::open(3, 0);
        assert_eq!(verdict(&NAVIGATE, &mut popup), Some(false));
        assert!(popup.handed_off);
        assert!(popup.log.is_empty(), "the control does the moving, not us");
        assert_eq!(
            popup.selected, 0,
            "we must not move the selection ourselves"
        );
    }

    #[test]
    fn navigate_declines_with_no_popup_so_arrows_move_the_caret() {
        let mut popup = FakePopup::closed();
        assert_eq!(verdict(&NAVIGATE, &mut popup), None);
        assert!(!popup.handed_off);
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
    fn the_surface_never_probes_and_is_therefore_never_classified() {
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
    }

    #[test]
    fn no_verb_requires_a_capability() {
        // Capabilities are sampled from the focus chain, and this surface has
        // none. A `requires` bit here would gate every completion key off
        // permanently — silently, since the gate is a declination.
        for spec in ACTIONS {
            assert_eq!(spec.requires, Caps::empty(), "{}", spec.id);
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
        // Eight rows for eight literals: <C-@>, <C-n>, <C-p>, Tab, Enter,
        // Escape, Up, Down. `Backspace` is deliberately absent — it was never
        // a routing decision, it is the post-engine re-filter in
        // `maybe_retrigger_completion`, which runs AFTER the key was already
        // handled and so has no binding to be.
        let lines: Vec<&str> = DEFAULTS.lines().filter(|l| !l.is_empty()).collect();
        assert_eq!(lines.len(), 8);
        for notation in [
            "<C-@>", "<C-n>", "<C-p>", "<Tab>", "<CR>", "<Esc>", "<Up>", "<Down>",
        ] {
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
