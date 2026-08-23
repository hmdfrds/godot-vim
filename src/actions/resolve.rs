//! The resolver: stages S5 through S8 of the dispatch model, as a pure
//! function.
//!
//! Nothing here takes a `Gd<T>`, allocates a Godot object or calls a Godot
//! API. That is the entire point: this crate is a `cdylib` GDExtension and
//! cannot construct a `Gd<InputEventKey>` under `cargo test`, so every
//! decision that lives inside a Godot type is a decision verified by hand in
//! a running editor. Everything Godot-shaped happens in exactly two places —
//! [`super::surface::FocusChain::sample`] on the way in and
//! `set_input_as_handled()` on the way out — and both are the transport's
//! business, not this module's.
//!
//! The walk is **probe-major**: for each interpretation of the keystroke, in
//! priority order, the whole surface path is walked leaf→root, and the FIRST
//! exact trie hit that survives the capability gate wins. Probe priority
//! therefore outranks surface depth, which is the only reading that makes the
//! probe list mean what [`super::keys`] says it means — a guess about physical
//! position must never beat what the user actually typed, on any surface.
//! Five things that ordering encodes, each of which used to be an `if` in the
//! dispatcher:
//!
//! - **Depth is specificity, within one probe.** `dock.filesystem` is walked
//!   before `dock` because it declares `dock` as its parent, which is what
//!   gives the FileSystem keyset first refusal — replacing the hardcoded
//!   `if fs_result.is_consumed()` branch.
//! - **A typed probe outranks a positional one everywhere.** Probes 1–2 are
//!   pass 1 over every surface; probe 3 is pass 2 over every surface, and only
//!   against rules that individually asked for it with `<physical>`.
//! - **A capability miss is a declination that still claims the key.** The
//!   walk carries on to the parent within the same pass — which is how `h`/`l`
//!   go inert on an `ItemList` with no widget class named anywhere here — but
//!   it suppresses the positional pass entirely, because "this key means
//!   something here and this widget cannot do it" is an answer, not an
//!   invitation to reinterpret the keystroke.
//! - **`RuleTarget::Native` terminates the walk**, and is emphatically not an
//!   action that declines: a declining action would fall through to `panel`'s
//!   `Consumption::Void` rule and consume the key anyway, silently defeating
//!   the give-it-back-to-Godot escape hatch.
//! - **Consumption is computed downstream of the outcome**, from the winning
//!   rule's declared policy, never by the action.
//!
//! See `DESIGN-rebindable-nav.md` (design notes, kept outside this repo) §5.6 through §5.9.

use compact_str::CompactString;
use vim_core::keymap::{KeyEvent, MappingEntry, TrieLookup};

use super::action::{ActionId, ActionRegistry, ActionSpec, Params, RuleTarget};
use super::bind::{BindingIndex, Consumption, Repeat};
use super::caps::Caps;
use super::keys::Probes;
use super::outcome::Outcome;
use super::surface::{Seal, SurfaceId, SurfacePath};

/// What the transport does with the keystroke.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Disposition {
    /// Call `set_input_as_handled()` on THIS transport's viewport.
    Consume,
    /// Do not consume. Godot's own handling proceeds, which for the primary
    /// transport means the key goes on to `gui_input` and the engine.
    Ignore,
    /// Not consumed, and the plugin's own remaining stages on THIS transport
    /// are skipped. On `gui_input` that is the vim engine, so
    /// `CodeEdit::gui_input` moves its own popup selection. On `_input` there
    /// is no plugin stage below the resolver, and this variant is unreachable
    /// there by audit A9': the only verb that can produce it requires
    /// `Caps::POPUP`, which no classified path grants.
    Handoff,
}

/// Where a matched rule sends the keystroke.
#[derive(Debug, Clone)]
pub(crate) enum CandidateTarget {
    /// One of the plugin's own registered verbs. The spec is copied out of
    /// the registry so no registry borrow survives into `run`.
    Action(ActionId, &'static ActionSpec),
    /// One of Godot's own registered editor shortcuts, by path.
    Shortcut(CompactString),
}

/// Compared by identity, not structurally: `ActionSpec` holds a `fn` pointer,
/// which has no meaningful equality, and the dotted id is the stable name the
/// whole design addresses a verb by.
impl PartialEq for CandidateTarget {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Action(a, sa), Self::Action(b, sb)) => a == b && sa.id == sb.id,
            (Self::Shortcut(a), Self::Shortcut(b)) => a == b,
            _ => false,
        }
    }
}

impl Eq for CandidateTarget {}

/// One thing the resolver decided could run, with the policy that governs it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Candidate {
    /// The surface whose trie held the rule. Reported by `:panelmap`.
    pub(crate) surface: SurfaceId,
    pub(crate) target: CandidateTarget,
    pub(crate) params: Params,
    pub(crate) consume: Consumption,
    pub(crate) repeat: Repeat,
}

/// Why the walk produced nothing.
///
/// A reason rather than a bare `None` because the introspector's whole job is
/// to answer "why is my key dead?", and every one of these is a different
/// answer with a different fix.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Stop {
    /// The anchor surface is a `Barrier` — `foreign`, or the editor in an
    /// insert-like mode. Nothing is intercepted there, ever.
    Barrier,
    /// A `Sealed` anchor swallowed a BARE key, which then falls through to
    /// the control's own `gui_input`. A modifier-bearing key would have
    /// continued to the forest root.
    Sealed(SurfaceId),
    /// A rule said `native` on this surface: the key is Godot's.
    Native(SurfaceId),
    /// The anchor yields to the engine and the engine claims this key — the
    /// user's own `:map` wins.
    Yielded(KeyEvent),
    /// The whole path was walked and nothing matched.
    Exhausted,
}

/// The resolver's verdict.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Resolution {
    /// Nothing to run, and why.
    None(Stop),
    /// Run these, deepest surface first, until one does not decline.
    Run {
        /// The probe that produced the match. S6 is evaluated on THIS key and
        /// not on a re-derivation from the logical keycode: on a non-Latin
        /// layout the two differ, and asking about the wrong one is what used
        /// to deny Cyrillic users panel navigation from inside the editor.
        matched: KeyEvent,
        candidates: Vec<Candidate>,
    },
}

/// Everything the resolver reads. Borrowed, never owned, and all of it plain
/// data — a test constructs one from literals.
pub(crate) struct ResolveInput<'a> {
    pub(crate) probes: &'a Probes,
    pub(crate) path: &'a SurfacePath,
    pub(crate) index: &'a BindingIndex,
    pub(crate) registry: &'a ActionRegistry,
    /// `VimController::could_start_mapping`, inverted polarity included.
    ///
    /// The transport passes
    /// `|k| controller.as_ref().is_some_and(|c| c.could_start_mapping(k))`,
    /// which is the `is_none_or` at the old `input.rs` with the predicate
    /// negated on both sides. Getting that flip wrong stops the plugin
    /// navigating panels in exactly the state where nothing else can either:
    /// no controller must mean INTERCEPT.
    pub(crate) vim_claims: &'a dyn Fn(KeyEvent) -> bool,
}

/// Whether the vim engine claims `key` — the `vim_claims` predicate, with the
/// polarity flip written down once.
///
/// The old dispatcher read
/// `should_intercept = controller.is_none_or(|c| !c.could_start_mapping(k))`.
/// Negating both sides gives `claims = controller.is_some_and(|c|
/// c.could_start_mapping(k))`, and it is `is_some_and` — **not** `is_none_or`
/// — precisely because the predicate inside was negated too. Get it backwards
/// and "no controller" starts meaning "the engine claims everything", which
/// stops the plugin navigating panels in exactly the state where nothing else
/// can either: no script open, no controller attached, docks the only thing
/// on screen.
///
/// Generic over the controller so the polarity is testable without a Godot
/// runtime; the transport instantiates it at
/// `VimController::could_start_mapping`.
pub(crate) fn engine_claims<C>(
    controller: Option<&C>,
    key: KeyEvent,
    could_start_mapping: impl Fn(&C, KeyEvent) -> bool,
) -> bool {
    controller.is_some_and(|c| could_start_mapping(c, key))
}

/// Resolve one keystroke against one surface path.
pub(crate) fn resolve(input: &ResolveInput<'_>) -> Resolution {
    // S3 — the barrier is a total hard stop. No hook, no lookup, no ancestor.
    if input.path.seal == Seal::Barrier {
        return Resolution::None(Stop::Barrier);
    }

    let (candidates, matched) = match walk_path(input) {
        Ok(hit) => hit,
        Err(stop) => return Resolution::None(stop),
    };

    // S6 — the editor arbitration seam. One gate, on the anchor surface,
    // after resolution has produced a winner and before anything executes.
    if input.path.anchor_yields_to_engine && (input.vim_claims)(matched) {
        log::trace!("resolve: yielding {matched} to the vim engine");
        return Resolution::None(Stop::Yielded(matched));
    }

    Resolution::Run {
        matched,
        candidates,
    }
}

/// What one exact trie hit on one surface yields.
///
/// Extracted so the single-key walk here and the pending-sequence resolution
/// in [`super::sequence`] cannot drift. A capability gate that fired on one
/// path and not on the other would be a verb that runs as `gg` but not as `g`,
/// or the reverse — and the two would be indistinguishable from a broken
/// keyboard.
pub(super) enum Hit {
    /// Run this.
    Run(Candidate),
    /// `native` — hands the key back to Godot and terminates the walk.
    Native,
    /// Gated out by capabilities, or a trie entry naming nothing live.
    /// Treated exactly as `NoMatch`: the caller carries on.
    Miss,
}

/// Turn one live trie entry into a [`Hit`], applying the capability gate.
pub(super) fn hit_from(
    index: &BindingIndex,
    registry: &ActionRegistry,
    caps: Caps,
    surface: SurfaceId,
    entry: &MappingEntry,
) -> Hit {
    let Some(rule) = BindingIndex::slot_in(entry).and_then(|s| index.rule_at(s)) else {
        // A live trie entry with no live rule is a programming error in the
        // index, not a user-reachable state. Treat it as a miss rather than
        // consuming a key with nothing behind it.
        log::error!("resolve: {surface} has a trie entry with no live rule");
        return Hit::Miss;
    };
    match &rule.target {
        // Terminates. NOT a declining action: a declining action would fall
        // through to `panel`'s Void rule and consume.
        RuleTarget::Native => Hit::Native,
        RuleTarget::Action(id) => {
            let Some(spec) = registry.get(*id) else {
                log::error!("resolve: {surface} binds unregistered action id {}", id.0);
                return Hit::Miss;
            };
            // The capability gate, and the whole replacement for
            // `matches!(dock_kind, DockKind::Tree)`. A miss is skipped AS IF
            // NoMatch — the walk continues to the parent.
            if !caps.satisfies(spec.requires) {
                log::trace!(
                    "resolve: {} gated out on {surface} — needs {:?}, path offers {caps:?}",
                    spec.id,
                    spec.requires,
                );
                return Hit::Miss;
            }
            Hit::Run(Candidate {
                surface,
                target: CandidateTarget::Action(*id, spec),
                params: rule.params.clone(),
                consume: rule.consume,
                repeat: rule.repeat,
            })
        }
        // No `ActionSpec`, therefore no `requires`, therefore no capability
        // gate. Stated so it reads as a decision.
        RuleTarget::Shortcut(path) => Hit::Run(Candidate {
            surface,
            target: CandidateTarget::Shortcut(path.clone()),
            params: rule.params.clone(),
            consume: rule.consume,
            repeat: rule.repeat,
        }),
    }
}

/// The surfaces one keystroke may consult on this path, and whether the seal
/// is what limited them.
///
/// The seal is the deepest surface's, so it can only ever cut the walk at one
/// place: after the anchor. Computing the scope once, up front, is what lets
/// the surface loop be the INNER one in a probe-major walk without the seal
/// check firing once per probe. One rule, three behaviours: `<CR>` still
/// reaches the FS prompt's `text_submitted`, typing in a dock filter box still
/// types, and Ctrl+hjkl still escapes both.
///
/// Shared with [`super::sequence`] so the single-key walk and the reservation
/// walk cannot disagree about where a sealed anchor stops them — a key that
/// was swallowed by the seal at one length and not at the other would be
/// indistinguishable from a broken keyboard.
pub(super) fn walk_scope<'p>(input: &'p ResolveInput<'_>) -> (&'p [SurfaceId], bool) {
    if input.path.seal == Seal::Sealed && !input.probes.has_command_modifier() {
        return (input.path.ids.get(..1).unwrap_or_default(), true);
    }
    (&input.path.ids, false)
}

/// The leaf→root candidate walk (S5), in two probe-major passes.
///
/// **Probe-major, not surface-major**, and that ordering is the whole
/// function. The probe list is a priority list over *interpretations of one
/// keystroke* — what the user typed, its Latin collapse, then a guess about
/// where the key sits on a US keyboard — and the module header states plainly
/// that "a lower-priority interpretation can never shadow a higher-priority
/// one". Nesting the probe loop inside the surface loop broke exactly that:
/// the deepest surface got all three probes before the next surface got any,
/// so the guess won on a deep surface over the typed key on a shallow one.
/// Colemak `j` in the FileSystem dock is the case that costs a user data —
/// probe 3 is `y`, `dock.filesystem y` is `godotvim.fs.yank_path`, and the
/// most-used navigation key in the keyset silently wrote the clipboard.
///
/// Two passes rather than one loop over `iter()`, because the passes are not
/// symmetric:
///
/// - Pass 1 (probes 1–2) is the user's actual keystroke. A capability MISS
///   here is a *declination that still claims the key*: `dock h` is
///   `godotvim.item.collapse`, an `ItemList` grants no `HIERARCHY`, and the
///   design says the key goes inert. Falling through to pass 2 instead
///   promotes the positional guess and moves the selection.
/// - Pass 2 is the guess, offered only where a rule asked for it — and asked
///   for it **per rule**, via that rule's own `<physical>`, not per surface.
///   A surface-wide test meant one flagged rule on `dock` opened probe 3 for
///   every other rule on `dock` too, including every rule a user added.
fn walk_path(input: &ResolveInput<'_>) -> Result<(Vec<Candidate>, KeyEvent), Stop> {
    let (scope, sealed) = walk_scope(input);
    let exhausted = || match (sealed, input.path.ids.first()) {
        (true, Some(&anchor)) => Stop::Sealed(anchor),
        _ => Stop::Exhausted,
    };

    // ── Pass 1: what the user typed, leaf→root ───────────────────────
    //
    // `typed_claimed` records that some typed probe found a rule and the
    // capability gate turned it down. That is a real answer — "this key means
    // something here and this widget cannot do it" — so pass 2 is skipped and
    // the key stays inert rather than being reinterpreted by position.
    let mut typed_claimed = false;
    for probe in input.probes.iter_typed() {
        for &surface in scope {
            let TrieLookup::ExactOnly(entry) = input.index.lookup(surface, &[probe]) else {
                // `Prefix` belongs to [`super::sequence`], which runs BEFORE
                // this walk and has already decided whether the key is
                // reserved; reaching a `Prefix` here means it was not, so the
                // key is simply unbound at this length. `NoMatch` and any
                // future variant are misses too.
                continue;
            };
            match hit_from(input.index, input.registry, input.path.caps, surface, entry) {
                Hit::Miss => typed_claimed = true,
                Hit::Native => return Err(Stop::Native(surface)),
                Hit::Run(candidate) => return Ok((vec![candidate], probe)),
            }
        }
    }

    // ── Pass 2: the US-QWERTY positional guess ───────────────────────
    if typed_claimed || input.path.anchor_refuses_positional {
        return Err(exhausted());
    }
    let Some(probe) = input.probes.positional() else {
        return Err(exhausted());
    };
    for &surface in scope {
        // A cheap bail-out and nothing more: it answers "could any rule here
        // want this?" over ~5 entries, so the per-rule test below is only
        // reached on the surfaces that carry a flag at all. The per-rule test
        // is the one that decides.
        if !input.index.has_physical_rule(surface) {
            continue;
        }
        let TrieLookup::ExactOnly(entry) = input.index.lookup(surface, &[probe]) else {
            continue;
        };
        let asked = BindingIndex::slot_in(entry)
            .and_then(|slot| input.index.rule_at(slot))
            .is_some_and(|rule| rule.physical);
        if !asked {
            continue;
        }
        match hit_from(input.index, input.registry, input.path.caps, surface, entry) {
            Hit::Miss => continue,
            Hit::Native => return Err(Stop::Native(surface)),
            Hit::Run(candidate) => return Ok((vec![candidate], probe)),
        }
    }
    Err(exhausted())
}

/// S7 and S8 — run the plan and compute consumption from it.
///
/// `run` is injected so the fold is testable without a Godot runtime; the
/// transport passes a closure that builds an `ActionCtx` and calls the spec.
///
/// Consumption is computed **downstream** of the outcome, from the rule's
/// declared policy — never by the action, which is the fifth joint the old
/// match arms fused.
pub(crate) fn dispose(
    candidates: &[Candidate],
    is_echo: bool,
    mut run: impl FnMut(&Candidate) -> Outcome,
) -> Disposition {
    for candidate in candidates {
        if is_echo && candidate.repeat == Repeat::Suppress {
            // Consume WITHOUT running. Returning `Ignore` would leak the
            // repeated Ctrl+J to Godot's own handling, which today's
            // unconditional `set_input_as_handled()` never does; running it
            // would queue a ~20/s storm of deferred `grab_focus` calls.
            log::trace!("dispose: echo suppressed on {}", candidate.surface);
            return Disposition::Consume;
        }
        let outcome = run(candidate);
        if candidate.consume == Consumption::Void {
            // Consumes AND terminates, even on `Declined`, even when `run`
            // short-circuited because there was no target at all. This is the
            // declarative form of the old `input.rs`, where
            // `handle_window_nav`'s result was discarded and the key consumed
            // regardless. Making it conditional leaks Ctrl+H/J/K/L to Godot.
            //
            // Tested BEFORE the outcome, so `<void>` keeps meaning "consume
            // regardless and terminate" on every surface; `Handoff` is tested
            // INSIDE the `is_consumed()` branch, so a declining `Handoff`
            // candidate falls through elastically instead of eating the key.
            return Disposition::Consume;
        }
        if outcome.is_consumed() {
            // Written as an exhaustive match so a fourth policy is a compile
            // error here rather than a silent `Consume`.
            return match candidate.consume {
                Consumption::Handoff => Disposition::Handoff,
                Consumption::Elastic | Consumption::Void => Disposition::Consume,
            };
        }
        // Declined + Elastic → the next candidate, and if there is none, the
        // key is not consumed and reaches Godot. That is what preserves `j`
        // at the end of a list and Enter with nothing selected.
    }
    Disposition::Ignore
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::actions::bind::{builtin_index, Rule};
    use crate::actions::caps::Caps;
    use crate::actions::specs;
    use crate::actions::surface::{Anchor, Forest, SurfacePath};
    use vim_core::keymap::{Key as VimKey, MappingOwner, Modifiers};

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

    fn named(k: VimKey) -> KeyEvent {
        KeyEvent::new(k, Modifiers::NONE)
    }

    fn probes(keys: &[KeyEvent]) -> Probes {
        Probes::from_slice(keys)
    }

    /// A path as the forest would produce it, with the caps spelled out.
    fn path(leaf: &'static str, caps: Caps) -> SurfacePath {
        let forest = crate::actions::providers::forest();
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

    const NEVER: &dyn Fn(KeyEvent) -> bool = &|_| false;
    const ALWAYS: &dyn Fn(KeyEvent) -> bool = &|_| true;

    fn run_on(p: &SurfacePath, keys: &[KeyEvent], claims: &dyn Fn(KeyEvent) -> bool) -> Resolution {
        resolve_with(p, &probes(keys), claims)
    }

    /// As `run_on`, but with the LAST probe marked positional — the shape a
    /// Dvorak / Colemak / AZERTY / QWERTZ keystroke really produces.
    fn run_positional(
        p: &SurfacePath,
        keys: &[KeyEvent],
        claims: &dyn Fn(KeyEvent) -> bool,
    ) -> Resolution {
        resolve_with(p, &Probes::from_slice_positional(keys), claims)
    }

    fn resolve_with(
        p: &SurfacePath,
        probes: &Probes,
        claims: &dyn Fn(KeyEvent) -> bool,
    ) -> Resolution {
        let index = builtin_index(&registry());
        let reg = registry();
        resolve(&ResolveInput {
            probes,
            path: p,
            index: &index,
            registry: &reg,
            vim_claims: claims,
        })
    }

    fn action_of(res: &Resolution) -> Option<&'static str> {
        match res {
            Resolution::Run { candidates, .. } => match candidates.first()?.target {
                CandidateTarget::Action(_, spec) => Some(spec.id),
                CandidateTarget::Shortcut(_) => None,
            },
            Resolution::None(_) => None,
        }
    }

    fn stop_of(res: &Resolution) -> Option<Stop> {
        match res {
            Resolution::None(stop) => Some(*stop),
            Resolution::Run { .. } => None,
        }
    }

    const TREE: Caps = Caps::VNAV.union(Caps::HIERARCHY).union(Caps::ACTIVATE);
    const LIST: Caps = Caps::VNAV.union(Caps::ACTIVATE);

    // ── The precedence table (§5.9), one row at a time ───────────────

    #[test]
    fn row3_a_barrier_resolves_nothing() {
        // `foreign` and `editor.insert`. Ctrl+H is backspace in Insert mode
        // and belongs to whatever text input has focus in a foreign control.
        for leaf in ["foreign", "editor.insert"] {
            let p = path(leaf, Caps::TEXTENTRY);
            assert_eq!(
                stop_of(&run_on(&p, &[ctrl('h')], NEVER)),
                Some(Stop::Barrier),
                "{leaf} must intercept nothing"
            );
        }
    }

    #[test]
    fn row4_the_deepest_surface_wins() {
        // `d` is bound on dock.filesystem and nowhere else; `j` is bound on
        // `dock`, the parent, and the walk reaches it. That depth IS the
        // FileSystem-first refusal the old `if fs_result.is_consumed()` gave.
        let fs = path("dock.filesystem", TREE | Caps::FILEOPS);
        assert_eq!(
            action_of(&run_on(&fs, &[ch('d')], NEVER)),
            Some("godotvim.fs.delete")
        );
        assert_eq!(
            action_of(&run_on(&fs, &[ch('j')], NEVER)),
            Some("godotvim.item.next")
        );
    }

    #[test]
    fn row4_a_plain_dock_never_reaches_the_filesystem_keyset() {
        let dock = path("dock", TREE);
        assert_eq!(
            stop_of(&run_on(&dock, &[ch('d')], NEVER)),
            Some(Stop::Exhausted)
        );
        assert_eq!(
            stop_of(&run_on(&dock, &[ch('a')], NEVER)),
            Some(Stop::Exhausted)
        );
    }

    #[test]
    fn row5_the_as_typed_probe_beats_the_positional_one() {
        // The user typed `/`; a physical position of `j` must not win. This
        // is the shadowing bug that motivated one probe list per keyset.
        let dock = path("dock", TREE);
        assert_eq!(
            action_of(&run_positional(&dock, &[ch('/'), ch('j')], NEVER)),
            Some("godotvim.dock.search")
        );
    }

    #[test]
    fn row5_a_later_probe_recovers_a_non_latin_layout() {
        let dock = path("dock", TREE);
        assert_eq!(
            action_of(&run_on(&dock, &[ch('о'), ch('j')], NEVER)),
            Some("godotvim.item.next")
        );
    }

    #[test]
    fn row5_the_positional_probe_is_offered_only_where_a_rule_asks_for_it() {
        // Both halves, because the name claims both. `dock j` carries
        // `<physical>`, so a QWERTZ `z` at the QWERTY-`j` position still
        // moves down…
        let dock = path("dock", TREE);
        assert_eq!(
            action_of(&run_positional(&dock, &[ch('z'), ch('j')], NEVER)),
            Some("godotvim.item.next")
        );
        // …while `dock <CR>` does not, so a positional Enter cannot
        // synthesize an activation the user never pressed. A named key never
        // produces probe 3 at all (`probes_from_parts` requires
        // `Key::Char`), so the guard is spelled with a character instead: a
        // rule the user added without `<physical>` must stay unreachable by
        // position even on a surface where OTHER rules asked for it.
        let mut index = builtin_index(&registry());
        index.upsert(unphysical_rule("dock", ch('q')));
        assert_eq!(
            stop_of(&resolve_on(
                &index,
                &dock,
                &Probes::from_slice_positional(&[ch('z'), ch('q')])
            )),
            Some(Stop::Exhausted),
            "`panelmap dock q …` never asked for the positional probe"
        );
    }

    /// A rule with every flag off — the shape a bare `panelmap <surface> <key>
    /// <action>` line produces, and therefore the shape that must NOT be
    /// reachable by physical position.
    fn unphysical_rule(surface: SurfaceId, key: KeyEvent) -> Rule {
        Rule {
            surface,
            lhs: vec![key],
            target: RuleTarget::Action(
                specs::registry()
                    .id_of("godotvim.item.activate")
                    .expect("shipped verb"),
            ),
            params: Params::new(),
            consume: Consumption::Elastic,
            repeat: Repeat::Allow,
            physical: false,
            shift_tolerant: false,
            nowait: false,
            owner: MappingOwner::User,
            desc: "activate".into(),
        }
    }

    /// Resolve against a caller-supplied index, so a test can add one rule.
    fn resolve_on(index: &BindingIndex, p: &SurfacePath, probes: &Probes) -> Resolution {
        let reg = registry();
        resolve(&ResolveInput {
            probes,
            path: p,
            index,
            registry: &reg,
            vim_claims: NEVER,
        })
    }

    #[test]
    fn a_surface_with_no_physical_rule_never_sees_the_positional_probe() {
        // `dock.debugger` binds `y` to `godotvim.debugger.yank_frame` and
        // carries no `<physical>` anywhere, so a Dvorak/Colemak `y` position
        // must not reach it — even though its PARENT `dock` is full of
        // `<physical>` rules. Fails under the mutation
        // `let positional = anchor_allows;`.
        let debugger = path("dock.debugger", TREE);
        assert_eq!(
            stop_of(&run_positional(&debugger, &[ch('f'), ch('y')], NEVER)),
            Some(Stop::Exhausted),
            "nothing on dock.debugger asked for the positional probe"
        );
    }

    #[test]
    fn row6_a_sealed_anchor_swallows_a_bare_key_and_passes_a_chord() {
        let search = path("searchbox", Caps::TEXTENTRY);
        // Typing into the filter box must reach the LineEdit.
        assert_eq!(
            stop_of(&run_on(&search, &[ch('x')], NEVER)),
            Some(Stop::Sealed("searchbox"))
        );
        // Ctrl+hjkl escapes a filter box unconditionally.
        assert_eq!(
            action_of(&run_on(&search, &[ctrl('l')], NEVER)),
            Some("godotvim.focus.right")
        );
    }

    #[test]
    fn row6_the_prompt_is_sealed_with_no_rules_at_all() {
        // Bare `<CR>` stays unbound so `text_submitted` still fires, and bare
        // `<Esc>` reaches the prompt's own `gui_input` transport.
        let prompt = path("prompt", Caps::TEXTENTRY);
        for key in [named(VimKey::Enter), named(VimKey::Escape), ch('a')] {
            assert_eq!(
                stop_of(&run_on(&prompt, &[key], NEVER)),
                Some(Stop::Sealed("prompt")),
                "{key} must reach the prompt LineEdit"
            );
        }
        assert_eq!(
            action_of(&run_on(&prompt, &[ctrl('k')], NEVER)),
            Some("godotvim.focus.up")
        );
    }

    #[test]
    fn row7_a_capability_miss_skips_the_candidate_and_walks_on() {
        // `l` needs HIERARCHY, which an ItemList does not offer. No widget
        // class is named anywhere in the resolver to make this happen.
        let list = path("dock", LIST);
        assert_eq!(
            stop_of(&run_on(&list, &[ch('l')], NEVER)),
            Some(Stop::Exhausted)
        );
        let tree = path("dock", TREE);
        assert_eq!(
            action_of(&run_on(&tree, &[ch('l')], NEVER)),
            Some("godotvim.item.expand")
        );
    }

    #[test]
    fn row7_a_rich_text_label_keeps_vertical_navigation() {
        // The docs panel and the Output log are focusable RichTextLabels and
        // j/k scroll them today. A "has a list" capability would have killed
        // both silently.
        let docs = path("dock", Caps::VNAV);
        assert_eq!(
            action_of(&run_on(&docs, &[ch('j')], NEVER)),
            Some("godotvim.item.next")
        );
        // …but Enter has nothing to activate there.
        assert_eq!(
            stop_of(&run_on(&docs, &[named(VimKey::Enter)], NEVER)),
            Some(Stop::Exhausted)
        );
    }

    #[test]
    fn row10_native_terminates_the_walk_instead_of_falling_through() {
        // The distinction the design calls load-bearing: modelled as a
        // declining action, `native` would fall through to `panel`'s Void
        // rule and consume the key anyway.
        let mut index = builtin_index(&registry());
        index.upsert(Rule {
            surface: "dock",
            lhs: vec![ctrl('h')],
            target: RuleTarget::Native,
            params: Params::new(),
            consume: Consumption::Elastic,
            repeat: Repeat::Allow,
            physical: false,
            shift_tolerant: false,
            nowait: false,
            owner: MappingOwner::User,
            desc: "give it back".into(),
        });
        let reg = registry();
        let p = path("dock", TREE);
        let probes = probes(&[ctrl('h')]);
        let res = resolve(&ResolveInput {
            probes: &probes,
            path: &p,
            index: &index,
            registry: &reg,
            vim_claims: NEVER,
        });
        assert_eq!(stop_of(&res), Some(Stop::Native("dock")));
    }

    #[test]
    fn row11_the_editor_yields_a_key_the_engine_claims() {
        let editor = path("editor.nav", Caps::empty());
        // Without a user mapping the panel chord wins.
        assert_eq!(
            action_of(&run_on(&editor, &[ctrl('h')], NEVER)),
            Some("godotvim.focus.left")
        );
        // With one, the user's `:map` wins and the key flows to gui_input.
        assert_eq!(
            stop_of(&run_on(&editor, &[ctrl('h')], ALWAYS)),
            Some(Stop::Yielded(ctrl('h')))
        );
    }

    #[test]
    fn row11_arbitration_is_evaluated_on_the_key_that_matched() {
        // Cyrillic: probe 1 is `<C-х>`, probe 2 recovers `<C-h>`. The engine
        // must be asked about `<C-h>` — the one we would consume — not about
        // the raw logical key, which is what used to deny Cyrillic users
        // panel navigation from inside the editor.
        let editor = path("editor.nav", Caps::empty());
        let claims_h: &dyn Fn(KeyEvent) -> bool = &|k| k == ctrl('h');
        assert_eq!(
            stop_of(&run_on(&editor, &[ctrl('х'), ctrl('h')], claims_h)),
            Some(Stop::Yielded(ctrl('h')))
        );
    }

    #[test]
    fn row11_only_the_editor_ever_asks_the_engine() {
        // A key pressed while a Tree has focus is none of the engine's
        // business, even when the engine would claim it.
        let dock = path("dock", TREE);
        assert_eq!(
            action_of(&run_on(&dock, &[ctrl('h')], ALWAYS)),
            Some("godotvim.focus.left")
        );
    }

    // ── The arbitration polarity ─────────────────────────────────────

    #[test]
    fn no_controller_means_intercept() {
        // THE flip. `is_none_or(|c| !claims)` inverted is
        // `is_some_and(|c| claims)`, and writing `is_none_or` here instead
        // would make a detached plugin yield every panel chord to an engine
        // that is not there — no script open, docks the only thing on screen,
        // and Ctrl+hjkl dead.
        let never_asked = |_: &(), _: KeyEvent| panic!("must not consult a missing controller");
        assert!(!engine_claims(None::<&()>, ctrl('h'), never_asked));
    }

    #[test]
    fn a_controller_that_claims_the_key_wins() {
        assert!(engine_claims(Some(&()), ctrl('h'), |_, _| true));
        assert!(!engine_claims(Some(&()), ctrl('h'), |_, _| false));
    }

    #[test]
    fn arbitration_asks_about_the_matched_key_only() {
        // `could_start_mapping` covers prefixes as well as exact matches
        // (`TrieLookup::Prefix != NoMatch`), so a user with
        // `:nnoremap <C-h><C-h> …` keeps the key. The predicate is opaque to
        // the resolver; what this pins is that it is called with the key that
        // MATCHED and with nothing else.
        let editor = path("editor.nav", Caps::empty());
        let seen = std::cell::RefCell::new(Vec::new());
        let record: &dyn Fn(KeyEvent) -> bool = &|k| {
            seen.borrow_mut().push(k);
            false
        };
        run_on(&editor, &[ctrl('h')], record);
        assert_eq!(seen.into_inner(), vec![ctrl('h')]);
    }

    #[test]
    fn a_barrier_never_asks_the_engine_at_all() {
        // Insert mode is a barrier, and Ctrl+H there is backspace. Asking the
        // engine would be harmless; resolving at all would not be.
        let insert = path("editor.insert", Caps::empty());
        let never = |_: &(), _: KeyEvent| panic!("a barrier must not resolve");
        assert!(!engine_claims(None::<&()>, ctrl('h'), never));
        assert_eq!(
            stop_of(&run_on(&insert, &[ctrl('h')], ALWAYS)),
            Some(Stop::Barrier)
        );
    }

    // ── The positional refusal (the P1 regression guard) ─────────────

    #[test]
    fn the_editor_refuses_the_positional_probe() {
        // THE Dvorak guard. The QWERTY-H position emits `d`, so a Dvorak user
        // pressing Ctrl+d produces probes [<C-d>, <C-h>]. Honouring the
        // positional probe here converts half-page-down into panel-left.
        let editor = path("editor.nav", Caps::empty());
        assert_eq!(
            stop_of(&run_positional(&editor, &[ctrl('d'), ctrl('h')], NEVER)),
            Some(Stop::Exhausted),
            "Ctrl+d in the editor must stay half-page-down"
        );
        // Colemak does the same to Ctrl+n (jump-forward) and Ctrl+e (scroll).
        for chord in [ctrl('n'), ctrl('e')] {
            assert_eq!(
                stop_of(&run_positional(&editor, &[chord, ctrl('j')], NEVER)),
                Some(Stop::Exhausted),
                "{chord} in the editor must reach the engine"
            );
        }
        // Every other surface honours it — that is what gives a Dvorak user
        // cross-panel navigation by position from a dock.
        let dock = path("dock", TREE);
        assert_eq!(
            action_of(&run_positional(&dock, &[ctrl('d'), ctrl('h')], NEVER)),
            Some("godotvim.focus.left")
        );
    }

    #[test]
    fn the_editor_still_gets_the_latin_probe() {
        // Refusing probe 3 must not cost probe 2: a Cyrillic Ctrl+х carries
        // `latin_key`, which collapses to `<C-h>` as an ordinary (non
        // positional) probe, so panel navigation from the editor survives.
        let editor = path("editor.nav", Caps::empty());
        assert_eq!(
            action_of(&run_on(&editor, &[ctrl('х'), ctrl('h')], NEVER)),
            Some("godotvim.focus.left")
        );
    }

    // ── Consumption (§5.8) ───────────────────────────────────────────

    fn void_plan() -> Vec<Candidate> {
        vec![Candidate {
            surface: "panel",
            target: CandidateTarget::Action(ActionId(0), &specs::FOCUS_LEFT),
            params: Params::new(),
            consume: Consumption::Void,
            repeat: Repeat::Suppress,
        }]
    }

    fn elastic_plan() -> Vec<Candidate> {
        vec![Candidate {
            surface: "dock",
            target: CandidateTarget::Action(ActionId(0), &specs::ITEM_NEXT),
            params: Params::new(),
            consume: Consumption::Elastic,
            repeat: Repeat::Allow,
        }]
    }

    fn handoff_plan() -> Vec<Candidate> {
        vec![Candidate {
            surface: "editor.completion",
            target: CandidateTarget::Action(ActionId(0), &specs::ITEM_NEXT),
            params: Params::new(),
            consume: Consumption::Handoff,
            repeat: Repeat::Suppress,
        }]
    }

    #[test]
    fn handoff_hands_the_key_to_the_control_on_acceptance() {
        // The third routing state: engine skipped, event NOT consumed, the
        // control's own gui_input gets the key. This is what Up/Down need so
        // CodeEdit moves its own popup selection.
        assert_eq!(
            dispose(&handoff_plan(), false, |_| Outcome::Handled),
            Disposition::Handoff
        );
    }

    #[test]
    fn handoff_falls_through_on_a_declination() {
        // A declining Handoff candidate behaves exactly as Elastic: the walk
        // continues, and with nothing left the key is not consumed.
        let mut plan = handoff_plan();
        plan.extend(elastic_plan());
        let mut ran = Vec::new();
        let d = dispose(&plan, false, |c| {
            ran.push(c.surface);
            Outcome::Declined
        });
        assert_eq!(ran, vec!["editor.completion", "dock"], "both must run");
        assert_eq!(d, Disposition::Ignore);
    }

    #[test]
    fn void_beats_handoff() {
        // A candidate declaring both is impossible: `<void>` wins in
        // `rule_from`. At the dispose level, a Void candidate ahead of a
        // Handoff one terminates the walk with Consume.
        let mut plan = void_plan();
        plan.extend(handoff_plan());
        let mut ran = 0;
        let d = dispose(&plan, false, |_| {
            ran += 1;
            Outcome::Handled
        });
        assert_eq!(d, Disposition::Consume);
        assert_eq!(ran, 1, "Void terminates before the Handoff candidate");
    }

    #[test]
    fn a_suppressed_echo_beats_handoff() {
        // The echo suppression arm answers before the consumption policy is
        // read, so a held key on a norepeat Handoff rule is swallowed rather
        // than handed to the control ~20 times a second.
        let mut ran = 0;
        let d = dispose(&handoff_plan(), true, |_| {
            ran += 1;
            Outcome::Handled
        });
        assert_eq!(d, Disposition::Consume);
        assert_eq!(ran, 0);
    }

    #[test]
    fn void_consumes_even_when_the_action_declines() {
        // The no-focus-owner case, verbatim: `handle_window_nav` was never
        // called and `set_input_as_handled()` fired anyway. Making this
        // conditional leaks Ctrl+H/J/K/L to Godot.
        let mut ran = 0;
        let d = dispose(&void_plan(), false, |_| {
            ran += 1;
            Outcome::Declined
        });
        assert_eq!(d, Disposition::Consume);
        assert_eq!(ran, 1, "Void still runs the action; it ignores the answer");
    }

    #[test]
    fn elastic_does_not_consume_a_declination() {
        // `j` at the end of an ItemList: Godot's own type-to-search and
        // arrow handling must still see the key.
        let d = dispose(&elastic_plan(), false, |_| Outcome::Declined);
        assert_eq!(d, Disposition::Ignore);
    }

    #[test]
    fn elastic_consumes_an_acceptance() {
        for outcome in [Outcome::Handled, Outcome::FocusChanged] {
            assert_eq!(
                dispose(&elastic_plan(), false, |_| outcome),
                Disposition::Consume
            );
        }
    }

    #[test]
    fn declination_falls_through_to_the_next_candidate() {
        let mut plan = elastic_plan();
        plan.extend(void_plan());
        let mut ran = Vec::new();
        let d = dispose(&plan, false, |c| {
            ran.push(c.surface);
            Outcome::Declined
        });
        assert_eq!(ran, vec!["dock", "panel"], "the walk must continue");
        assert_eq!(d, Disposition::Consume, "…and Void still terminates it");
    }

    #[test]
    fn an_exhausted_plan_consumes_nothing() {
        assert_eq!(
            dispose(&[], false, |_| Outcome::Handled),
            Disposition::Ignore
        );
    }

    #[test]
    fn a_suppressed_echo_consumes_without_running() {
        let mut ran = 0;
        let d = dispose(&void_plan(), true, |_| {
            ran += 1;
            Outcome::Handled
        });
        assert_eq!(d, Disposition::Consume, "an echo must not leak to Godot");
        assert_eq!(ran, 0, "…and must not queue another grab_focus");
    }

    #[test]
    fn an_allowed_echo_runs_normally() {
        // Held `j`/`k` auto-repeat in a dock is desirable and is preserved.
        let mut ran = 0;
        let d = dispose(&elastic_plan(), true, |_| {
            ran += 1;
            Outcome::Handled
        });
        assert_eq!(d, Disposition::Consume);
        assert_eq!(ran, 1);
    }

    // ── The full shipped default set, resolved ───────────────────────

    #[test]
    fn the_four_panel_chords_resolve_from_every_non_barrier_surface() {
        let wanted = [
            (ctrl('h'), "godotvim.focus.left"),
            (ctrl('j'), "godotvim.focus.down"),
            (ctrl('k'), "godotvim.focus.up"),
            (ctrl('l'), "godotvim.focus.right"),
        ];
        for leaf in [
            "dock.filesystem",
            "dock",
            "searchbox",
            "prompt",
            "unknown",
            "panel",
            "editor.nav",
        ] {
            let p = path(leaf, Caps::all());
            for (key, id) in wanted {
                assert_eq!(
                    action_of(&run_on(&p, &[key], NEVER)),
                    Some(id),
                    "{key} must reach {id} from {leaf}"
                );
            }
        }
    }

    #[test]
    fn the_panel_chords_are_void_and_norepeat_wherever_they_resolve() {
        let p = path("unknown", Caps::empty());
        let Resolution::Run { candidates, .. } = run_on(&p, &[ctrl('h')], NEVER) else {
            panic!("Ctrl+h must resolve with no focus owner at all");
        };
        assert_eq!(candidates[0].consume, Consumption::Void);
        assert_eq!(candidates[0].repeat, Repeat::Suppress);
        assert_eq!(candidates[0].surface, "panel");
    }

    #[test]
    fn the_filesystem_keyset_needs_fileops() {
        // Without the grant, `panelmap dock a godotvim.fs.create` on a Scene
        // tree would create files at res:// root.
        let no_grant = SurfacePath {
            caps: TREE,
            ..path("dock.filesystem", TREE)
        };
        for key in ['a', 'd', 'r', 'y', 'R'] {
            assert_eq!(
                stop_of(&run_on(&no_grant, &[ch(key)], NEVER)),
                Some(Stop::Exhausted),
                "{key} must be gated out without FILEOPS"
            );
        }
    }

    #[test]
    fn the_search_box_tolerates_shift_where_a_dock_does_not() {
        let search = path("searchbox", Caps::TEXTENTRY);
        let shift_enter = KeyEvent::new(VimKey::Enter, Modifiers::SHIFT);
        assert_eq!(
            action_of(&run_on(&search, &[shift_enter], NEVER)),
            Some("godotvim.search.accept")
        );
        let dock = path("dock", TREE);
        assert_eq!(
            stop_of(&run_on(&dock, &[shift_enter], NEVER)),
            Some(Stop::Exhausted),
            "Shift+Enter is inert in a dock"
        );
    }

    // ── The retired P0 oracles, restated against the live path ───────
    //
    // P0's characterization suite pinned three hand-written decision tables —
    // `dock_action_for`, `direction_for` and `fs_action_for` in
    // `crate::navigation` — that the binding index replaced. Those tables are
    // gone. Most of what they asserted was already stated above against
    // `resolve`; what was not is stated here, so no behaviour lost its only
    // test when the oracles were deleted.

    #[test]
    fn a_dock_binds_no_modified_key_of_its_own() {
        // `dock_action_for` opened with `if modifiers() != NONE { return None }`.
        // Live that is not a guard but an absence: the `dock` surface binds
        // seven BARE keys and nothing else, so a modified `j` walks past it.
        let dock = path("dock", TREE);
        for m in [
            Modifiers::ALT,
            Modifiers::META,
            Modifiers::SHIFT,
            Modifiers::CTRL | Modifiers::SHIFT,
            Modifiers::CTRL | Modifiers::ALT,
        ] {
            assert_eq!(
                stop_of(&run_on(
                    &dock,
                    &[KeyEvent::new(VimKey::Char('j'), m)],
                    NEVER
                )),
                Some(Stop::Exhausted),
                "{m:?}+j must not navigate a dock"
            );
        }
        // The one deliberate divergence from the old table, which answered
        // `None` for every modifier alike: `Ctrl+j` is not the dock's key, it
        // is `panel`'s, and the walk is what reaches it.
        assert_eq!(
            action_of(&run_on(&dock, &[ctrl('j')], NEVER)),
            Some("godotvim.focus.down")
        );
    }

    #[test]
    fn every_dock_key_is_tried_at_each_probe_before_the_next_probe_runs() {
        // The `/`-shadowing bug, restated per key. Under the old per-arm
        // fallback the hjkl arm consulted the physical position and returned
        // before the arm owning `/` was reached, so on a layout whose QWERTY-J
        // position types `/` the filter box was unreachable. `row5_*` covers
        // the `/`-over-`j` pair; these are the rest of the keyset, including
        // the two named keys, which a positional probe can never synthesize
        // and which therefore only ever lead.
        let dock = path("dock", TREE);
        for (keys, want) in [
            (vec![ch('j'), ch('/')], "godotvim.item.next"),
            (vec![ch('ю'), ch('/')], "godotvim.dock.search"),
            (
                vec![named(VimKey::Escape), ch('j')],
                "godotvim.focus.editor",
            ),
            (
                vec![named(VimKey::Enter), ch('l')],
                "godotvim.item.activate",
            ),
        ] {
            assert_eq!(
                action_of(&run_on(&dock, &keys, NEVER)),
                Some(want),
                "{keys:?}"
            );
        }
    }

    #[test]
    fn a_filter_box_swallows_typing_and_breaks_its_seal_only_for_a_chord() {
        let search = path("searchbox", Caps::TEXTENTRY);
        // `<CR>` and `<Esc>` leave the box, keeping the filter text.
        for key in [VimKey::Enter, VimKey::Escape] {
            assert_eq!(
                action_of(&run_on(&search, &[named(key)], NEVER)),
                Some("godotvim.search.accept"),
                "{key:?} must leave the filter box"
            );
        }
        // Everything else reaches the LineEdit — `j` and `/` included. They
        // are the dock's keys, and the seal is the whole reason they do not
        // fire in the box the user is typing a filter into.
        for key in [ch('a'), ch('j'), ch('/'), named(VimKey::Backspace)] {
            assert_eq!(
                stop_of(&run_on(&search, &[key], NEVER)),
                Some(Stop::Sealed("searchbox")),
                "{key} must reach the LineEdit"
            );
        }
        // Ctrl/Alt/Meta break the seal — and then match nothing, rather than
        // accepting the filter. This asymmetry is what `CMD_MODS` means, and
        // until `keys::cmd_mods_is_ctrl_alt_and_meta_but_never_shift` existed
        // the ALT and META bits of that constant were pinned by exactly one
        // test, which was an oracle test.
        for m in [Modifiers::CTRL, Modifiers::ALT, Modifiers::META] {
            assert_eq!(
                stop_of(&run_on(&search, &[KeyEvent::new(VimKey::Escape, m)], NEVER)),
                Some(Stop::Exhausted),
                "{m:?}+Esc must not accept the filter"
            );
        }
    }

    #[test]
    fn the_four_panel_chords_are_the_only_chords_bound_anywhere() {
        // `direction_for` answered `Some` only for Ctrl+hjkl, and only when
        // Ctrl was the sole modifier. Live, both halves are one question:
        // does any OTHER chord resolve? Asked from `dock`, whose walk passes
        // through `panel`, where all four live.
        let dock = path("dock", TREE);
        for c in ['a', 'z', '/', '1', 'd', 'w'] {
            assert_eq!(
                stop_of(&run_on(&dock, &[ctrl(c)], NEVER)),
                Some(Stop::Exhausted),
                "Ctrl+{c} is not a panel chord"
            );
        }
        assert_eq!(
            stop_of(&run_on(
                &dock,
                &[KeyEvent::new(VimKey::Enter, Modifiers::CTRL)],
                NEVER
            )),
            Some(Stop::Exhausted),
            "Ctrl+Enter is not a panel chord"
        );
        // Probe 1 is authoritative even when a later probe is also a chord —
        // that is what makes an OS-level remap take effect.
        assert_eq!(
            action_of(&run_on(&dock, &[ctrl('j'), ctrl('l')], NEVER)),
            Some("godotvim.focus.down")
        );
    }

    #[test]
    fn the_filesystem_keyset_is_five_bare_keys_and_nothing_adjacent() {
        // `fs_action_for` bound exactly a/d/r/y/R and rejected every modifier.
        // Two of the near misses are destructive and neither is hypothetical:
        // a shifted `D` reaching `godotvim.fs.delete` would delete a file on a
        // keystroke nobody bound, and `Ctrl+d` — half-page-down everywhere
        // else in this plugin — must not reach it either.
        let fs = path("dock.filesystem", TREE | Caps::FILEOPS);
        for c in ['A', 'D', 'Y', 'n', 'z', 'q'] {
            assert_eq!(
                stop_of(&run_on(&fs, &[ch(c)], NEVER)),
                Some(Stop::Exhausted),
                "{c} must be unbound in the FileSystem dock"
            );
        }
        for m in [Modifiers::ALT, Modifiers::META, Modifiers::CTRL] {
            assert_eq!(
                stop_of(&run_on(&fs, &[KeyEvent::new(VimKey::Char('d'), m)], NEVER)),
                Some(Stop::Exhausted),
                "{m:?}+d must not delete"
            );
        }
        // …and a non-Latin layout still reaches the keyset by a later probe.
        assert_eq!(
            action_of(&run_on(&fs, &[ch('ф'), ch('a')], NEVER)),
            Some("godotvim.fs.create")
        );
    }

    #[test]
    fn every_shipped_default_resolves_to_a_registered_verb() {
        // Anti-drift: a default that loads but names nothing is a key that
        // consumes and does nothing.
        let index = builtin_index(&registry());
        let reg = registry();
        assert!(!index.is_empty());
        for rule in index.rules() {
            let RuleTarget::Action(id) = rule.target else {
                continue;
            };
            assert!(
                reg.get(id).is_some(),
                "{:?} on {} names no spec",
                rule.lhs,
                rule.surface
            );
        }
    }

    #[test]
    fn the_forest_audit_is_clean() {
        let errors = Forest::audit(&crate::actions::providers::forest());
        assert!(errors.is_empty(), "{errors:?}");
    }

    // ── Chain → classify → resolve, the way the dispatcher does it ───
    //
    // Everything above builds a `SurfacePath` by hand. These start from a
    // literal `FocusChain` and go through the real classifier, which is what
    // catches a surface whose probe and whose rules disagree.

    mod end_to_end {
        use super::*;
        use crate::actions::surface::fixtures::{code_edit, id, item_list, plain, tree};
        use crate::actions::surface::FocusChain;
        use godot::global::Key as GodotKey;
        use vim_core::primitives::{Mode, Operator, VisualType};

        const ATTACHED: i64 = 7;

        fn editor(mode: Option<Mode>) -> FocusChain {
            FocusChain {
                nodes: vec![code_edit(ATTACHED), plain("CodeTextEditor", 8)],
                attached_editor: Some(id(ATTACHED)),
                editor_mode: mode,
                ..Default::default()
            }
        }

        fn resolve_chain(
            chain: &FocusChain,
            keys: &[KeyEvent],
            claims: &dyn Fn(KeyEvent) -> bool,
        ) -> Resolution {
            let path = crate::actions::providers::forest()
                .classify(chain)
                .expect("the shipped forest is total");
            resolve_with(&path, &Probes::from_slice(keys), claims)
        }

        /// Resolve the probe list the REAL pipeline builds for one physical
        /// keystroke, rather than a hand-written one.
        ///
        /// The difference is the whole point of the cases below:
        /// `Probes::from_slice` marks nothing positional, so a bug that only
        /// bites once probe 3 exists is invisible to every other test in this
        /// file. `keycode` is the layout-dependent code Godot reports,
        /// `physical` the US-QWERTY scan position, `unicode` what the key
        /// actually typed.
        fn resolve_layout(
            chain: &FocusChain,
            keycode: GodotKey,
            physical: GodotKey,
            unicode: char,
        ) -> Resolution {
            let path = crate::actions::providers::forest()
                .classify(chain)
                .expect("the shipped forest is total");
            let probes = crate::actions::keys::probes_from_parts(
                keycode,
                physical,
                unicode as u32,
                false,
                false,
                false,
                false,
                None,
            );
            resolve_with(&path, &probes, NEVER)
        }

        fn fs_dock() -> FocusChain {
            FocusChain {
                nodes: vec![
                    tree("FileSystemTree", 1),
                    plain("SplitContainer", 2),
                    plain("FileSystemDock", 3),
                ],
                in_filesystem_dock: true,
                ..Default::default()
            }
        }

        #[test]
        fn colemak_j_navigates_instead_of_writing_the_clipboard() {
            // Colemak puts `j` at the QWERTY-`y` position, so the probe list
            // is ['j', 'y'] with 'y' positional. Walked surface-major,
            // `dock.filesystem` got BOTH probes before `dock` got either, so
            // probe 3 matched `godotvim.fs.yank_path` on the deeper surface —
            // and the most-used key in the dock keyset silently wrote the
            // clipboard instead of moving down, elastically consumed with no
            // fallback. Probe-major is what makes the typed key win.
            assert_eq!(
                action_of(&resolve_layout(&fs_dock(), GodotKey::J, GodotKey::Y, 'j')),
                Some("godotvim.item.next")
            );
        }

        #[test]
        fn the_qwertz_positional_alias_still_reaches_the_deeper_keyset() {
            // The twin that must NOT change. QWERTZ swaps `z` and `y`, so the
            // key at the QWERTY-`y` position types `z`; nothing binds `z`, so
            // probe 3 is the only interpretation left and
            // `panelmap <physical> dock.filesystem y` is exactly what it is
            // for. Depth still decides — pass 2 walks leaf→root too.
            assert_eq!(
                action_of(&resolve_layout(&fs_dock(), GodotKey::Z, GodotKey::Y, 'z')),
                Some("godotvim.fs.yank_path")
            );
        }

        #[test]
        fn a_capability_gated_typed_hit_never_promotes_the_positional_guess() {
            // Dvorak puts `h` at the QWERTY-`j` position. `dock h` is
            // `godotvim.item.collapse`, which needs HIERARCHY; an `ItemList`
            // does not grant it, so probe 1 is a capability MISS — and a miss
            // is a declination, not an invitation to guess. Promoting probe 3
            // there moves the selection DOWN in the open-scripts list, the
            // docs panel and the Output log, on every Dvorak keyboard, with
            // no config and no way for the user to see why.
            let list = FocusChain {
                nodes: vec![item_list("ItemList", 1), plain("VBoxContainer", 2)],
                ..Default::default()
            };
            assert_eq!(
                stop_of(&resolve_layout(&list, GodotKey::H, GodotKey::J, 'h')),
                Some(Stop::Exhausted),
                "a gated-out typed hit must leave the key inert"
            );
            // …and the twin that proves the capability gate is the cause: a
            // `Tree` grants HIERARCHY, probe 1 hits, nothing else is asked.
            let a_tree = FocusChain {
                nodes: vec![tree("Tree", 1), plain("VBoxContainer", 2)],
                ..Default::default()
            };
            assert_eq!(
                action_of(&resolve_layout(&a_tree, GodotKey::H, GodotKey::J, 'h')),
                Some("godotvim.item.collapse")
            );
        }

        /// CapsLock over the FileSystem `r`/`R` pair, pinned rather than
        /// "fixed".
        ///
        /// With CapsLock on, the `r` key types `R`, so probe 1 is `R` and
        /// resolves to `godotvim.fs.refresh`; `godotvim.fs.rename` is only
        /// reachable as the positional guess. That inversion is **intended**
        /// and is not what probe-major changed: probe 1 already won here
        /// before the restructure, because `R` and `r` are both bound on
        /// `dock.filesystem` and the as-typed probe is tried first on that
        /// surface either way.
        ///
        /// It is pinned because the alternative — letting probe 3 outrank a
        /// live probe-1 match so a CapsLocked `r` renames — inverts the one
        /// invariant the whole probe list exists to state: a guess about
        /// physical position must never beat the character the user actually
        /// typed. A user who wants the other behaviour writes two lines of
        /// vimrc; a user who gets it silently cannot.
        #[test]
        fn capslock_r_refreshes_and_that_is_the_documented_answer() {
            assert_eq!(
                action_of(&resolve_layout(&fs_dock(), GodotKey::R, GodotKey::R, 'R')),
                Some("godotvim.fs.refresh")
            );
            // Unshifted, CapsLock off: the ordinary case is untouched.
            assert_eq!(
                action_of(&resolve_layout(&fs_dock(), GodotKey::R, GodotKey::R, 'r')),
                Some("godotvim.fs.rename")
            );
        }

        #[test]
        fn insert_like_modes_never_intercept_the_panel_chords() {
            // Ctrl+H is backspace, Ctrl+J is newline, Ctrl+K is a digraph.
            // Select is in this list deliberately: it is insert-like, and
            // treating it as a nav mode would make Ctrl+H navigate panels
            // while the user is replacing a selection.
            for mode in [
                Mode::Insert,
                Mode::Replace,
                Mode::VirtualReplace,
                Mode::CommandLine,
                Mode::Select(VisualType::Char),
                Mode::Select(VisualType::Line),
                Mode::Select(VisualType::Block),
            ] {
                for key in [ctrl('h'), ctrl('j'), ctrl('k'), ctrl('l')] {
                    assert_eq!(
                        stop_of(&resolve_chain(&editor(Some(mode)), &[key], NEVER)),
                        Some(Stop::Barrier),
                        "{mode:?} must not intercept {key}"
                    );
                }
            }
        }

        #[test]
        fn nav_modes_do_intercept_the_panel_chords() {
            for mode in [
                Mode::Normal,
                Mode::Visual(VisualType::Char),
                Mode::OperatorPending(Operator::Delete),
            ] {
                assert_eq!(
                    action_of(&resolve_chain(&editor(Some(mode)), &[ctrl('h')], NEVER)),
                    Some("godotvim.focus.left"),
                    "{mode:?} must navigate"
                );
            }
        }

        #[test]
        fn no_controller_at_all_still_intercepts() {
            // `editor_mode == None` maps to `editor.nav`, not to the barrier
            // — the `is_none_or` polarity, seen from the surface side.
            assert_eq!(
                action_of(&resolve_chain(&editor(None), &[ctrl('h')], NEVER)),
                Some("godotvim.focus.left")
            );
        }

        #[test]
        fn a_foreign_text_input_is_never_touched() {
            // A Project Settings LineEdit: no sibling nav control, so it is
            // `foreign`, so it is a Barrier. Consuming Ctrl+H mid-word is the
            // regression this exists to prevent.
            let foreign = FocusChain {
                nodes: vec![
                    crate::actions::surface::fixtures::line_edit(1),
                    plain("VBoxContainer", 2),
                ],
                ..Default::default()
            };
            for key in [ctrl('h'), ch('j'), named(VimKey::Escape)] {
                assert_eq!(
                    stop_of(&resolve_chain(&foreign, &[key], NEVER)),
                    Some(Stop::Barrier),
                    "{key} must reach the LineEdit"
                );
            }
        }

        #[test]
        fn a_non_attached_code_edit_is_foreign() {
            let theirs = FocusChain {
                nodes: vec![code_edit(99)],
                attached_editor: Some(id(ATTACHED)),
                editor_mode: Some(Mode::Normal),
                ..Default::default()
            };
            assert_eq!(
                stop_of(&resolve_chain(&theirs, &[ctrl('h')], NEVER)),
                Some(Stop::Barrier)
            );
        }

        #[test]
        fn no_focus_owner_still_reaches_the_panel_chords() {
            let nothing = FocusChain::default();
            let Resolution::Run { candidates, .. } = resolve_chain(&nothing, &[ctrl('j')], NEVER)
            else {
                panic!("Ctrl+j must resolve with no focus owner at all");
            };
            assert_eq!(candidates[0].surface, "panel");
            assert_eq!(candidates[0].consume, Consumption::Void);
            // …and it consumes even though there is no target to move to.
            assert_eq!(
                dispose(&candidates, false, |_| Outcome::Declined),
                Disposition::Consume
            );
        }

        #[test]
        fn the_filesystem_dock_gets_first_refusal_by_depth_alone() {
            let fs = FocusChain {
                nodes: vec![
                    tree("FileSystemTree", 1),
                    plain("SplitContainer", 2),
                    plain("FileSystemDock", 3),
                ],
                in_filesystem_dock: true,
                ..Default::default()
            };
            for (key, want) in [
                ('a', "godotvim.fs.create"),
                ('d', "godotvim.fs.delete"),
                ('r', "godotvim.fs.rename"),
                ('y', "godotvim.fs.yank_path"),
                ('R', "godotvim.fs.refresh"),
            ] {
                assert_eq!(
                    action_of(&resolve_chain(&fs, &[ch(key)], NEVER)),
                    Some(want),
                    "{key} must resolve on dock.filesystem"
                );
            }
            // …while the generic dock keyset still falls through to `dock`.
            assert_eq!(
                action_of(&resolve_chain(&fs, &[ch('j')], NEVER)),
                Some("godotvim.item.next")
            );
        }

        #[test]
        fn the_same_widget_outside_the_filesystem_dock_has_no_file_keyset() {
            // The Scene tree. Without the `dock.filesystem` grant, `a` must
            // not create files — `get_selected_path` returns None there and
            // `begin_create` would fall back to `res://` root.
            let scene = FocusChain {
                nodes: vec![tree("SceneTreeEditor", 1), plain("SceneTreeDock", 2)],
                ..Default::default()
            };
            for key in ['a', 'd', 'r', 'y', 'R'] {
                assert_eq!(
                    stop_of(&resolve_chain(&scene, &[ch(key)], NEVER)),
                    Some(Stop::Exhausted),
                    "{key} must be unbound outside the FileSystem dock"
                );
            }
        }

        #[test]
        fn hierarchy_keys_go_inert_on_a_list_and_live_on_a_tree() {
            let list = FocusChain {
                nodes: vec![item_list("ItemList", 1), plain("VBoxContainer", 2)],
                ..Default::default()
            };
            let a_tree = FocusChain {
                nodes: vec![tree("Tree", 1), plain("VBoxContainer", 2)],
                ..Default::default()
            };
            for key in ['h', 'l'] {
                assert_eq!(
                    stop_of(&resolve_chain(&list, &[ch(key)], NEVER)),
                    Some(Stop::Exhausted),
                    "{key} must be inert on an ItemList"
                );
                assert!(
                    action_of(&resolve_chain(&a_tree, &[ch(key)], NEVER)).is_some(),
                    "{key} must work on a Tree"
                );
            }
            // j/k work on both, which is the point of naming the bit VNAV.
            for chain in [&list, &a_tree] {
                assert_eq!(
                    action_of(&resolve_chain(chain, &[ch('j')], NEVER)),
                    Some("godotvim.item.next")
                );
            }
        }

        /// Every `FocusChain` field a probe reads, and what invalidates it in
        /// `plugin::input::ChainKey`.
        ///
        /// Sampling walks the scene tree and asks six `is_class` questions
        /// per node, so it is cached — which makes "did we key the cache on
        /// everything a probe reads?" a correctness question with teeth. Each
        /// case below varies exactly one field and asserts the classification
        /// moves, which is what makes that field probe-relevant; the comment
        /// on each names the cache-key component that covers it.
        ///
        /// A field added here with no cache-key component is a stale
        /// classification: the sharpest instance is `editor_mode`, which
        /// changes WITHOUT the focus owner changing, so keying on focus alone
        /// would leave `Normal` behind after the user typed `i` and
        /// `editor.insert` would stop being a barrier while Ctrl+H is
        /// backspace.
        #[test]
        fn every_probe_relevant_fact_moves_the_classification() {
            let forest = crate::actions::providers::forest();
            let ids = |chain: &FocusChain| forest.classify(chain).expect("total").ids;

            // `nodes` — keyed by the focus owner's InstanceId.
            let empty = FocusChain::default();
            let with_tree = FocusChain {
                nodes: vec![tree("Tree", 1)],
                ..Default::default()
            };
            assert_ne!(ids(&empty), ids(&with_tree));

            // `attached_editor` — keyed directly.
            let code = FocusChain {
                nodes: vec![code_edit(ATTACHED)],
                editor_mode: Some(Mode::Normal),
                ..Default::default()
            };
            let attached = FocusChain {
                attached_editor: Some(id(ATTACHED)),
                ..code.clone()
            };
            assert_ne!(ids(&code), ids(&attached));

            // `editor_mode` — keyed directly, and the one that changes with
            // no focus change at all.
            let inserting = FocusChain {
                editor_mode: Some(Mode::Insert),
                ..attached.clone()
            };
            assert_ne!(ids(&attached), ids(&inserting));

            // `in_filesystem_dock` — a pure function of the focus owner, so
            // the focus InstanceId covers it.
            let fs = FocusChain {
                in_filesystem_dock: true,
                ..with_tree.clone()
            };
            assert_ne!(ids(&with_tree), ids(&fs));

            // `sibling_nav_control` — likewise a function of the focus owner.
            let bare_line = FocusChain {
                nodes: vec![crate::actions::surface::fixtures::line_edit(1)],
                ..Default::default()
            };
            let filter_box = FocusChain {
                sibling_nav_control: Some(id(9)),
                ..bare_line.clone()
            };
            assert_ne!(ids(&bare_line), ids(&filter_box));

            // `is_plugin_prompt` — keyed by the prompt LineEdit's InstanceId,
            // which is what `FocusChain::sample` compares the focus owner to.
            let prompt = FocusChain {
                is_plugin_prompt: true,
                ..bare_line.clone()
            };
            assert_ne!(ids(&bare_line), ids(&prompt));
        }

        #[test]
        fn a_focused_button_in_a_dock_still_gets_the_panel_chords() {
            // …and nothing else. Widening the `dock` probe to reach it would
            // wake the dead `find_best_nav_target` recursion and move a Tree
            // the user is not focused on.
            let button = FocusChain {
                nodes: vec![plain("Button", 1), plain("HBoxContainer", 2)],
                ..Default::default()
            };
            assert_eq!(
                action_of(&resolve_chain(&button, &[ctrl('l')], NEVER)),
                Some("godotvim.focus.right")
            );
            assert_eq!(
                stop_of(&resolve_chain(&button, &[ch('j')], NEVER)),
                Some(Stop::Exhausted)
            );
        }
    }
}
