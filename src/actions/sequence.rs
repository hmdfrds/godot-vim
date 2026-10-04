//! Pending prefixes: multi-key sequences outside the editor, and the shell
//! timer that resolves them.
//!
//! This is the only place the shell plane holds state *between* keystrokes,
//! and it exists in exactly one shape because of one fact about the host:
//!
//! # Godot's `_input()` has no replay channel
//!
//! `set_input_as_handled()` destroys the event permanently. There is no
//! synchronous way to put a key back, and `Input::parse_input_event` is not
//! one either — it appends to the list `flush_buffered_events` is draining, so
//! a "replay" would arrive in a later frame against a possibly different focus
//! owner. Everything below follows from that:
//!
//! - **Speculative consumption is forbidden.** Consuming `g` on the chance
//!   that `gg` follows would kill a `Tree`'s incremental type-to-search
//!   mid-word, and on an editor-reachable surface it would turn `<C-w>s` into
//!   a bare destructive `s`. So a prefix key is consumed **only** where the
//!   user has explicitly bound a sequence starting with it, on that surface.
//! - **Reservation is visible.** Binding `dd` on `dock.filesystem` implicitly
//!   reserves bare `d` there, and `:panelmap` prints the reservation. A
//!   silently reserved key is the silent dead key this design exists to
//!   prevent.
//! - **A timeout cannot flush keys as literals.** Vim would; we cannot. On
//!   timeout an exact match at the buffered prefix runs, and otherwise the
//!   buffer is simply dropped. See [`Pending::on_timeout`].
//! - **Multi-key LHSs are rejected at registration on editor-reachable
//!   surfaces** ([`super::bind::BindingIndex::try_insert`]), so the shell
//!   plane can never hold a pending prefix while the sampled leaf is the
//!   script editor. `<C-w>s`, `gg`, `gU` and `gv` stay vim-core's.
//!
//! The resolution rules, in order (§5.10):
//!
//! 1. `pending` empty and the key **not reserved** on this surface: single-key
//!    exact lookup only. **No state is created and nothing is consumed on a
//!    miss.** This is why `g` then `o` still reaches a `Tree`'s type-to-search
//!    when nothing reserves `g`.
//! 2. Key **is** reserved, or `pending` is non-empty: push (capped at
//!    [`MAX_KEY_SEQUENCE_LEN`], deliberately the same cap vim-core applies so
//!    the two planes cannot disagree), then `lookup(&pending)`.
//!    - `ExactOnly` → run it, clear `pending`.
//!    - `Prefix { .. }` → consume, buffer, arm the shell timer.
//!    - `NoMatch` → clear `pending` and **consume this key too**. A reserved
//!      prefix owns its whole subtree; otherwise the terminating key leaks
//!      into `Tree` incremental search. A deliberate divergence from Vim,
//!      which would flush both as literals.
//!
//! See `DESIGN-rebindable-nav.md` (design notes, kept outside this repo) §5.10 and the `P8` block in §10.

use vim_core::keymap::{KeyEvent, TrieLookup, MAX_KEY_SEQUENCE_LEN};

use super::action::ActionRegistry;
use super::bind::BindingIndex;
use super::caps::Caps;
use super::resolve::{hit_from, walk_scope, Candidate, Hit, ResolveInput};
use super::surface::{SurfaceId, SurfacePath};

/// What the sequence layer decided about one keystroke.
///
/// Every variant except [`Self::Passthrough`] consumes the event. That
/// asymmetry is the whole safety property: the *only* way to reach the
/// ordinary single-key walk is to have created no state and consumed nothing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SeqStep {
    /// Not a sequence. Resolve this key single-key exactly as before —
    /// nothing was buffered and nothing was consumed.
    Passthrough,
    /// An auto-repeat echo arrived while a buffer was open. Consume and
    /// **discard** it: it must not extend the buffer (holding `g` would turn
    /// `gg` into `ggg`) and it must not restart the timer (a held prefix key
    /// would keep the buffer alive forever).
    Echo,
    /// The buffer opened or grew and could still continue. Consume the key
    /// and arm the shell timer.
    Buffered,
    /// The buffer completed. Run these, then the buffer is already clear.
    Run(Vec<Candidate>, KeyEvent),
    /// A reserved prefix's subtree has no continuation for this key — or the
    /// only continuation was gated out, or `native` was reached where there is
    /// nothing left to hand back. Consume this key too and clear.
    ///
    /// `:checkhealth` reports this rather than leaving a user to discover it.
    DeadPrefix(SurfaceId),
}

/// The pending prefix buffer.
///
/// Empty (`surface == None`) is the overwhelmingly common state and costs one
/// `Option` compare per keystroke: the shipped zero-config keyset reserves
/// nothing at all, so [`Self::step`] returns [`SeqStep::Passthrough`] before
/// touching the trie unless the user bound a sequence.
#[derive(Debug, Default)]
pub(crate) struct Pending {
    keys: Vec<KeyEvent>,
    /// The surface whose trie owns the buffered sequence. `None` means no
    /// buffer is open, and is the single source of truth for "is a sequence
    /// in flight" — `keys` is cleared alongside it.
    surface: Option<SurfaceId>,
    /// The capabilities of the path the sequence started on.
    ///
    /// Snapshotted rather than re-derived at timeout, because the timeout
    /// fires with no keystroke and therefore with no fresh classification.
    /// Safe because every path that changes the focus owner clears the buffer.
    caps: Caps,
}

impl Pending {
    /// Whether a sequence is in flight.
    pub(crate) fn is_active(&self) -> bool {
        self.surface.is_some()
    }

    /// The keys buffered so far. For diagnostics and tests.
    pub(crate) fn keys(&self) -> &[KeyEvent] {
        &self.keys
    }

    /// The surface the buffer is anchored to, if one is open.
    pub(crate) fn surface(&self) -> Option<SurfaceId> {
        self.surface
    }

    /// Drop the buffer.
    ///
    /// Called on execute, `NoMatch`, timeout, focus-owner change, plugin
    /// disable and config reload — the six clearing events of §5.10. It is
    /// deliberately **not** called on echo.
    pub(crate) fn clear(&mut self) {
        self.keys.clear();
        self.surface = None;
        self.caps = Caps::empty();
    }

    /// Feed one keystroke through the sequence layer.
    pub(crate) fn step(&mut self, input: &ResolveInput<'_>, is_echo: bool) -> SeqStep {
        // A buffer anchored to a surface that is no longer on the path means
        // the focus owner moved without the transport noticing. Dropping it is
        // strictly better than resolving `g` typed in the FileSystem dock
        // against whatever has focus now.
        if let Some(surface) = self.surface {
            if !input.path.ids.contains(&surface) {
                log::debug!("sequence: dropping the {surface} buffer — surface left the path");
                self.clear();
            }
        }

        if self.is_active() {
            if is_echo {
                return SeqStep::Echo;
            }
            return self.extend(input);
        }
        if is_echo {
            // An echo can never OPEN a buffer. Without this the timer becomes
            // a metronome: `on_timeout` runs the prefix's exact-match action
            // and clears the buffer, the key is still held, and the next echo
            // reaches `open()`, which finds the key reserved, buffers it and
            // re-arms the timer — so the action re-fires once per
            // `timeoutlen`, forever. `<norepeat>` cannot stop it either;
            // `Repeat::Suppress` is only read in [`super::resolve::dispose`],
            // which this path never reaches.
            //
            // `Passthrough` rather than `Echo` so the held key still reaches
            // Godot's own repeat handling — the elastic default, and what
            // `an_echo_with_no_buffer_open_is_an_ordinary_passthrough` already
            // guarantees for every unreserved key.
            return SeqStep::Passthrough;
        }
        self.open(input)
    }

    /// Rule 1 / rule 2's first half: is this key reserved anywhere on the path?
    ///
    /// Walks leaf→root exactly as [`super::resolve::resolve`] does, so depth
    /// is specificity here too, and honours the same seal — a reservation must
    /// not swallow a character a sealed filter box is entitled to.
    ///
    /// One stage of the model is deliberately **absent**: the S6 arbitration
    /// seam (`vim_claims`) is never consulted, so `input.vim_claims` goes
    /// unread on this path. That is sound only because a multi-key LHS is
    /// rejected at registration on every editor-reachable surface, which makes
    /// "a reservation exists on a surface that yields to the engine"
    /// unrepresentable. It is named here rather than left implicit: a future
    /// phase that relaxes the registration guard must add the gate here too,
    /// or a reserved key would be consumed out from under a user's own `:map`.
    /// Probe-major for the same reason [`super::resolve::walk_path`] is: probe
    /// priority outranks surface depth. A reservation reached by physical
    /// position on `dock.filesystem` must not swallow a key the user actually
    /// typed and `dock` reserves.
    fn open(&mut self, input: &ResolveInput<'_>) -> SeqStep {
        let (scope, _sealed) = walk_scope(input);

        // ── Pass 1: what the user typed, leaf→root ───────────────────
        for probe in input.probes.iter_typed() {
            for &surface in scope {
                if let Some(step) = self.try_reserve(input, surface, probe) {
                    return step;
                }
            }
        }

        // ── Pass 2: the US-QWERTY positional guess ───────────────────
        if input.path.anchor_refuses_positional {
            return SeqStep::Passthrough;
        }
        let Some(probe) = input.probes.positional() else {
            return SeqStep::Passthrough;
        };
        for &surface in scope {
            // Per RULE, not per surface: the reservation is created by the
            // multi-key rules that start with this key, so it is those rules'
            // own `<physical>` that decides whether a physical position may
            // open the buffer. `has_physical_rule` stays only as the cheap
            // bail-out ahead of the scan.
            if !input.index.has_physical_rule(surface) {
                continue;
            }
            if !input
                .index
                .sequences_from(surface, probe)
                .any(|rule| rule.physical)
            {
                continue;
            }
            if let Some(step) = self.try_reserve(input, surface, probe) {
                return step;
            }
        }
        SeqStep::Passthrough
    }

    /// Open the buffer if `probe` is a live reservation on `surface`.
    fn try_reserve(
        &mut self,
        input: &ResolveInput<'_>,
        surface: SurfaceId,
        probe: KeyEvent,
    ) -> Option<SeqStep> {
        // THE anti-speculation pair. `is_reserved` reads the live slot table;
        // the `Prefix` answer reads the trie's shape. They are redundant *by
        // construction* today — every trie child comes from a `slot_of` entry
        // — and both are kept on purpose: `is_reserved` is the fact
        // `:panelmap` prints, and buffering on a different fact than the one
        // the introspector announces is how a reservation becomes
        // unexplainable. The equivalence is pinned by
        // `a_trie_prefix_always_means_a_printed_reservation`.
        //
        // Together they are the reason nothing is consumed speculatively: an
        // unreserved key creates no state and goes on to the single-key walk
        // and then to Godot, which is what keeps a `Tree`'s incremental
        // type-to-search alive.
        if !input.index.is_reserved(surface, probe) {
            return None;
        }
        // `<nowait>` makes the trie promote the shorter LHS to `ExactOnly`
        // even though `dd` shares the prefix, so `d` fires immediately. Fall
        // through to the single-key walk, which finds it.
        let TrieLookup::Prefix { .. } = input.index.lookup(surface, &[probe]) else {
            return None;
        };
        self.keys.push(probe);
        self.surface = Some(surface);
        self.caps = input.path.caps;
        log::trace!("sequence: {surface} reserved {probe}; buffering");
        Some(SeqStep::Buffered)
    }

    /// Rule 2's second half: extend an open buffer.
    fn extend(&mut self, input: &ResolveInput<'_>) -> SeqStep {
        let Some(surface) = self.surface else {
            return SeqStep::Passthrough;
        };
        if self.keys.len() >= MAX_KEY_SEQUENCE_LEN {
            // Unreachable through the parser, which caps an LHS at the same
            // value, so a `Prefix` at this depth cannot exist. Bounded anyway:
            // an unbounded buffer in `_input()` is a memory leak driven by the
            // key-repeat rate.
            log::warn!("sequence: {surface} buffer hit the {MAX_KEY_SEQUENCE_LEN}-key cap");
            self.clear();
            return SeqStep::DeadPrefix(surface);
        }
        // Probes 1–2 first and unconditionally, then the positional guess —
        // the same two passes the resolver walks, for the same reason. Only
        // one surface is in play here, so the passes differ from the resolver
        // only in what gates pass 2.
        for probe in input.probes.iter_typed() {
            if let Some(step) = self.try_continue(input, surface, probe) {
                return step;
            }
        }
        if !input.path.anchor_refuses_positional {
            if let Some(probe) = input.probes.positional() {
                // Per RULE: the continuation the guess would take must itself
                // carry `<physical>`. `has_physical_rule` is the cheap
                // bail-out ahead of the scan and nothing more.
                let next = [self.keys.as_slice(), &[probe]].concat();
                if input.index.has_physical_rule(surface)
                    && input
                        .index
                        .rules_on(surface)
                        .any(|rule| rule.physical && rule.lhs.starts_with(&next))
                {
                    if let Some(step) = self.try_continue(input, surface, probe) {
                        return step;
                    }
                }
            }
        }
        log::debug!("sequence: dead prefix on {surface}");
        self.clear();
        SeqStep::DeadPrefix(surface)
    }

    /// Push `probe` onto the buffer and read the trie's answer, un-pushing if
    /// the sequence does not continue that way.
    ///
    /// `None` means "this probe is not a continuation" — the caller tries the
    /// next one. Every `Some` is terminal for this keystroke.
    fn try_continue(
        &mut self,
        input: &ResolveInput<'_>,
        surface: SurfaceId,
        probe: KeyEvent,
    ) -> Option<SeqStep> {
        self.keys.push(probe);
        match input.index.lookup(surface, &self.keys) {
            TrieLookup::ExactOnly(entry) => {
                let hit = hit_from(input.index, input.registry, self.caps, surface, entry);
                self.clear();
                Some(match hit {
                    Hit::Run(candidate) => SeqStep::Run(vec![candidate], probe),
                    // Both terminal-but-nothing-to-run cases collapse here,
                    // and both consume. `native` in particular cannot do its
                    // job at the end of a sequence: the prefix keys are
                    // already destroyed, so "hand the key back to Godot" would
                    // hand back a fragment.
                    Hit::Native | Hit::Miss => {
                        log::debug!("sequence: {surface} completed with nothing runnable");
                        SeqStep::DeadPrefix(surface)
                    }
                })
            }
            TrieLookup::Prefix { .. } => Some(SeqStep::Buffered),
            // `NoMatch`, and any future variant: this probe does not continue
            // the sequence. Un-push and let the next probe try.
            _ => {
                self.keys.pop();
                None
            }
        }
    }

    /// The shell timer fired: resolve whatever is buffered, then clear.
    ///
    /// Returns the candidates to run when an exact match exists at the
    /// buffered prefix (`:panelmap dock g …` alongside `:panelmap dock gg …`),
    /// and `None` otherwise.
    ///
    /// `None` means the buffered keys are **dropped**. They are deliberately
    /// not synthesised back: Godot's `_input()` has no replay channel —
    /// `set_input_as_handled()` already destroyed them, and re-injecting
    /// through `Input::parse_input_event` would re-dispatch them in a later
    /// frame against a focus owner that may have changed. Flushing them as
    /// literals is therefore not "more Vim-faithful"; it is a different and
    /// unpredictable action.
    pub(crate) fn on_timeout(
        &mut self,
        index: &BindingIndex,
        registry: &ActionRegistry,
    ) -> Option<(Vec<Candidate>, KeyEvent)> {
        let resolved = self.resolve_at_timeout(index, registry);
        self.clear();
        resolved
    }

    fn resolve_at_timeout(
        &self,
        index: &BindingIndex,
        registry: &ActionRegistry,
    ) -> Option<(Vec<Candidate>, KeyEvent)> {
        let surface = self.surface?;
        let last = *self.keys.last()?;
        // `Prefix { exact }` is the ambiguous case the timer exists for.
        // `ExactOnly` cannot normally be buffered — the walk would have run it
        // — but it is honoured rather than dropped if some future rebuild
        // produces one.
        let entry = match index.lookup(surface, &self.keys) {
            TrieLookup::Prefix { exact: Some(entry) } => entry,
            TrieLookup::ExactOnly(entry) => entry,
            _ => return None,
        };
        match hit_from(index, registry, self.caps, surface, entry) {
            Hit::Run(candidate) => Some((vec![candidate], last)),
            Hit::Native | Hit::Miss => None,
        }
    }
}

/// The deepest surface on `path` that reserves any bare key, if there is one.
///
/// This is the *entire* scope of `Tree::set_allow_search(false)` /
/// `ItemList::set_allow_search(false)`. Godot's `Tree` runs incremental
/// type-to-search on bare printable keys, so a reserved `g` would otherwise do
/// both jobs at once: buffer in the shell plane and start a type-search in the
/// control. Suppressing search on controls whose surface reserves *nothing*
/// would break type-to-search for every user who never bound a sequence, which
/// is every user of the shipped keyset.
pub(crate) fn path_reserves(index: &BindingIndex, path: &SurfacePath) -> Option<SurfaceId> {
    path.ids
        .iter()
        .copied()
        .find(|&surface| index.reserves_any(surface))
}

/// Where the shell timer's `timeoutlen` comes from, in priority order.
///
/// `from_engine` is `VimController::timeoutlen()`, which delegates to
/// `engine()` and is therefore available in both `Attached` and `Detached`
/// controller phases — but **not** when there is no controller at all, which
/// is every moment between `exit_tree` and the next `enter_tree`, and any
/// state a panic left behind. `from_settings` is `SettingsSnapshot.timeoutlen`,
/// the same `i64` that is clamped into `VimOptions::set_timeoutlen_ms`.
///
/// Naming the fallback matters because the headline capability of this phase
/// is "a dock prefix resolves with no script open": falling back to a
/// hardcoded constant there would silently ignore the user's own setting in
/// precisely the state where dock bindings are all there is.
///
/// Clamped to the same bounds `SettingsSnapshot::apply_delta` uses, so a
/// hand-edited EditorSettings value cannot arm a 0ms (instant) or multi-minute
/// timer.
pub(crate) fn timeoutlen_ms(from_engine: Option<u32>, from_settings: Option<i64>) -> i64 {
    use crate::settings::defaults;
    from_engine
        .map(i64::from)
        .or(from_settings)
        .unwrap_or(defaults::TIMEOUTLEN)
        .clamp(defaults::TIMEOUTLEN_MIN, defaults::TIMEOUTLEN_MAX)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::actions::action::{Params, RuleTarget};
    use crate::actions::bind::{builtin_index, Consumption, Repeat, Rule};
    use crate::actions::keys::Probes;
    use crate::actions::outcome::Outcome;
    use crate::actions::providers;
    use crate::actions::resolve::{dispose, CandidateTarget, Disposition};
    use crate::actions::specs;
    use crate::actions::surface::Anchor;
    use vim_core::keymap::{Key as VimKey, MappingOwner, Modifiers};

    const NEVER: &dyn Fn(KeyEvent) -> bool = &|_| false;

    const TREE: Caps = Caps::VNAV.union(Caps::HIERARCHY).union(Caps::ACTIVATE);

    /// The whole shipped registry — `specs::SHIPPED` **plus** every
    /// `Provider::actions` table. Looping `SHIPPED` alone here would leave a
    /// provider's own verbs unregistered, and `builtin_index` would then
    /// reject that provider's defaults with `UnknownAction` — a
    /// `debug_assert!` under `Provenance::Builtin`, so the failure is loud
    /// but the cause reads as unrelated.
    fn registry() -> ActionRegistry {
        specs::registry()
    }

    fn ch(c: char) -> KeyEvent {
        KeyEvent::new(VimKey::Char(c), Modifiers::NONE)
    }

    fn ctrl(c: char) -> KeyEvent {
        KeyEvent::new(VimKey::Char(c), Modifiers::CTRL)
    }

    fn path(leaf: &'static str, caps: Caps) -> SurfacePath {
        let forest = providers::forest();
        let spec = forest.get(leaf).expect("declared surface");
        SurfacePath {
            ids: forest.path_from(leaf),
            anchor: Anchor::Node(0),
            caps,
            seal: spec.seal,
            anchor_yields_to_engine: spec.yields_to_engine,
            anchor_refuses_positional: spec.refuses_positional,
        }
    }

    /// An index carrying the shipped defaults plus `lines`, which are written
    /// in exactly the syntax a user would type into a vimrc.
    fn index_with(lines: &str) -> BindingIndex {
        let mut index = builtin_index(&registry());
        let mut diagnostics = Vec::new();
        crate::actions::bind::apply_text(
            &mut index,
            &registry(),
            lines,
            &MappingOwner::User,
            "test",
            crate::actions::bind::Provenance::User,
            &mut diagnostics,
        );
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        index
    }

    /// Feed one keystroke, as-typed only.
    fn press(
        pending: &mut Pending,
        index: &BindingIndex,
        registry: &ActionRegistry,
        path: &SurfacePath,
        key: KeyEvent,
    ) -> SeqStep {
        press_echo(pending, index, registry, path, key, false)
    }

    fn press_echo(
        pending: &mut Pending,
        index: &BindingIndex,
        registry: &ActionRegistry,
        path: &SurfacePath,
        key: KeyEvent,
        is_echo: bool,
    ) -> SeqStep {
        let probes = Probes::from_slice(&[key]);
        press_probes(pending, index, registry, path, &probes, is_echo)
    }

    /// Feed one keystroke carrying its **whole** probe list.
    ///
    /// Every other helper here builds a one-probe list, which is the US-layout
    /// case and only that case. A real keystroke on Cyrillic, Greek or any
    /// `:set langmap` layout arrives with two or three interpretations, and the
    /// sequence layer's two-pass walk exists entirely for them.
    fn press_probes(
        pending: &mut Pending,
        index: &BindingIndex,
        registry: &ActionRegistry,
        path: &SurfacePath,
        probes: &Probes,
        is_echo: bool,
    ) -> SeqStep {
        pending.step(
            &ResolveInput {
                probes,
                path,
                index,
                registry,
                vim_claims: NEVER,
            },
            is_echo,
        )
    }

    /// A rule installed through [`BindingIndex::upsert`], the primitive
    /// underneath `try_insert`.
    ///
    /// Bypassing the gate is the point: `try_insert` refuses a multi-key LHS on
    /// every editor-reachable surface, so a reservation on `panel` — the state
    /// the seal has to be tested against — is unreachable through the parser by
    /// design. What is installed is a real rule: same trie, same slot table,
    /// same `is_reserved`.
    fn seq_rule(surface: SurfaceId, lhs: &[KeyEvent], action: &str) -> Rule {
        Rule {
            surface,
            lhs: lhs.to_vec(),
            target: RuleTarget::Action(registry().id_of(action).expect("registered")),
            params: Params::new(),
            consume: Consumption::Elastic,
            repeat: Repeat::Allow,
            physical: false,
            shift_tolerant: false,
            nowait: false,
            owner: MappingOwner::User,
            desc: "test".into(),
        }
    }

    fn ran(step: &SeqStep) -> Option<&'static str> {
        match step {
            SeqStep::Run(candidates, _) => match candidates.first()?.target {
                CandidateTarget::Action(_, spec) => Some(spec.id),
                CandidateTarget::Shortcut(_) => None,
            },
            _ => None,
        }
    }

    // ── Rule 1: reservation, never speculation ───────────────────────

    #[test]
    fn an_unreserved_key_is_never_consumed_and_never_buffered() {
        // THE anti-speculation test. `g` is bound nowhere in the shipped
        // keyset, so pressing it must create no state and must reach Godot —
        // which is what keeps a `Tree`'s incremental type-to-search alive.
        let index = builtin_index(&registry());
        let reg = registry();
        let p = path("dock", TREE);
        let mut pending = Pending::default();
        for key in [ch('g'), ch('o'), ch('z'), ch('j')] {
            assert_eq!(
                press(&mut pending, &index, &reg, &p, key),
                SeqStep::Passthrough,
                "{key} must fall through untouched"
            );
            assert!(!pending.is_active(), "{key} must not open a buffer");
            assert!(pending.keys().is_empty());
        }
    }

    #[test]
    fn the_shipped_keyset_reserves_nothing_anywhere() {
        // Zero-config users must never lose type-to-search, and must never
        // meet a pending buffer at all.
        let index = builtin_index(&registry());
        for surface in providers::forest().ids() {
            assert!(
                !index.reserves_any(surface),
                "{surface} reserves a prefix out of the box"
            );
            assert!(index.reservations(surface).is_empty());
        }
    }

    #[test]
    fn a_reserved_key_buffers_and_consumes() {
        let index = index_with("panelmap dock gg godotvim.item.next");
        let reg = registry();
        let p = path("dock", TREE);
        let mut pending = Pending::default();
        assert_eq!(
            press(&mut pending, &index, &reg, &p, ch('g')),
            SeqStep::Buffered
        );
        assert!(pending.is_active());
        assert_eq!(pending.keys(), &[ch('g')]);
        assert_eq!(pending.surface(), Some("dock"));
    }

    #[test]
    fn reserving_g_leaves_every_other_key_alone() {
        // The reservation is per-key, not per-surface: binding `gg` must not
        // make `o` or `j` start behaving like prefixes.
        let index = index_with("panelmap dock gg godotvim.item.next");
        let reg = registry();
        let p = path("dock", TREE);
        let mut pending = Pending::default();
        for key in [ch('o'), ch('j'), ch('z')] {
            assert_eq!(
                press(&mut pending, &index, &reg, &p, key),
                SeqStep::Passthrough
            );
            assert!(!pending.is_active());
        }
    }

    // ── Rule 2: completion, dead prefixes, `<nowait>` ────────────────

    #[test]
    fn gg_fires_on_the_second_g() {
        let index = index_with("panelmap dock gg godotvim.item.prev");
        let reg = registry();
        let p = path("dock", TREE);
        let mut pending = Pending::default();
        assert_eq!(
            press(&mut pending, &index, &reg, &p, ch('g')),
            SeqStep::Buffered
        );
        let step = press(&mut pending, &index, &reg, &p, ch('g'));
        assert_eq!(ran(&step), Some("godotvim.item.prev"));
        assert!(!pending.is_active(), "the buffer clears on execute");
    }

    #[test]
    fn g_then_x_consumes_both_and_reports_a_dead_prefix() {
        // The deliberate divergence from Vim: a reserved prefix owns its whole
        // subtree, so the terminating key is consumed too rather than leaking
        // into `Tree` incremental search.
        let index = index_with("panelmap dock gg godotvim.item.prev");
        let reg = registry();
        let p = path("dock", TREE);
        let mut pending = Pending::default();
        press(&mut pending, &index, &reg, &p, ch('g'));
        assert_eq!(
            press(&mut pending, &index, &reg, &p, ch('x')),
            SeqStep::DeadPrefix("dock")
        );
        assert!(!pending.is_active(), "the buffer clears on NoMatch");
    }

    #[test]
    fn nowait_fires_immediately_despite_a_longer_sequence() {
        // `<nowait>` makes the trie answer `ExactOnly` at the shorter LHS, so
        // the sequence layer must decline to buffer and let the single-key
        // walk have it.
        let index = index_with(
            "panelmap dock gg godotvim.item.prev\n\
             panelmap <nowait> dock g godotvim.item.next",
        );
        let reg = registry();
        let p = path("dock", TREE);
        let mut pending = Pending::default();
        assert_eq!(
            press(&mut pending, &index, &reg, &p, ch('g')),
            SeqStep::Passthrough,
            "<nowait> must not wait"
        );
        assert!(!pending.is_active());
    }

    #[test]
    fn a_three_key_sequence_buffers_twice() {
        let index = index_with("panelmap dock gxj godotvim.item.next");
        let reg = registry();
        let p = path("dock", TREE);
        let mut pending = Pending::default();
        assert_eq!(
            press(&mut pending, &index, &reg, &p, ch('g')),
            SeqStep::Buffered
        );
        assert_eq!(
            press(&mut pending, &index, &reg, &p, ch('x')),
            SeqStep::Buffered
        );
        assert_eq!(pending.keys(), &[ch('g'), ch('x')]);
        let step = press(&mut pending, &index, &reg, &p, ch('j'));
        assert_eq!(ran(&step), Some("godotvim.item.next"));
    }

    #[test]
    fn a_completed_sequence_is_gated_by_capabilities() {
        // `godotvim.item.expand` needs HIERARCHY, which an `ItemList` does not
        // offer. The gate must fire at the END of a sequence exactly as it
        // does on a single key — and because the prefix is already consumed,
        // the miss is a dead prefix rather than a fall-through.
        let index = index_with("panelmap dock gl godotvim.item.expand");
        let reg = registry();
        let list = path("dock", Caps::VNAV | Caps::ACTIVATE);
        let mut pending = Pending::default();
        press(&mut pending, &index, &reg, &list, ch('g'));
        assert_eq!(
            press(&mut pending, &index, &reg, &list, ch('l')),
            SeqStep::DeadPrefix("dock")
        );

        // …and lives on a Tree.
        let tree = path("dock", TREE);
        let mut pending = Pending::default();
        press(&mut pending, &index, &reg, &tree, ch('g'));
        let step = press(&mut pending, &index, &reg, &tree, ch('l'));
        assert_eq!(ran(&step), Some("godotvim.item.expand"));
    }

    #[test]
    fn the_deepest_surface_wins_a_reservation() {
        let index = index_with(
            "panelmap dock gg godotvim.item.next\n\
             panelmap dock.filesystem gg godotvim.fs.refresh",
        );
        let reg = registry();
        let fs = path("dock.filesystem", TREE | Caps::FILEOPS);
        let mut pending = Pending::default();
        press(&mut pending, &index, &reg, &fs, ch('g'));
        assert_eq!(pending.surface(), Some("dock.filesystem"));
        let step = press(&mut pending, &index, &reg, &fs, ch('g'));
        assert_eq!(ran(&step), Some("godotvim.fs.refresh"));
    }

    #[test]
    fn a_sealed_anchor_still_swallows_a_bare_key_before_a_parent_reservation() {
        // The seal, honoured on the way UP, exactly as `resolve` honours it:
        // a reservation on a PARENT must not claim a character the filter box
        // is entitled to.
        //
        // The reservation is on `panel`, which is on the searchbox path and
        // above the seal — the only arrangement that tests the seal at all.
        // The previous version of this test put its rule on `dock`, which is
        // not on the searchbox path, and said so in its own comment: it
        // asserted that an absent rule does not fire, and passed with the seal
        // removed entirely.
        let mut index = builtin_index(&registry());
        index.upsert(seq_rule(
            "panel",
            &[ch('g'), ch('g')],
            "godotvim.focus.left",
        ));
        index.upsert(seq_rule(
            "panel",
            &[ctrl('x'), ctrl('f')],
            "godotvim.focus.left",
        ));
        let reg = registry();
        let search = path("searchbox", Caps::TEXTENTRY);
        assert!(
            index.is_reserved("panel", ch('g')) && index.is_reserved("panel", ctrl('x')),
            "the fixture must really reserve both, or this test proves nothing"
        );

        let mut pending = Pending::default();
        assert_eq!(
            press(&mut pending, &index, &reg, &search, ch('g')),
            SeqStep::Passthrough,
            "a sealed anchor swallows a bare key before the parent is consulted"
        );
        assert!(!pending.is_active(), "…and must open no buffer");
        assert!(pending.keys().is_empty());

        // The mirror, and what makes the half above a claim about the SEAL
        // rather than about an empty path: a command chord continues past the
        // anchor to `panel`, where the same kind of reservation is honoured.
        let mut pending = Pending::default();
        assert_eq!(
            press(&mut pending, &index, &reg, &search, ctrl('x')),
            SeqStep::Buffered,
            "Ctrl continues past the seal"
        );
        assert_eq!(pending.surface(), Some("panel"));
    }

    // ── Multi-probe keystrokes (non-QWERTY layouts) ──────────────────

    #[test]
    fn a_lower_priority_probe_continues_a_sequence_the_first_probe_cannot() {
        // The sequence analogue of `the_editor_still_gets_the_latin_probe`,
        // and the NORMAL case for every Cyrillic/Greek user and everyone with
        // `:set langmap`: probe 1 is the character they typed, probe 2 its
        // Latin collapse. `try_continue` must un-push a probe the trie
        // rejected, or the buffer becomes `[g, <typed>, j]`, the sequence dies
        // as a dead prefix, and the key is consumed anyway.
        let reg = registry();
        let p = path("dock", TREE);
        let probes = Probes::from_slice(&[ch('x'), ch('j')]);

        // (i) the second probe COMPLETES the sequence.
        let index = index_with("panelmap dock gj godotvim.item.next");
        let mut pending = Pending::default();
        assert_eq!(
            press(&mut pending, &index, &reg, &p, ch('g')),
            SeqStep::Buffered
        );
        let step = press_probes(&mut pending, &index, &reg, &p, &probes, false);
        assert_eq!(ran(&step), Some("godotvim.item.next"));
        assert!(!pending.is_active());

        // (ii) the second probe EXTENDS it, which makes the buffer itself
        // observable: exactly the two keys that matched, with the rejected
        // probe gone rather than buried in the middle.
        let index = index_with("panelmap dock gjj godotvim.item.prev");
        let mut pending = Pending::default();
        press(&mut pending, &index, &reg, &p, ch('g'));
        assert_eq!(
            press_probes(&mut pending, &index, &reg, &p, &probes, false),
            SeqStep::Buffered
        );
        assert_eq!(pending.keys(), &[ch('g'), ch('j')]);
    }

    // ── `<physical>` scoping, both passes ────────────────────────────

    #[test]
    fn a_positional_guess_opens_a_buffer_only_where_a_rule_asked_for_one() {
        // Probe 3 is a guess about where the key sits on a US keyboard, and
        // `<physical>` is how an individual rule opts into being reached by
        // it. `has_physical_rule` is the cheap bail-out ahead of the scan and
        // nothing more: reading it alone would let one unrelated physical rule
        // re-open the positional guess for every sequence on the surface.
        let reg = registry();
        let p = path("dock", TREE);
        // Probe 1 is what was typed and reserves nothing; probe 2 is the
        // US-QWERTY position, which the fixture reserves.
        let probes = Probes::from_slice_positional(&[ch('q'), ch('g')]);

        let plain = index_with("panelmap dock gg godotvim.item.next");
        assert!(
            plain.has_physical_rule("dock"),
            "the shipped dock keyset carries <physical>, so the bail-out is \
             open and the per-rule scan is the only thing deciding"
        );
        let mut pending = Pending::default();
        assert_eq!(
            press_probes(&mut pending, &plain, &reg, &p, &probes, false),
            SeqStep::Passthrough,
            "`gg` did not ask to be reachable by position"
        );
        assert!(!pending.is_active());

        // The same keystroke against a rule that did ask.
        let physical = index_with("panelmap <physical> dock gg godotvim.item.next");
        let mut pending = Pending::default();
        assert_eq!(
            press_probes(&mut pending, &physical, &reg, &p, &probes, false),
            SeqStep::Buffered
        );
        assert_eq!(pending.keys(), &[ch('g')]);
        assert_eq!(pending.surface(), Some("dock"));
    }

    #[test]
    fn a_positional_guess_continues_a_sequence_only_where_the_continuation_asked() {
        // The same scoping one level in, on `extend`. The surface carries a
        // `<physical>` rule (`zz`) so the bail-out passes, but the
        // continuation the guess would take (`gg`) does not — and it is the
        // continuation that decides.
        let index = index_with(
            "panelmap <physical> dock zz godotvim.item.next\n\
             panelmap dock gg godotvim.item.prev",
        );
        let reg = registry();
        let p = path("dock", TREE);
        assert!(index.has_physical_rule("dock"), "the bail-out must be open");

        let mut pending = Pending::default();
        assert_eq!(
            press(&mut pending, &index, &reg, &p, ch('g')),
            SeqStep::Buffered
        );
        let probes = Probes::from_slice_positional(&[ch('q'), ch('g')]);
        assert_eq!(
            press_probes(&mut pending, &index, &reg, &p, &probes, false),
            SeqStep::DeadPrefix("dock"),
            "`gg` is not <physical>, so the guess must not complete it"
        );
        assert!(!pending.is_active());
    }

    // ── The echo rule (§5.8) ─────────────────────────────────────────

    #[test]
    fn an_echo_mid_sequence_is_swallowed_without_extending_the_buffer() {
        // THE echo rule. Holding `g` must not turn `gg` into `ggg`: the echo
        // is consumed and discarded, the buffer is left exactly as it was, and
        // the sequence still completes on the next genuine press.
        let index = index_with("panelmap dock gg godotvim.item.prev");
        let reg = registry();
        let p = path("dock", TREE);
        let mut pending = Pending::default();
        press(&mut pending, &index, &reg, &p, ch('g'));
        for _ in 0..5 {
            assert_eq!(
                press_echo(&mut pending, &index, &reg, &p, ch('g'), true),
                SeqStep::Echo
            );
            assert_eq!(
                pending.keys(),
                &[ch('g')],
                "an echo must not extend the buffer"
            );
            assert!(pending.is_active(), "an echo must not abort the sequence");
        }
        let step = press(&mut pending, &index, &reg, &p, ch('g'));
        assert_eq!(ran(&step), Some("godotvim.item.prev"));
    }

    #[test]
    fn a_held_prefix_key_cannot_re_open_the_buffer_after_a_timeout() {
        // THE runaway. `step` used to consult `is_echo` only inside
        // `if self.is_active()`, so the sequence was: press `g` → Buffered;
        // timeoutlen elapses → `on_timeout` runs `dock g` and CLEARS the
        // buffer; the key is still held, so the next auto-repeat echo reaches
        // `open()`, which never looked at `is_echo`, found `g` reserved,
        // buffered it and re-armed the timer — and the whole cycle repeated
        // once per `timeoutlen` for as long as the key was held. `<norepeat>`
        // could not stop it: `Repeat::Suppress` is only read in `dispose`,
        // which this path never reaches. For a destructive verb that is
        // repeated destruction from a single held key.
        //
        // `Passthrough` rather than `Echo` so the held key still reaches
        // Godot's own repeat handling, matching the elastic default.
        let index = index_with(
            "panelmap dock g godotvim.item.prev\n\
             panelmap dock gg godotvim.item.next",
        );
        let reg = registry();
        let p = path("dock", TREE);
        let mut pending = Pending::default();

        assert_eq!(
            press(&mut pending, &index, &reg, &p, ch('g')),
            SeqStep::Buffered
        );
        // The timer fires: the exact match at the buffered prefix runs and
        // the buffer is dropped.
        assert!(
            pending.on_timeout(&index, &reg).is_some(),
            "`dock g` is an exact match under the `gg` prefix"
        );
        assert!(!pending.is_active());

        // The key was never released, so every subsequent event is an echo.
        for _ in 0..5 {
            assert_eq!(
                press_echo(&mut pending, &index, &reg, &p, ch('g'), true),
                SeqStep::Passthrough,
                "a held prefix key must not re-open the buffer"
            );
            assert!(
                !pending.is_active(),
                "…and must not re-arm the timer either"
            );
            assert!(pending.keys().is_empty());
        }

        // A genuine press still opens it.
        assert_eq!(
            press(&mut pending, &index, &reg, &p, ch('g')),
            SeqStep::Buffered
        );
    }

    #[test]
    fn an_echo_with_no_buffer_open_is_an_ordinary_passthrough() {
        // Held `j` in a dock must keep auto-repeating; the echo rule applies
        // only while a sequence is in flight.
        let index = index_with("panelmap dock gg godotvim.item.prev");
        let reg = registry();
        let p = path("dock", TREE);
        let mut pending = Pending::default();
        assert_eq!(
            press_echo(&mut pending, &index, &reg, &p, ch('j'), true),
            SeqStep::Passthrough
        );
    }

    // ── The timeout ──────────────────────────────────────────────────

    #[test]
    fn timeout_runs_the_exact_match_at_the_buffered_prefix() {
        // `g` and `gg` both bound: `g` buffers (ambiguous), and the timer is
        // what makes the shorter one reachable at all.
        let index = index_with(
            "panelmap dock gg godotvim.item.prev\n\
             panelmap dock g godotvim.item.next",
        );
        let reg = registry();
        let p = path("dock", TREE);
        let mut pending = Pending::default();
        assert_eq!(
            press(&mut pending, &index, &reg, &p, ch('g')),
            SeqStep::Buffered
        );
        let (candidates, _) = pending
            .on_timeout(&index, &reg)
            .expect("the exact match at `g` must run");
        assert_eq!(
            match candidates[0].target {
                CandidateTarget::Action(_, spec) => spec.id,
                CandidateTarget::Shortcut(_) => "<shortcut>",
            },
            "godotvim.item.next"
        );
        assert!(!pending.is_active(), "the buffer clears on timeout");
    }

    #[test]
    fn timeout_without_an_exact_match_flushes_and_synthesises_nothing() {
        // There is no replay channel: the buffered `g` is gone, and inventing
        // a `g` to hand back would be a different action in a later frame.
        let index = index_with("panelmap dock gg godotvim.item.prev");
        let reg = registry();
        let p = path("dock", TREE);
        let mut pending = Pending::default();
        press(&mut pending, &index, &reg, &p, ch('g'));
        assert!(pending.on_timeout(&index, &reg).is_none());
        assert!(!pending.is_active());
    }

    #[test]
    fn timeout_with_nothing_buffered_is_a_no_op() {
        let index = builtin_index(&registry());
        let reg = registry();
        let mut pending = Pending::default();
        assert!(pending.on_timeout(&index, &reg).is_none());
    }

    #[test]
    fn a_timed_out_exact_match_is_still_capability_gated() {
        let index = index_with(
            "panelmap dock gl godotvim.item.next\n\
             panelmap dock g godotvim.item.expand",
        );
        let reg = registry();
        let list = path("dock", Caps::VNAV | Caps::ACTIVATE);
        let mut pending = Pending::default();
        press(&mut pending, &index, &reg, &list, ch('g'));
        assert!(
            pending.on_timeout(&index, &reg).is_none(),
            "item.expand needs HIERARCHY, which an ItemList has not"
        );
    }

    // ── Clearing ─────────────────────────────────────────────────────

    #[test]
    fn a_surface_that_leaves_the_path_drops_the_buffer() {
        let index = index_with("panelmap dock gg godotvim.item.prev");
        let reg = registry();
        let dock = path("dock", TREE);
        let mut pending = Pending::default();
        press(&mut pending, &index, &reg, &dock, ch('g'));
        assert!(pending.is_active());

        // The user tabbed to a filter box: `dock` is no longer on the path.
        let search = path("searchbox", Caps::TEXTENTRY);
        assert_eq!(
            press(&mut pending, &index, &reg, &search, ch('g')),
            SeqStep::Passthrough
        );
        assert!(!pending.is_active());
    }

    #[test]
    fn clear_is_idempotent_and_total() {
        let index = index_with("panelmap dock gg godotvim.item.prev");
        let reg = registry();
        let p = path("dock", TREE);
        let mut pending = Pending::default();
        press(&mut pending, &index, &reg, &p, ch('g'));
        pending.clear();
        pending.clear();
        assert!(!pending.is_active());
        assert!(pending.keys().is_empty());
    }

    // ── The cap ──────────────────────────────────────────────────────

    #[test]
    fn the_buffer_is_capped_at_the_vim_core_maximum() {
        // Reached by hand rather than through the parser, which rejects a
        // 9-key LHS: what this pins is that the buffer cannot grow without
        // bound at the OS key-repeat rate.
        let mut index = builtin_index(&registry());
        let long: Vec<KeyEvent> = (0..MAX_KEY_SEQUENCE_LEN).map(|_| ch('g')).collect();
        index.upsert(Rule {
            surface: "dock",
            lhs: long.clone(),
            target: RuleTarget::Action(registry().id_of("godotvim.item.next").expect("registered")),
            params: Params::new(),
            consume: Consumption::Elastic,
            repeat: Repeat::Allow,
            physical: false,
            shift_tolerant: false,
            nowait: false,
            owner: MappingOwner::User,
            desc: "long".into(),
        });
        let reg = registry();
        let p = path("dock", TREE);
        let mut pending = Pending::default();
        for _ in 0..MAX_KEY_SEQUENCE_LEN - 1 {
            assert_eq!(
                press(&mut pending, &index, &reg, &p, ch('g')),
                SeqStep::Buffered
            );
        }
        assert_eq!(pending.keys().len(), MAX_KEY_SEQUENCE_LEN - 1);
        // The last key completes it, and the buffer never exceeded the cap.
        let step = press(&mut pending, &index, &reg, &p, ch('g'));
        assert_eq!(ran(&step), Some("godotvim.item.next"));
        assert!(pending.keys().len() <= MAX_KEY_SEQUENCE_LEN);
    }

    // ── `allow_search` scoping ───────────────────────────────────────

    #[test]
    fn path_reserves_names_only_a_surface_that_actually_reserves() {
        // THE `set_allow_search` scope. A path with no multi-key rule anywhere
        // on it must answer `None`, or every user who never bound a sequence
        // loses Godot's incremental type-to-search.
        let bare = builtin_index(&registry());
        for leaf in ["dock", "dock.filesystem", "searchbox", "unknown", "panel"] {
            assert_eq!(
                path_reserves(&bare, &path(leaf, Caps::all())),
                None,
                "{leaf} reserves nothing out of the box"
            );
        }

        let with_seq = index_with("panelmap dock gg godotvim.item.next");
        assert_eq!(
            path_reserves(&with_seq, &path("dock", TREE)),
            Some("dock"),
            "a dock Tree must lose type-to-search once `g` is reserved"
        );
        assert_eq!(
            path_reserves(&with_seq, &path("dock.filesystem", TREE)),
            Some("dock"),
            "…and so must a FileSystem Tree, which inherits `dock`"
        );
        assert_eq!(
            path_reserves(&with_seq, &path("searchbox", Caps::TEXTENTRY)),
            None,
            "a filter box does not inherit `dock` and keeps its own behaviour"
        );
    }

    #[test]
    fn a_trie_prefix_always_means_a_printed_reservation() {
        // The two facts `open` consults must not diverge: the trie's `Prefix`
        // answer is what makes dispatch buffer, and `is_reserved` is what
        // `:panelmap` prints. A `Prefix` with no reservation would be a key
        // that waits with no way for the user to find out why; the reverse
        // (a reservation the trie promoted to `ExactOnly`) is `<nowait>` and
        // is legitimate.
        //
        // `<shift>` is in the fixture deliberately: it is the one construct
        // that writes an LHS into the trie without writing it into `slot_of`.
        let index = index_with(
            "panelmap dock gg godotvim.item.prev\n\
             panelmap dock gjj godotvim.item.next\n\
             panelmap <nowait> dock dd godotvim.item.next\n\
             panelmap <shift> dock <Tab> godotvim.item.next\n\
             panelmap dock.filesystem yy godotvim.fs.yank_path",
        );
        let keys: Vec<KeyEvent> = "gjdyRahlk"
            .chars()
            .map(ch)
            .chain([
                KeyEvent::new(VimKey::Tab, Modifiers::NONE),
                KeyEvent::new(VimKey::Tab, Modifiers::SHIFT),
                KeyEvent::new(VimKey::Enter, Modifiers::NONE),
                ctrl('h'),
            ])
            .collect();
        for surface in providers::forest().ids() {
            for &key in &keys {
                if matches!(index.lookup(surface, &[key]), TrieLookup::Prefix { .. }) {
                    assert!(
                        index.is_reserved(surface, key),
                        "{surface} buffers {key} but :panelmap would not print it"
                    );
                }
            }
        }
    }

    #[test]
    fn a_reservation_disappears_with_the_rule_that_created_it() {
        // `panelunmap` must give type-to-search back, which is only true
        // because reservations are DERIVED from the live slot table rather
        // than stored beside it.
        let index = index_with(
            "panelmap dock gg godotvim.item.next\n\
             panelunmap dock gg",
        );
        assert!(!index.reserves_any("dock"));
        assert_eq!(path_reserves(&index, &path("dock", TREE)), None);
    }

    // ── `timeoutlen` ─────────────────────────────────────────────────

    #[test]
    fn timeoutlen_falls_back_to_the_settings_snapshot_with_no_controller() {
        // THE detached source. With no controller reachable at all — the
        // state in which dock bindings are the only bindings there are — the
        // user's own `timeoutlen` must still be honoured rather than silently
        // replaced by the compiled default.
        assert_eq!(timeoutlen_ms(None, Some(750)), 750);
        assert_eq!(timeoutlen_ms(None, Some(250)), 250);
    }

    #[test]
    fn timeoutlen_prefers_the_engine_when_one_is_reachable() {
        // `:set timeoutlen=300` typed at the command line must win over the
        // EditorSettings value it diverged from.
        assert_eq!(timeoutlen_ms(Some(300), Some(1000)), 300);
    }

    #[test]
    fn timeoutlen_falls_back_to_the_compiled_default_when_nothing_is_reachable() {
        assert_eq!(
            timeoutlen_ms(None, None),
            crate::settings::defaults::TIMEOUTLEN
        );
    }

    #[test]
    fn timeoutlen_is_clamped_to_the_same_bounds_as_the_engine() {
        use crate::settings::defaults;
        assert_eq!(timeoutlen_ms(None, Some(0)), defaults::TIMEOUTLEN_MIN);
        assert_eq!(timeoutlen_ms(None, Some(-1)), defaults::TIMEOUTLEN_MIN);
        assert_eq!(timeoutlen_ms(None, Some(999_999)), defaults::TIMEOUTLEN_MAX);
        assert_eq!(timeoutlen_ms(Some(0), None), defaults::TIMEOUTLEN_MIN);
    }

    // ── The registration guard this phase depends on ─────────────────

    #[test]
    fn a_sequence_cannot_be_bound_on_an_editor_reachable_surface() {
        // P5's guard, restated here because P8 *depends* on it: if `panel`
        // could hold `<C-w>s`, a pending buffer could be open while the script
        // editor has focus and `s` would be destroyed on the way to a bare
        // delete-char.
        let mut index = builtin_index(&registry());
        let mut diagnostics = Vec::new();
        crate::actions::bind::apply_text(
            &mut index,
            &registry(),
            "panelmap panel gg godotvim.focus.left\n\
             panelmap editor.nav gg godotvim.focus.left",
            &MappingOwner::User,
            "test",
            crate::actions::bind::Provenance::User,
            &mut diagnostics,
        );
        assert_eq!(diagnostics.len(), 2, "{diagnostics:?}");
        assert!(!index.reserves_any("panel"));
        assert!(!index.reserves_any("editor.nav"));
        assert_eq!(
            path_reserves(&index, &path("editor.nav", Caps::empty())),
            None
        );
    }

    // ── The consumption fold, end to end ─────────────────────────────

    #[test]
    fn a_completed_sequence_runs_through_the_ordinary_consumption_fold() {
        // The sequence layer produces `Candidate`s and nothing else: `dispose`
        // is unchanged, so `<void>`/`<norepeat>` mean the same thing at the
        // end of `gg` as they do on a bare key.
        let index = index_with("panelmap <void> dock gg godotvim.item.next");
        let reg = registry();
        let p = path("dock", TREE);
        let mut pending = Pending::default();
        press(&mut pending, &index, &reg, &p, ch('g'));
        let SeqStep::Run(candidates, matched) = press(&mut pending, &index, &reg, &p, ch('g'))
        else {
            panic!("gg must complete");
        };
        assert_eq!(matched, ch('g'));
        assert_eq!(
            dispose(&candidates, false, |_| Outcome::Declined),
            Disposition::Consume,
            "<void> still consumes at the end of a sequence"
        );
    }

    #[test]
    fn a_chord_can_be_reserved_too() {
        // Nothing about the layer is printable-only; `<C-x>` prefixes work,
        // and a chord is exempt from the type-to-search conflict entirely.
        let index = index_with("panelmap dock <C-x><C-f> godotvim.item.next");
        let reg = registry();
        let p = path("dock", TREE);
        let mut pending = Pending::default();
        assert_eq!(
            press(&mut pending, &index, &reg, &p, ctrl('x')),
            SeqStep::Buffered
        );
        let step = press(&mut pending, &index, &reg, &p, ctrl('f'));
        assert_eq!(ran(&step), Some("godotvim.item.next"));
    }
}
