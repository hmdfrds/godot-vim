//! Input handler implementations for the two Godot entry points: global
//! `input()` (cross-panel/dock navigation) and per-editor `gui_input()`
//! (keystroke processing through the Vim engine).

// Promote #[must_use] warnings to errors so that dropping an EngineOutcome
// without calling .apply_ui_update() or .discard() is a compile-time error.
#![deny(unused_must_use)]

use godot::classes::{
    CodeEdit, Control, EditorInterface, InputEvent, InputEventKey, ItemList, Tree, Viewport,
};
use godot::global::Key;
use godot::prelude::*;
use vim_core::keymap::KeyEvent;

use crate::actions::action::{ActionCtx, ActionSpec, Params};
use crate::actions::outcome::Outcome;
use crate::actions::resolve::{
    self, Candidate, CandidateTarget, Disposition, Resolution, ResolveInput,
};
use crate::actions::sequence::SeqStep;
use crate::actions::surface::{FocusChain, Seal, SurfacePath};
use crate::bridge;
use crate::controller::VimController;
use crate::ui::UiCoordinator;

use super::outcome::EngineOutcome;
use super::processing_guard::ProcessingKeyGuard;
use super::{GodotVimCore, SearchSuppression};

/// What the transport must do with the keystroke, once the sequence layer and
/// the resolver have both had their say.
///
/// A separate enum rather than a direct call because every arm needs `&mut
/// self` (the timer, the buffer) while the decision itself is taken with
/// `&self.bindings` and `&self.actions` borrowed. Naming the decision lets the
/// borrows end before the side effects begin.
enum Plan {
    /// Nothing runs and nothing is consumed: Godot's own handling proceeds.
    Drop,
    /// Consume, and touch neither the timer nor the buffer. The echo arm, and
    /// half of what stops a held prefix key from keeping the buffer alive
    /// forever: it does not restart the timer. The other half is in
    /// [`crate::actions::sequence::Pending::step`], which refuses to
    /// *open* a buffer on an echo — without that, a timeout that clears the
    /// buffer simply lets the next echo start a new one.
    Swallow,
    /// Consume and (re)arm the shell timer: a prefix is pending.
    Arm,
    /// Consume, and clear the buffer and the timer. The dead-prefix arm.
    Clear,
    /// Run this plan through the ordinary consumption fold.
    Run(Vec<Candidate>, KeyEvent),
}

/// What a [`Plan`] does to the shell timer.
///
/// Named rather than inlined for the same reason `Plan` itself is: the timer
/// is `&mut self` state, so the decision and the effect want to be separable —
/// and the decision half is then testable without a `Gd<T>`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TimerAction {
    /// Touch neither the timer nor the buffer. The echo arm's other half:
    /// a held prefix key must not keep restarting its own timeout.
    Leave,
    /// (Re)arm the shell timer — a prefix is pending.
    Arm,
    /// Stop the timer, so a stale timeout cannot fire into whatever runs next.
    Stop,
}

/// The whole consumption-and-timer contract of the five plans, as data.
///
/// The `Disposition` is what the **plan itself** decides, before any action
/// runs. `Plan::Run` therefore answers `Ignore`: it consumes nothing on its
/// own, and `resolve::dispose` — which is where `<void>`, `<norepeat>` and the
/// decline-and-fall-through rule live — has the say. Every other arm is
/// terminal, so its answer here is final.
fn plan_effects(plan: &Plan) -> (Disposition, TimerAction) {
    match plan {
        // Nothing matched. Godot's own handling proceeds, and a pending
        // buffer (if any) keeps waiting: a key that resolved to nothing is
        // not evidence that the prefix was abandoned.
        Plan::Drop => (Disposition::Ignore, TimerAction::Leave),
        Plan::Swallow => (Disposition::Consume, TimerAction::Leave),
        Plan::Arm => (Disposition::Consume, TimerAction::Arm),
        // Consumed on purpose. A reserved prefix owns its whole subtree:
        // letting the terminating key through would send it to Tree
        // incremental search after the prefix has already been destroyed.
        Plan::Clear => (Disposition::Consume, TimerAction::Stop),
        // A sequence that completed has already cleared its buffer; stopping
        // the timer is what keeps a stale timeout from firing into the action
        // that just ran.
        Plan::Run(_, _) => (Disposition::Ignore, TimerAction::Stop),
    }
}

/// Perform a disposition on the primary transport.
///
/// An exhaustive match, not `== Disposition::Consume`, so a future variant is
/// a compile error at the two places that commit rather than silently taking
/// the not-consumed path.
fn commit(viewport: &mut Gd<Viewport>, disposition: Disposition) {
    match disposition {
        Disposition::Consume => viewport.set_input_as_handled(),
        Disposition::Ignore => {}
        // Unreachable here by audit A9': the only verb that can produce it
        // requires `Caps::POPUP`, which no classified path grants. Logged
        // rather than folded silently into "not consumed", because the day it
        // fires is the day someone weakened the audit. NOT a panic: a panic
        // across the FFI boundary in an input handler is fatal (see ec417b3).
        Disposition::Handoff => log::error!(
            "input: Disposition::Handoff on the primary transport; audit A9' was weakened"
        ),
    }
}

/// Build the cache key from the five facts a re-sample depends on.
///
/// A free function so the field-by-field copy is reachable from a test: every
/// caller sits behind a `Gd<Viewport>`, so before this existed a component
/// that stopped being read — `mode: None` being the dangerous one — could not
/// be caught by anything but review. See
/// `every_chain_key_component_moves_the_key`.
fn chain_key(
    focus: Option<InstanceId>,
    attached: Option<InstanceId>,
    mode: Option<vim_core::primitives::Mode>,
    prompt: Option<InstanceId>,
    generation: u64,
) -> ChainKey {
    ChainKey {
        focus,
        attached,
        mode,
        prompt,
        generation,
    }
}

/// The sampled chain, and the state it was sampled against.
///
/// `FocusChain::sample` walks the focus owner's ancestors and asks each one
/// six `is_class` questions, then runs an unbounded `is_ancestor_of` and, for
/// a `LineEdit`, a depth-20 sibling DFS. That is far too much to do at the OS
/// key-repeat rate, so it is done once per *distinct* key below and reused.
///
/// Every field of the key is a fact a probe reads. `editor_mode` is in there
/// for a reason worth stating: a mode change does **not** change the focus
/// owner, so keying on focus alone would leave a stale `Normal` behind when
/// the user typed `i` — and `editor.insert` would stop being a barrier while
/// Ctrl+H is backspace. Caching the expensive half and re-stamping the cheap
/// half would work equally well; keying on all of it is simpler and cannot
/// silently miss a field.
#[derive(Debug)]
pub(super) struct ChainCache {
    key: ChainKey,
    chain: FocusChain,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ChainKey {
    focus: Option<InstanceId>,
    attached: Option<InstanceId>,
    mode: Option<vim_core::primitives::Mode>,
    prompt: Option<InstanceId>,
    /// Bumped whenever the binding index is rebuilt, which is what makes a
    /// config reload invalidate the cache.
    generation: u64,
}

impl GodotVimCore {
    /// Global `input()` handler (Godot stage 1 — fires before `gui_input`).
    ///
    /// The staged model of `DESIGN-rebindable-nav.md` (design notes, kept outside this repo) §5, in order:
    /// transport guards → surface sampling → barrier → probes → per-surface
    /// hooks → resolution → arbitration → execution → consumption.
    ///
    /// Nothing here knows what a dock is, which widget classes navigate, or
    /// which key does what. It samples where the keystroke is, asks the
    /// binding plane what that means, runs the answer, and applies the
    /// answer's declared consumption policy. Adding a panel touches this
    /// function zero times.
    ///
    /// # Ordering note
    ///
    /// The design lists key identity (S1) before sampling (S2); this samples
    /// first and decodes after the barrier check. The two are observably
    /// identical — building a probe list has no side effects — and the swap
    /// buys back the early return the old dispatcher had: typing in Insert
    /// mode or inside a foreign `LineEdit` hits a `Barrier` and never runs
    /// `translate_key` a second time at the OS repeat rate.
    pub(super) fn handle_input_impl(&mut self, event: Gd<InputEvent>) {
        // ── S0: transport guards, unchanged ──────────────────────────
        if !self.enabled {
            return;
        }
        let Ok(key_event) = event.try_cast::<InputEventKey>() else {
            return;
        };
        if !key_event.is_pressed() {
            return;
        }
        if matches!(
            key_event.get_keycode(),
            Key::SHIFT
                | Key::CTRL
                | Key::ALT
                | Key::META
                | Key::CAPSLOCK
                | Key::NUMLOCK
                | Key::SCROLLLOCK
        ) {
            return;
        }

        let Some(base_control) = EditorInterface::singleton().get_base_control() else {
            return;
        };
        let Some(mut viewport) = base_control.get_viewport() else {
            return;
        };

        // Stale-editor self-heal. Phase 1 holds only an immutable borrow of
        // `attached_editor`; phase 2 releases it before taking `&mut self`.
        let stale = matches!(self.attached_editor.as_ref(), Some(e) if !e.is_instance_valid());
        if stale {
            // Deref-free: drop the stale handle. `detach()` is self-completing.
            self.detach();
            self.last_editor_id = None;
        }

        // ── S2: sample the chain (cached) and classify it ────────────
        let Some(path) = self.surface_path(&viewport) else {
            // Unreachable with the shipped forest — `unknown` probes
            // unconditionally — but a third-party forest with no total probe
            // must not consume the key.
            log::error!("input: no surface claimed the focus chain");
            return;
        };

        // ── S2.5: scope `allow_search` to this focus owner ───────────
        // Before the barrier return, so that moving focus INTO a foreign text
        // input still restores the Tree we left behind.
        self.sync_search_suppression(&path, &viewport);

        // ── S3: a barrier is a total hard stop, before any hook ──────
        if path.seal == Seal::Barrier {
            return;
        }

        // ── S1: one key vocabulary for the whole shell-side surface ──
        let probes = crate::actions::keys::probes(&key_event, self.langmap.as_ref());
        if probes.is_empty() {
            return;
        }
        let is_echo = key_event.is_echo();

        // ── S4: per-surface hooks, before any lookup ─────────────────
        self.run_surface_hooks(&path, &viewport);

        // ── S5/S6: sequences first, then resolve and arbitrate ───────
        //
        // The pending layer runs BEFORE the single-key walk because a reserved
        // key must never reach it: `d` on a surface that binds `dd` is a
        // prefix, not a delete. It returns `Passthrough` — having created no
        // state and consumed nothing — for every key the user did not reserve,
        // which is every key in the shipped keyset.
        let plan = {
            let controller = self.controller.as_ref();
            // The polarity flip lives in `engine_claims`, written down once
            // and unit-tested there: no controller must mean INTERCEPT.
            let claims = |k: KeyEvent| {
                resolve::engine_claims(controller, k, VimController::could_start_mapping)
            };
            let input = ResolveInput {
                probes: &probes,
                path: &path,
                index: &self.bindings,
                registry: &self.actions,
                vim_claims: &claims,
            };
            match self.pending.step(&input, is_echo) {
                SeqStep::Echo => Plan::Swallow,
                SeqStep::Buffered => Plan::Arm,
                SeqStep::DeadPrefix(surface) => {
                    // Consumed on purpose. A reserved prefix owns its whole
                    // subtree: letting the terminating key through would send
                    // it to Tree incremental search after the prefix has
                    // already been destroyed, which is worse than eating it.
                    log::debug!("input: dead prefix on {surface}");
                    Plan::Clear
                }
                SeqStep::Run(candidates, matched) => Plan::Run(candidates, matched),
                SeqStep::Passthrough => match resolve::resolve(&input) {
                    Resolution::Run {
                        matched,
                        candidates,
                    } => Plan::Run(candidates, matched),
                    Resolution::None(_) => Plan::Drop,
                },
            }
        };

        // The decision is `plan_effects`; this match only performs it. Both
        // effects are idempotent and act on unrelated objects, so their order
        // relative to each other is not observable.
        let (immediate, timer) = plan_effects(&plan);
        match timer {
            TimerAction::Leave => {}
            TimerAction::Arm => self.arm_panel_timer(),
            TimerAction::Stop => self.stop_panel_timer(),
        }
        let (candidates, matched) = match plan {
            Plan::Run(candidates, matched) => (candidates, matched),
            // Every other arm is terminal: `plan_effects` has already said
            // everything there is to say about the keystroke.
            Plan::Drop | Plan::Swallow | Plan::Arm | Plan::Clear => {
                commit(&mut viewport, immediate);
                return;
            }
        };

        // ── S7/S8: run the plan, then apply its consumption policy ───
        let target = viewport
            .gui_get_focus_owner()
            .map(godot::obj::Gd::upcast::<Control>);
        let disposition = resolve::dispose(&candidates, is_echo, |candidate| {
            self.run_candidate(candidate, target.clone())
        });

        // ── S9: commit on THIS transport's viewport ──────────────────
        if disposition == Disposition::Consume {
            log::trace!(
                "input: consumed {matched} via {}",
                candidates.first().map_or("<none>", |c| c.surface)
            );
        }
        commit(&mut viewport, disposition);
    }

    /// §5.10 step 3 — `Tree`/`ItemList` incremental type-to-search, off for
    /// exactly as long as the focused control's surface reserves a bare prefix.
    ///
    /// Godot's `Tree` type-searches on bare printable keys. A reserved `g`
    /// would otherwise do both jobs at once: buffer in the shell plane and
    /// start a type-search in the control. Removing the conflict at its source
    /// beats racing it.
    ///
    /// Three things this deliberately does **not** do. It does not touch a
    /// control whose surface stack reserves nothing — which is every control
    /// in the shipped zero-config keyset, so a user who never bound a sequence
    /// never loses type-to-search. It does not assume the previous value:
    /// `previous` is read back and restored verbatim, because the editor (or
    /// the user) may have had search off already. And it suppresses exactly
    /// one control at a time, so moving focus restores the last one.
    fn sync_search_suppression(&mut self, path: &SurfacePath, viewport: &Gd<Viewport>) {
        let wanted = crate::actions::sequence::path_reserves(&self.bindings, path)
            .and_then(|_| viewport.gui_get_focus_owner())
            .map(|owner| owner.instance_id());
        if self.search_suppression.as_ref().map(|s| s.control) == wanted {
            // The overwhelmingly common case, and the only one on the hot
            // path for a user with no sequences bound: `None == None`.
            return;
        }
        self.restore_search_suppression();
        let Some(control) = wanted
            .and_then(|id| Gd::<Control>::try_from_instance_id(id).ok())
            .filter(godot::obj::Gd::is_instance_valid)
        else {
            return;
        };
        // `None` for a focus owner that is neither a `Tree` nor an `ItemList`
        // — a Button, a RichTextLabel. Nothing is recorded, so this retries
        // next keystroke; that costs two failed casts per key and only while
        // a reservation is live.
        let Some(previous) = set_allow_search(&control, false) else {
            return;
        };
        log::debug!(
            "sequence: suppressed type-to-search on {} (was {previous})",
            control.get_class()
        );
        self.search_suppression = Some(SearchSuppression {
            control: control.instance_id(),
            previous,
        });
    }

    /// Give back whatever `sync_search_suppression` took.
    ///
    /// Called on focus change, on teardown, on plugin disable and inside the
    /// `panic_guard` recovery path. `allow_search` lives on a control that
    /// outlives the plugin, so a missed restore is a permanent, unattributable
    /// regression in the user's editor session.
    pub(super) fn restore_search_suppression(&mut self) {
        let Some(state) = self.search_suppression.take() else {
            return;
        };
        let Ok(control) = Gd::<Control>::try_from_instance_id(state.control) else {
            // Freed. Nothing to restore and nothing leaked — the flag lived on
            // the object that is gone.
            return;
        };
        if !control.is_instance_valid() {
            return;
        }
        set_allow_search(&control, state.previous);
    }

    /// Fired by the shell timer after `timeoutlen` ms with a prefix pending.
    ///
    /// Two outcomes only. An exact match at the buffered prefix runs — that is
    /// what makes `g` reachable at all when `gg` also exists. Otherwise the
    /// buffer is dropped: there is **no replay channel** in Godot's `_input()`
    /// stage, so the keys cannot be flushed back as literals the way Vim's
    /// timeout does. `set_input_as_handled()` destroyed them, and re-injecting
    /// through `Input::parse_input_event` would re-dispatch them in a later
    /// frame against a focus owner that may have changed — a different and
    /// unpredictable action, not a more faithful one.
    pub(super) fn on_panel_timeout_impl(&mut self) {
        let Some((candidates, matched)) = self.pending.on_timeout(&self.bindings, &self.actions)
        else {
            log::debug!("sequence: timeout flushed an incomplete prefix");
            return;
        };
        let target = EditorInterface::singleton()
            .get_base_control()
            .and_then(|c| c.get_viewport())
            .and_then(|vp| vp.gui_get_focus_owner())
            .map(godot::obj::Gd::upcast::<Control>);
        // There is no keystroke left to consume, so the disposition is
        // irrelevant — but `dispose` is still the path, because `<void>` and
        // the decline-and-fall-through rule must mean the same thing here as
        // they do on a keystroke.
        let disposition = resolve::dispose(&candidates, false, |candidate| {
            self.run_candidate(candidate, target.clone())
        });
        log::trace!("sequence: timeout resolved {matched} -> {disposition:?}");
    }

    /// Sample the focus chain (cached) and classify it into a surface path.
    ///
    /// Classifies against the index's OWN forest rather than a fresh
    /// `providers::forest()`: that constructor allocates a `Vec` of every
    /// declared surface, and this runs at the OS key-repeat rate. Reading the
    /// index's copy also guarantees the classification and the trie lookups
    /// that follow agree about what the forest is.
    fn surface_path(&mut self, viewport: &Gd<Viewport>) -> Option<SurfacePath> {
        self.refresh_focus_chain(viewport);
        let chain = &self.chain_cache.as_ref()?.chain;
        self.bindings.forest().classify(chain)
    }

    /// Re-sample the chain if anything a probe reads has changed.
    fn refresh_focus_chain(&mut self, viewport: &Gd<Viewport>) {
        let key = chain_key(
            viewport
                .gui_get_focus_owner()
                .map(|owner| owner.instance_id()),
            self.attached_editor
                .as_ref()
                .filter(|e| e.is_instance_valid())
                .map(Gd::instance_id),
            self.controller.as_ref().map(VimController::mode),
            self.fs_explorer.prompt_instance(),
            self.bindings.generation,
        );
        if self.chain_cache.as_ref().is_none_or(|c| c.key != key) {
            log::trace!("input: resampling focus chain");
            // §5.10 — `pending` clears on focus-owner change and on config
            // reload, and this key moves on both (`generation` is bumped by
            // every index rebuild). A buffer that survived a focus change
            // would resolve a `g` typed in the FileSystem dock against
            // whatever has focus now; one that survived a reload would resolve
            // against an index its surface may no longer exist in.
            if self.pending.is_active() {
                log::debug!("sequence: focus or index changed; dropping the pending prefix");
                self.pending.clear();
                self.stop_panel_timer();
            }
            let chain = FocusChain::sample(viewport, key.attached, key.mode, key.prompt);
            self.chain_cache = Some(ChainCache { key, chain });
        } else if let Some(cached) = self.chain_cache.as_ref() {
            // A hit whose chain disagrees with its own key means the key and
            // the sampler read different things — the exact desync that would
            // leave `editor.insert` classified as `editor.nav`. Cheap enough
            // to assert on every keystroke in a debug build.
            debug_assert_eq!(cached.chain.attached_editor, key.attached);
            debug_assert_eq!(cached.chain.editor_mode, key.mode);
        }
    }

    /// Drop the cached chain and the pending prefix.
    ///
    /// It has exactly one caller — `rebuild_bindings` — and that is the whole
    /// story rather than an omission. Every other fact a probe reads is
    /// already a `ChainKey` component (focus owner, attached editor, mode,
    /// prompt instance, index generation), so the cache invalidates itself on
    /// the next keystroke and nothing has to remember to call this. Read the
    /// key, not this function, when asking what can go stale.
    ///
    /// What `rebuild_bindings` needs beyond the key is the pending prefix: the
    /// `generation` bump already moves the key, but a multi-key buffer opened
    /// against the old index must not survive into the new one, and clearing
    /// it is not something re-sampling would do.
    pub(super) fn invalidate_focus_chain(&mut self) {
        self.chain_cache = None;
        self.pending.clear();
        self.stop_panel_timer();
    }

    /// S4 — run every surface's `on_key` hook, deepest first.
    ///
    /// Before any lookup and regardless of whether a binding matches, because
    /// the one shipped hook belongs to no binding: it is the FileSystem
    /// dock's stale-prompt auto-dismiss, which used to run at the top of
    /// `handle_key` for every key including modified ones.
    fn run_surface_hooks(&mut self, path: &SurfacePath, viewport: &Gd<Viewport>) {
        // Collected first: the forest lookup borrows `self.bindings` and the
        // hook takes `&mut self.fs_explorer`. `ArrayVec`-free because the
        // shipped forest declares exactly one hook and the common case
        // allocates nothing at all — `Vec::new()` on the empty path.
        let hooks: Vec<fn(&mut ActionCtx<'_>)> = path
            .ids
            .iter()
            .filter_map(|id| self.bindings.forest().get(id).and_then(|spec| spec.on_key))
            .collect();
        if hooks.is_empty() {
            return;
        }
        let target = viewport
            .gui_get_focus_owner()
            .map(godot::obj::Gd::upcast::<Control>);
        let mut cx = ActionCtx::new(target, Params::new()).with_fs(&mut self.fs_explorer);
        for hook in hooks {
            hook(&mut cx);
        }
    }

    /// S7 — run one candidate.
    ///
    /// The `&'static ActionSpec` is copied out of the resolution rather than
    /// re-read from the registry, so no registry borrow spans the call and
    /// the action is free to take `&mut` on the plugin's own fields.
    fn run_candidate(&mut self, candidate: &Candidate, target: Option<Gd<Control>>) -> Outcome {
        match &candidate.target {
            CandidateTarget::Action(_, spec) => {
                let mut cx =
                    ActionCtx::new(target, candidate.params.clone()).with_fs(&mut self.fs_explorer);
                let outcome = (spec.run)(&mut cx);
                log::trace!("input: {} -> {outcome:?}", spec.id);
                outcome
            }
            CandidateTarget::Shortcut(path) => {
                // Deliberately not implemented at this phase. Delegating to
                // one of Godot's own shortcuts means re-injecting an event
                // into the same `_input` flush that dispatched it, and
                // `Input::parse_input_event` appends to the list
                // `flush_buffered_events` is draining — an unguarded
                // delegation is a hard editor hang, not a slow loop. The
                // registration-time cycle audit, the injection fingerprint
                // and the per-frame budget that make it safe are a phase of
                // their own; until then the rule declines and the key
                // reaches Godot, which is the failure direction that loses
                // nothing. No shipped default uses this target.
                log::warn!("input: <Shortcut>({path}) targets are not dispatched yet");
                Outcome::Declined
            }
        }
    }

    /// Resolve a keystroke on the `editor.completion` surface (P9).
    ///
    /// This is the whole of "completion routing became rebindable". It is a
    /// **direct lookup by surface name**, not a forest walk: `editor.completion`
    /// declares `probe: |_| None` because popup visibility is a per-keystroke
    /// fact while the sampled `FocusChain` is cached per focus change, so a
    /// probe here would answer from a cache that is stale by construction.
    ///
    /// Three things a walked surface would get and this one deliberately does
    /// not, all inert rather than wrong:
    ///
    /// - **`<physical>`** — only probe 1 (the canonicalized logical key) is
    ///   offered. A positional guess inside the attached editor is what
    ///   `refuses_positional` exists to forbid; honouring it here would turn a
    ///   Dvorak `Ctrl+p` into a completion key.
    /// - **`<void>` / `<norepeat>`** — the verdict is the action's own
    ///   `Outcome`, i.e. always elastic. That is not a shortcut: consuming
    ///   `<CR>` when no popup is up would stop Enter inserting a newline, so
    ///   `Void` has no correct meaning on this surface.
    /// - **multi-key sequences** — rejected at registration by V8, since
    ///   `editor.completion` is editor-reachable.
    fn completion_binding(&self, key: vim_core::keymap::KeyEvent) -> Option<&'static ActionSpec> {
        let lhs = [crate::actions::keys::canonicalize(key)];
        let rule = self
            .bindings
            .rule_for(crate::actions::providers::completion::SURFACE, &lhs)?;
        match rule.target {
            crate::actions::action::RuleTarget::Action(id) => self
                .actions
                .get(id)
                // THE gate every walked surface gets at `resolve.rs`'s
                // `hit_from`, and that this transport structurally cannot: it
                // has no classified path, so it has no `Caps` to satisfy
                // anything with. Empty `requires` is therefore the only
                // requirement it can honour, and anything else must not run —
                // `process_cycle` calls `(spec.run)` with
                // `ActionCtx::new(None, …)`, and the ctx-free FS verbs never
                // read their ctx at all, so `godotvim.fs.delete` bound here
                // would delete a file from a keystroke typed in a script.
                //
                // `BindingIndex::try_insert` rejects such a rule at
                // registration; this is the second wall, because the cost of
                // the two disagreeing is a deleted file.
                .filter(|spec| spec.requires.is_empty()),
            // `native` and `<Shortcut>(…)` have no meaning against a popup:
            // both mean "not ours", which on this transport is exactly what
            // `None` already says.
            _ => None,
        }
    }

    /// Per-editor keystroke handler. Connected to `gui_input` on the attached CodeEdit.
    pub(super) fn handle_gui_input_impl(&mut self, event: Gd<InputEvent>) {
        let Some(editor) = &self.attached_editor else {
            return;
        };
        // has_focus() guards against deferred delivery edge cases where gui_input
        // arrives after focus has moved away.
        if !editor.is_instance_valid() || !editor.has_focus() {
            return;
        }

        // ── Mouse wheel interception ─────────────────────────────────────
        // Route mouse wheel through the vim engine as Ctrl-Y (scroll up) /
        let Ok(key_event) = event.try_cast::<InputEventKey>() else {
            return;
        };
        // Accept both press and echo (key-repeat) -- held-key repeat is
        // correct Vim semantics (e.g. holding `j` to scroll down).
        if !key_event.is_pressed() {
            return;
        }
        let Some(key) = bridge::input::parse_godot_key(&key_event) else {
            return;
        };

        // IME compose guard: when TextEdit is actively composing (CJK input,
        // dead keys, alt-code unicode), don't consume the key — let it flow
        // through to TextEdit's native IME handling. Guards text-input modes
        // (Insert/Replace/CommandLine) where IME composition is meaningful.
        //
        // Escape-class keys (Escape, Ctrl+C, Ctrl+[) force-cancel the IME
        // composition so the user can always exit — even if the IME framework
        // doesn't cancel on Escape by itself.
        if editor.has_ime_text() {
            if let Some(controller) = &self.controller {
                let mode = controller.mode();
                if matches!(
                    mode,
                    vim_core::primitives::Mode::Insert
                        | vim_core::primitives::Mode::Replace
                        | vim_core::primitives::Mode::VirtualReplace
                        | vim_core::primitives::Mode::CommandLine
                ) {
                    let is_escape_key = matches!(key.key(), vim_core::keymap::Key::Escape)
                        || key == vim_core::keymap::KeyEvent::ctrl('c')
                        || key == vim_core::keymap::KeyEvent::ctrl('[');
                    if is_escape_key {
                        log::debug!("gui_input: force-cancelling IME for escape key={}", key);
                        let mut ed = editor.clone();
                        ed.cancel_ime();
                        // Fall through — Escape reaches the engine
                    } else {
                        log::trace!(
                            "gui_input: IME compose active in {:?}, passing through key={}",
                            mode,
                            key
                        );
                        return;
                    }
                } else {
                    // Stale IME composition in a non-insert mode — cancel it.
                    // This shouldn't happen (deactivate_ime cancels on mode exit),
                    // but some platforms/IMEs can leave stale state.
                    log::debug!("gui_input: cancelling stale IME in {:?}", mode);
                    let mut ed = editor.clone();
                    ed.cancel_ime();
                    // Fall through — key reaches the engine normally
                }
            }
        }

        // Cancel any pending tooltip — a new keystroke supersedes the hover.
        // Done before cloning `editor` to avoid borrow conflicts with `&mut self`.
        self.cancel_pending_tooltip();

        // Re-borrow after the mutable cancel call.
        let Some(editor) = &self.attached_editor else {
            return;
        };
        let mut ed = editor.clone();

        // Resolved here and passed down, rather than looked up inside the
        // controller: the `BindingIndex` lives on the plugin, and the
        // controller holding a reference to it would be a second cache of the
        // index generation to keep honest. Deliberately AFTER the IME guard
        // above — a preedit must reach `TextEdit` untouched, and that guard is
        // one of the three reasons these keys never moved to `_input`.
        let completion_binding = self.completion_binding(key);

        let outcome = {
            let _guard = ProcessingKeyGuard::new(&mut self.processing_key);
            let Some(controller) = &mut self.controller else {
                return;
            };
            controller.process_cycle(key, &mut ed, completion_binding)
        };

        let snap = {
            let Some(controller) = &mut self.controller else {
                return;
            };
            controller.ui_snapshot(ed.instance_id())
        };

        let applied = EngineOutcome::with_snapshot(snap, outcome).apply_ui_update(
            &mut self.ui,
            &mut ed,
            &mut self.caret_reconciler,
        );

        log::trace!(
            "gui_input: key={} outcome={}",
            key,
            applied.pipeline.log_label()
        );
        if applied.pipeline.should_mark_handled() {
            if let Some(mut vp) = editor.get_viewport() {
                vp.set_input_as_handled();
            }
        }

        if let Some(controller) = &mut self.controller {
            for action in controller.take_pending_ui_actions() {
                self.handle_pending_ui_action(action);
            }
        }

        // Start/restart the mapping timer if keys are buffered, stop if not.
        if let Some(controller) = &self.controller {
            if controller.has_pending_mapping() {
                if let Some(timer) = &mut self.mapping_timer {
                    let timeout_sec = controller.timeoutlen() as f64 / 1000.0;
                    timer.set_wait_time(timeout_sec);
                    timer.start();
                    log::trace!(
                        "gui_input: mapping timer started ({}ms)",
                        controller.timeoutlen()
                    );
                }
            } else if let Some(timer) = &mut self.mapping_timer {
                timer.stop();
            }
        }
    }

    /// Fired by the mapping timer after `timeoutlen` ms without further input.
    /// Flushes buffered keys as literals (or expands an exact match).
    pub(super) fn on_mapping_timeout_impl(&mut self) {
        let Some(editor) = &self.attached_editor else {
            return;
        };
        if !editor.is_instance_valid() {
            return;
        }
        let mut ed = editor.clone();

        let had_operations = {
            let _guard = ProcessingKeyGuard::new(&mut self.processing_key);
            if let Some(controller) = &mut self.controller {
                controller.resolve_mapping_timeout(&mut ed);
                controller.operations_this_cycle() > 0
            } else {
                false
            }
        };

        let editor_id = ed.instance_id();
        if let Some(controller) = &mut self.controller {
            let snap = controller.ui_snapshot(editor_id);
            // Use EngineConsumed when operations happened so apply_ui_update
            // sets a caret expectation; Passthrough otherwise so it does not.
            let pipeline = if had_operations {
                // Dummy ProcessResult -- only may_have_moved_cursor() matters,
                // which is true for EngineConsumed regardless of the payload.
                crate::controller::PipelineOutcome::EngineConsumed(
                    vim_core::execution::host_api::ProcessResult {
                        consumed: true,
                        host_requests: Vec::new(),
                        deferred_actions: Vec::new(),
                    },
                )
            } else {
                crate::controller::PipelineOutcome::Passthrough
            };
            EngineOutcome::with_snapshot(snap, pipeline).apply_ui_update(
                &mut self.ui,
                &mut ed,
                &mut self.caret_reconciler,
            );
        }

        if let Some(controller) = &mut self.controller {
            for action in controller.take_pending_ui_actions() {
                self.handle_pending_ui_action(action);
            }
        }

        // Timeout resolution may produce new pending keys (e.g. partial
        // match of a longer mapping) -- restart the timer so they resolve.
        if let Some(controller) = &self.controller {
            if controller.has_pending_mapping() {
                if let Some(timer) = &mut self.mapping_timer {
                    let timeout_sec = controller.timeoutlen() as f64 / 1000.0;
                    timer.set_wait_time(timeout_sec);
                    timer.start();
                }
            }
        }
    }

    /// Route a mouse-wheel event through the vim engine.
    ///
    /// Feeds Ctrl-Y (scroll up) or Ctrl-E (scroll down) through the normal
    /// `process_cycle` pipeline, repeating 3 times to match Godot's default
    /// scroll speed. Consumes the event so Godot's native scroll doesn't fire.
    /// Reconcile external cursor/selection changes with Vim engine state.
    /// Connected DEFERRED to avoid re-entrancy during text edits.
    ///
    /// Four cases based on (has_selection, vim_mode):
    /// 1. Selection + Normal  -- mouse drag entered Visual
    /// 2. No selection + Normal -- mouse click; sync sticky column
    /// 3. No selection + Visual -- click deselected; exit Visual
    /// 4. Selection + Visual  -- mouse extending; update Visual extents
    pub(super) fn on_caret_changed_impl(&mut self) {
        // Read caret position and check reconciler BEFORE any mutable borrows.
        // Uses a block to drop the immutable borrow of attached_editor before
        // cancel_pending_tooltip borrows &mut self.
        let (line, col) = {
            let Some(editor) = &self.attached_editor else {
                return;
            };
            if !editor.is_instance_valid() {
                return;
            }
            (editor.get_caret_line(), editor.get_caret_column())
        };

        match self.caret_reconciler.check_and_consume(line, col) {
            super::caret_reconcile::CaretOrigin::VimDriven => return,
            super::caret_reconcile::CaretOrigin::External => {}
        }

        self.cancel_pending_tooltip();

        let Some(controller) = &mut self.controller else {
            return;
        };

        let Some(editor) = &self.attached_editor else {
            return;
        };
        if !editor.is_instance_valid() {
            return;
        }
        let mut ed = editor.clone();
        let has_selection = ed.has_selection();
        let mode = controller.mode();

        if has_selection && !mode.is_visual_or_select() {
            log::debug!("on_caret_changed: mouse selection detected (entering visual)");
            apply_mouse_selection(
                controller,
                &mut ed,
                &mut self.caret_reconciler,
                &mut self.ui,
            );
        } else if !has_selection && mode.is_visual_or_select() {
            log::debug!("on_caret_changed: click deselect, exiting visual mode");
            let editor_id = ed.instance_id();
            controller.exit_mode_via_pipeline(&mut ed);
            controller.cleanup_visual_artifacts(editor_id, &mut ed);

            let char_col = crate::bridge::codec::i32_to_usize(ed.get_caret_column());
            let line_text = ed.get_line(ed.get_caret_line()).to_string();
            let grapheme_col = crate::bridge::codec::char_col_to_grapheme_col(&line_text, char_col);
            controller.set_engine_sticky_column(grapheme_col);

            let snap = controller.ui_snapshot(editor_id);
            EngineOutcome::with_snapshot(snap, crate::controller::PipelineOutcome::Passthrough)
                .apply_ui_update(&mut self.ui, &mut ed, &mut self.caret_reconciler);
        } else if has_selection && mode.is_visual_or_select() {
            log::trace!("on_caret_changed: visual selection updated");
            apply_mouse_selection(
                controller,
                &mut ed,
                &mut self.caret_reconciler,
                &mut self.ui,
            );
        } else {
            let char_col = crate::bridge::codec::i32_to_usize(ed.get_caret_column());
            let line_text = ed.get_line(ed.get_caret_line()).to_string();
            let grapheme_col = crate::bridge::codec::char_col_to_grapheme_col(&line_text, char_col);
            controller.set_engine_sticky_column(grapheme_col);
        }
    }
}

/// Turn Godot's incremental type-to-search on or off, returning the previous
/// value.
///
/// `None` for a control that has no such setting — a Button, a
/// `RichTextLabel`. Both `Tree` and `ItemList` expose it, and both are
/// tried because both are `VNAV` surfaces in a dock.
fn set_allow_search(control: &Gd<Control>, allow: bool) -> Option<bool> {
    if let Ok(mut tree) = control.clone().try_cast::<Tree>() {
        let previous = tree.get_allow_search();
        tree.set_allow_search(allow);
        return Some(previous);
    }
    if let Ok(mut list) = control.clone().try_cast::<ItemList>() {
        let previous = list.get_allow_search();
        list.set_allow_search(allow);
        return Some(previous);
    }
    None
}

/// Translate Godot's selection extents into Vim anchor/head and forward to the
/// controller. Determines drag direction from caret position. Shared by Cases 1
/// (enter Visual) and 4 (extend Visual) in `on_caret_changed_impl`.
fn apply_mouse_selection(
    controller: &mut VimController,
    ed: &mut Gd<CodeEdit>,
    reconciler: &mut super::caret_reconcile::CaretReconciler,
    ui: &mut UiCoordinator,
) {
    let shape = detect_selection_shape(ed);

    let from_line = ed.get_selection_from_line();
    let from_col = ed.get_selection_from_column();
    let to_line = ed.get_selection_to_line();
    let to_col = ed.get_selection_to_column();

    // Godot puts the caret at the drag endpoint -- if caret is at the start
    // of the selection, the user dragged backward.
    let caret_line = ed.get_caret_line();
    let caret_col = ed.get_caret_column();
    let caret_at_start = caret_line == from_line && caret_col == from_col;

    let (anchor_line, anchor_col, head_line, head_col) = if caret_at_start {
        (to_line, to_col, from_line, from_col)
    } else {
        (from_line, from_col, to_line, to_col)
    };

    log::debug!(
        "apply_mouse_selection: shape={:?} anchor=({},{}) head=({},{})",
        shape,
        anchor_line,
        anchor_col,
        head_line,
        head_col
    );

    let did_change =
        controller.process_mouse_selection(ed, anchor_line, anchor_col, head_line, head_col, shape);

    if did_change {
        let editor_id = ed.instance_id();
        let snap = controller.ui_snapshot(editor_id);
        // Use EngineConsumed so apply_ui_update sets a caret expectation
        // (mouse selection moves the cursor from the engine's perspective).
        EngineOutcome::with_snapshot(
            snap,
            crate::controller::PipelineOutcome::EngineConsumed(
                vim_core::execution::host_api::ProcessResult {
                    consumed: true,
                    host_requests: Vec::new(),
                    deferred_actions: Vec::new(),
                },
            ),
        )
        .apply_ui_update(ui, ed, reconciler);
    }
}

/// Heuristic: Godot's triple-click produces a selection from col 0 to col 0
/// of the next line. Detecting this pattern lets us enter Visual Line mode
/// instead of Visual Char mode for triple-click selections.
fn detect_selection_shape(editor: &Gd<CodeEdit>) -> vim_core::primitives::SelectionShape {
    use vim_core::primitives::SelectionShape;

    let from_col = editor.get_selection_from_column();
    let to_line = editor.get_selection_to_line();
    let to_col = editor.get_selection_to_column();
    let from_line = editor.get_selection_from_line();

    if from_col == 0 && to_col == 0 && to_line > from_line {
        SelectionShape::Line
    } else {
        SelectionShape::Char
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::actions::bind::{Consumption, Repeat};
    use crate::actions::surface::{ChainNode, FocusChain};
    use vim_core::keymap::{Key as VimKey, Modifiers};
    use vim_core::primitives::Mode;

    // ── What is NOT decidable here ───────────────────────────────────
    //
    // Everything below is the *decision* half of the transport. The effect
    // half stays live-editor-only and is deliberately not faked:
    //
    // - that `viewport.set_input_as_handled()` reaches the same viewport the
    //   keystroke arrived on (`Gd<Viewport>` is unconstructible under
    //   `cargo test` in a cdylib);
    // - that `arm_panel_timer` / `stop_panel_timer` drive a real `Timer` node
    //   at `timeoutlen`;
    // - that `FocusChain::sample` reads the ancestors Godot actually has, and
    //   that `refresh_focus_chain` passes the live values into `chain_key`
    //   rather than a constant — the argument expressions are still review-only,
    //   which is exactly why they were reduced to five bare reads;
    // - that `gui_get_focus_owner` answers before Godot has moved focus.

    fn id(n: i64) -> InstanceId {
        InstanceId::from_i64(n)
    }

    // ── (a) The chain cache key ──────────────────────────────────────

    #[test]
    fn every_chain_key_component_moves_the_key() {
        // The cache is the one piece of shipped state that can silently
        // MISCLASSIFY: a component that stops being read means a stale chain
        // is reused, and the answer is wrong rather than merely slow. `mode`
        // is the sharp one — a mode change does not move the focus owner, so
        // dropping it leaves `editor.insert` classified as `editor.nav` and
        // Ctrl+H stops being backspace.
        let base = chain_key(Some(id(1)), Some(id(2)), Some(Mode::Normal), Some(id(3)), 7);
        assert_eq!(
            base,
            chain_key(Some(id(1)), Some(id(2)), Some(Mode::Normal), Some(id(3)), 7),
            "the same five facts must produce the same key, or nothing ever hits"
        );

        let moved = [
            (
                "focus",
                chain_key(Some(id(9)), Some(id(2)), Some(Mode::Normal), Some(id(3)), 7),
            ),
            (
                "attached",
                chain_key(Some(id(1)), Some(id(9)), Some(Mode::Normal), Some(id(3)), 7),
            ),
            (
                "mode",
                chain_key(Some(id(1)), Some(id(2)), Some(Mode::Insert), Some(id(3)), 7),
            ),
            (
                "prompt",
                chain_key(Some(id(1)), Some(id(2)), Some(Mode::Normal), Some(id(9)), 7),
            ),
            (
                "generation",
                chain_key(Some(id(1)), Some(id(2)), Some(Mode::Normal), Some(id(3)), 8),
            ),
        ];
        for (component, key) in moved {
            assert_ne!(base, key, "{component} does not move the cache key");
        }
    }

    #[test]
    fn absence_is_a_distinct_key_for_every_component() {
        // `None` must not collapse onto some other value: no controller, no
        // attached editor and no prompt are all real states the dispatcher
        // runs in, and each has to invalidate a key sampled while they existed.
        let full = chain_key(Some(id(1)), Some(id(2)), Some(Mode::Normal), Some(id(3)), 7);
        for (component, key) in [
            (
                "focus",
                chain_key(None, Some(id(2)), Some(Mode::Normal), Some(id(3)), 7),
            ),
            (
                "attached",
                chain_key(Some(id(1)), None, Some(Mode::Normal), Some(id(3)), 7),
            ),
            (
                "mode",
                chain_key(Some(id(1)), Some(id(2)), None, Some(id(3)), 7),
            ),
            (
                "prompt",
                chain_key(Some(id(1)), Some(id(2)), Some(Mode::Normal), None, 7),
            ),
        ] {
            assert_ne!(full, key, "{component} losing its value must move the key");
        }
    }

    #[test]
    fn every_focus_chain_field_is_covered_by_a_chain_key_component() {
        // The totality fence. `FocusChain` is what a re-sample produces, and
        // `ChainKey` is what decides whether to re-sample; a field added to
        // the first with no component covering it in the second is a fact that
        // can change while the cache reports a hit. The exhaustive destructure
        // below fails to COMPILE when that happens, which is the only way to
        // force the decision — `every_probe_relevant_fact_moves_the_classification`
        // looks like it covers this and does not: it asserts each field moves
        // the CLASSIFICATION and never touches `ChainKey` at all.
        let FocusChain {
            nodes,
            attached_editor,
            editor_mode,
            in_filesystem_dock,
            sibling_nav_control,
            is_plugin_prompt,
        } = FocusChain {
            nodes: vec![ChainNode::new(
                "Tree",
                "Tree",
                id(1),
                crate::actions::surface::ClassMask::TREE,
            )],
            attached_editor: Some(id(2)),
            editor_mode: Some(Mode::Normal),
            in_filesystem_dock: true,
            sibling_nav_control: Some(id(4)),
            is_plugin_prompt: false,
        };

        let base = chain_key(Some(id(1)), Some(id(2)), Some(Mode::Normal), Some(id(3)), 7);

        // `nodes` ← `focus`. The whole chain is walked upward from the focus
        // owner, so a different owner is a different chain by construction.
        assert!(!nodes.is_empty());
        assert_ne!(
            base,
            chain_key(Some(id(9)), Some(id(2)), Some(Mode::Normal), Some(id(3)), 7)
        );

        // `attached_editor` ← `attached`. Passed straight into `sample`.
        assert_eq!(attached_editor, Some(id(2)));
        assert_ne!(
            base,
            chain_key(Some(id(1)), Some(id(9)), Some(Mode::Normal), Some(id(3)), 7)
        );

        // `editor_mode` ← `mode`. The field that does NOT follow from focus:
        // typing `i` changes the mode and moves nothing else.
        assert_eq!(editor_mode, Some(Mode::Normal));
        assert_ne!(
            base,
            chain_key(Some(id(1)), Some(id(2)), Some(Mode::Insert), Some(id(3)), 7)
        );

        // `in_filesystem_dock` ← `focus`. `is_ancestor_of(focus_owner)`, so it
        // cannot change while the focus owner stands still.
        assert!(in_filesystem_dock);

        // `sibling_nav_control` ← `focus`. A depth-20 DFS from the focus
        // owner, gated on the owner being a `LineEdit`.
        assert_eq!(sibling_nav_control, Some(id(4)));

        // `is_plugin_prompt` ← `focus` AND `prompt`: instance equality between
        // the two, so either moving re-samples.
        assert!(!is_plugin_prompt);
        assert_ne!(
            base,
            chain_key(Some(id(1)), Some(id(2)), Some(Mode::Normal), Some(id(9)), 7)
        );
    }

    // ── (b) The five consumption arms ────────────────────────────────

    fn a_candidate() -> Candidate {
        Candidate {
            surface: "dock",
            target: CandidateTarget::Shortcut("filesystem_dock/delete".into()),
            params: Params::new(),
            consume: Consumption::Elastic,
            repeat: Repeat::Allow,
        }
    }

    #[test]
    fn every_plan_declares_its_consumption_and_its_timer_effect() {
        // The whole consumption contract of the transport, in one table.
        // Before this existed, deleting `viewport.set_input_as_handled()` from
        // the `Plan::Clear` arm passed 1416/1416: a dead prefix would leak its
        // terminating key into Tree incremental search after the prefix had
        // already been destroyed.
        let key = KeyEvent::new(VimKey::Char('g'), Modifiers::NONE);
        let cases = [
            ("Drop", Plan::Drop, Disposition::Ignore, TimerAction::Leave),
            (
                "Swallow",
                Plan::Swallow,
                Disposition::Consume,
                TimerAction::Leave,
            ),
            ("Arm", Plan::Arm, Disposition::Consume, TimerAction::Arm),
            (
                "Clear",
                Plan::Clear,
                Disposition::Consume,
                TimerAction::Stop,
            ),
            (
                "Run",
                Plan::Run(vec![a_candidate()], key),
                Disposition::Ignore,
                TimerAction::Stop,
            ),
        ];
        for (name, plan, disposition, timer) in cases {
            assert_eq!(plan_effects(&plan), (disposition, timer), "Plan::{name}");
        }
    }

    #[test]
    fn only_the_echo_arm_leaves_a_pending_prefix_armed() {
        // The other half of "a held prefix key does not keep its buffer alive
        // forever": `Swallow` must NOT restart the timer, or an autorepeating
        // `g` re-arms `timeoutlen` ~20 times a second and the prefix never
        // times out. `Arm` is the only plan allowed to touch it.
        for plan in [Plan::Drop, Plan::Swallow] {
            assert_eq!(plan_effects(&plan).1, TimerAction::Leave);
        }
        assert_eq!(plan_effects(&Plan::Arm).1, TimerAction::Arm);
    }

    #[test]
    fn a_plan_that_runs_leaves_consumption_to_the_fold() {
        // `Plan::Run` answering `Consume` here would make `<void>`,
        // `<norepeat>` and decline-and-fall-through unreachable: `j` at the
        // end of a list would be eaten instead of reaching Godot.
        let key = KeyEvent::new(VimKey::Char('j'), Modifiers::NONE);
        assert_eq!(
            plan_effects(&Plan::Run(Vec::new(), key)).0,
            Disposition::Ignore
        );
        assert_eq!(
            plan_effects(&Plan::Run(vec![a_candidate()], key)).0,
            Disposition::Ignore,
            "the candidate list must not change the plan's own verdict"
        );
    }
}
