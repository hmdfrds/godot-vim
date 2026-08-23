//! The binding plane: which key, on which surface, means which verb.
//!
//! This is the fifth and last of the decisions the old dispatcher fused into
//! single match arms, and the only one that is the user's business. Everything
//! else — where the keystroke is ([`super::surface`]), what a widget can do
//! ([`super::caps`]), what a verb is called ([`super::action`]) — exists so
//! that this table can be *data*.
//!
//! # One trie per surface, and metadata outside it
//!
//! Each surface owns a [`MappingTrie`], which does exactly two jobs: prefix
//! lookup and `<nowait>`. Its payload is an opaque [`SlotId`] into a side
//! arena of [`Rule`]s, and **no rule metadata lives in the trie**. Two
//! verified reasons, neither cosmetic:
//!
//! - `MappingEntry` carries exactly one owner and `insert` overwrites at the
//!   same LHS, so a shared LHS across providers would let `remove_by_owner`
//!   delete a builtin when a third party unregisters. Teardown is therefore a
//!   full rebuild, and ownership is recorded in the arena where a rebuild can
//!   read it.
//! - `Key::Action(7).to_vim_notation()` renders as literally `<Action>(7)`, so
//!   `MappingTrie::entries()` can never be the listing source of truth for
//!   `:panelmap`.
//!
//! # Exactly one rule per `(surface, lhs)`
//!
//! Forced, not chosen: `MappingTrie::insert` writes `node.entry = Some(entry)`
//! and `TrieLookup::ExactOnly` yields one `&MappingEntry`. Candidate
//! *plurality* comes from the forest **walk** — one candidate per surface on
//! the active path — never from a single LHS. Conflating the two sources is
//! what would destroy `panelunmap` and last-writer-wins.
//!
//! See `DESIGN-rebindable-nav.md` (design notes, kept outside this repo) §4.7, §5.6 and §6.3.

use compact_str::CompactString;
use vim_core::keymap::{
    Key, KeyEvent, MappingEntry, MappingKind, MappingOwner, MappingTrie, Modifiers, TrieLookup,
};

use super::action::{ActionRegistry, Params, RuleTarget};
use super::keys::{starts_vim_grammar_sequence, CMD_MODS};
use super::surface::{Forest, Seal, SurfaceId};
use crate::config::panelmap::{parse_panel_line, PanelLine, PanelMap, PanelParseError, TargetSpec};

/// When a matched rule consumes the keystroke.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Consumption {
    /// Consume iff the action accepted.
    ///
    /// Preserves `j` at the end of a list and Enter with nothing selected:
    /// both decline, the key falls through, and Godot's own handling proceeds.
    Elastic,
    /// Consume regardless of outcome, and terminate the walk.
    ///
    /// The declarative form of `src/plugin/input.rs`, where
    /// `handle_window_nav`'s result is discarded and `set_input_as_handled()`
    /// fires even with no focus owner and no target found.
    Void,
    /// NEVER consume, and skip the plugin's own remaining stages. When the
    /// action declines, behaves exactly as `Elastic`.
    ///
    /// NOT spellable in the `panelmap` grammar, and that is a safety property
    /// rather than a convenience. `config/sandbox.rs` admits any
    /// non-`Shortcut` `Map` from a committed project vimrc at every trust
    /// tier, so a spellable handoff flag could be attached to a verb with
    /// empty `requires` and suppress the vim engine on Escape. Carried by the
    /// verb through [`super::action::ActionSpec::default_consume`], audit A9'
    /// can see it.
    Handoff,
}

/// Whether a rule fires on `InputEventKey::is_echo()` repeats.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Repeat {
    Allow,
    /// Per-rule, never global: held `j`/`k` auto-repeat in a dock is
    /// desirable, while a ~20/s storm of deferred `grab_focus` from a held
    /// Ctrl+J is not.
    Suppress,
}

/// An opaque handle into the rule arena, and the trie's entire payload.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SlotId(pub(crate) u32);

/// One binding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Rule {
    pub(crate) surface: SurfaceId,
    /// Canonicalized, non-empty, at most `MAX_KEY_SEQUENCE_LEN` long.
    pub(crate) lhs: Vec<KeyEvent>,
    pub(crate) target: RuleTarget,
    pub(crate) params: Params,
    pub(crate) consume: Consumption,
    pub(crate) repeat: Repeat,
    /// Opt-in US-QWERTY positional probe.
    ///
    /// This flag is what finally *scopes* the positional guess. Probes 1 and 2
    /// are offered on every surface; probe 3 is offered only where a rule asks
    /// for it, so a Dvorak `Ctrl+d` inside the editor stays half-page-down
    /// instead of becoming panel-left. See [`super::keys::Probes::iter`] vs
    /// [`super::keys::Probes::iter_typed`].
    pub(crate) physical: bool,
    /// Also match this LHS with SHIFT set.
    ///
    /// Expanded at **registration** into a second trie LHS pointing at the
    /// same [`SlotId`] — dispatch gains no stage for it. Reproduces
    /// `handle_search_input`'s guard, which rejects ctrl/alt/meta but tolerates
    /// shift, without giving every rule that tolerance.
    pub(crate) shift_tolerant: bool,
    /// `<nowait>`: build the entry with `MappingEntry::new_nowait` so
    /// `MappingTrie::lookup` promotes `Prefix` to `ExactOnly` internally.
    pub(crate) nowait: bool,
    /// Which provider (or the user) installed this rule.
    pub(crate) owner: MappingOwner,
    pub(crate) desc: CompactString,
}

/// Why a rule cannot be registered.
///
/// Distinct from [`PanelParseError`]: those are answerable by reading one
/// line, these need the forest and the action registry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum RuleReject {
    /// The line did not parse at all.
    Parse(PanelParseError),
    UnknownSurface(CompactString),
    /// A typo in a target must never become a silent dead key.
    UnknownAction(CompactString),
    /// `foreign` and `editor.insert` take no rules: dispatch returns `Ignore`
    /// before any lookup there, so a rule would be unreachable by
    /// construction — and accepting one would let a project vimrc claim keys
    /// inside a Project Settings `LineEdit` or in Insert mode.
    BarrierSurface(SurfaceId),
    /// The surface is an `editor.*` surface or an ancestor of one.
    MultiKeyOnEditorPath(SurfaceId),
    /// The first key starts a vim-core grammar sequence.
    VimGrammarPrefix(KeyEvent),
    /// The target parses and registers, but nothing on this surface's
    /// transport can dispatch it — so the key would be permanently dead while
    /// `:panelmap` reported it as eligible.
    UndispatchedTarget {
        surface: SurfaceId,
        target: CompactString,
    },
}

impl std::fmt::Display for RuleReject {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Parse(e) => write!(f, "{e}"),
            Self::UnknownSurface(s) => write!(f, "no surface named '{s}' is declared"),
            Self::UnknownAction(a) => write!(f, "no action named '{a}' is registered"),
            Self::BarrierSurface(s) => write!(
                f,
                "surface '{s}' is a barrier and takes no bindings; \
                 keys there belong to the control that has focus"
            ),
            Self::MultiKeyOnEditorPath(s) => write!(
                f,
                "surface '{s}' is reachable from the script editor, \
                 so its bindings must be a single key"
            ),
            Self::VimGrammarPrefix(k) => write!(
                f,
                "'{}' begins a Vim command sequence; binding it here would \
                 destroy the key that follows it",
                k.to_vim_notation()
            ),
            Self::UndispatchedTarget { surface, target } => write!(
                f,
                "surface '{surface}' has no transport that can dispatch '{target}', \
                 so the key would be consumed and do nothing"
            ),
        }
    }
}

/// Who wrote a rule — which is what splits the diagnostic policy.
///
/// Distinct from [`MappingOwner`], which records *which provider*: two builtin
/// providers have different owners and the same provenance.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Provenance {
    /// Shipped provider defaults. A failure is a programming error, so it is a
    /// `debug_assert!` plus `log::error!` — never warn-and-skip. A shipped
    /// default that silently does not load is a keyset regression with a green
    /// test suite.
    Builtin,
    /// The resolved vimrc, or a `:panelmap` typed at the command line. Failure
    /// is warn-and-skip per line, accumulating for `:checkhealth`.
    User,
}

/// One rejected line, ready to print.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PanelDiagnostic {
    /// 1-based, when the rule came from a file.
    pub(crate) line: Option<u32>,
    /// The provider tag, or the vimrc path.
    pub(crate) source: CompactString,
    pub(crate) reject: RuleReject,
}

impl std::fmt::Display for PanelDiagnostic {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.line {
            Some(n) => write!(f, "{}:{n}: {}", self.source, self.reject),
            None => write!(f, "{}: {}", self.source, self.reject),
        }
    }
}

/// One surface's trie, plus the registration-time slot table.
#[derive(Debug, Default)]
struct SurfaceBindings {
    surface: SurfaceId,
    trie: MappingTrie,
    /// `(canonical LHS, SlotId)` for every LIVE rule on this surface.
    ///
    /// Registration-time only, and it is what makes a re-insert **reuse** the
    /// slot rather than orphan the previous rule in the arena where
    /// `:panelmap` would still list it. Prefix reservations are derived from
    /// it rather than stored beside it, so the two cannot drift.
    slot_of: Vec<(Vec<KeyEvent>, SlotId)>,
}

/// Every binding, indexed by surface.
///
/// Iteration order is registration order. That is deliberate and load-bearing:
/// the introspector's golden snapshots depend on it, and both
/// `std::collections::HashMap` and `ahash::AHashMap` are randomly seeded per
/// process. At ~16 surfaces a linear scan over `&'static str` beats hashing
/// anyway, and costs no dependency.
#[derive(Debug, Default)]
pub(crate) struct BindingIndex {
    surfaces: Vec<SurfaceBindings>,
    /// Append-only. A superseded rule stays here; `slots` is what says which
    /// entry is live.
    arena: Vec<Rule>,
    /// `SlotId` → arena index, or `None` once the rule was unmapped.
    slots: Vec<Option<u32>>,
    forest: Forest,
    pub(crate) generation: u64,
}

impl BindingIndex {
    pub(crate) fn new(forest: Forest) -> Self {
        Self {
            forest,
            ..Self::default()
        }
    }

    pub(crate) fn forest(&self) -> &Forest {
        &self.forest
    }

    /// Whether a rule on `surface` can fire while the script editor has focus.
    ///
    /// True for an `editor.*` surface **or an ancestor of one in the declared
    /// forest**. `panel` is `editor.nav`'s parent, so `panelmap panel <C-w>h`
    /// is live in the editor — which is exactly why "reject on `editor.*`" is
    /// not sufficient and this asks the forest instead.
    pub(crate) fn editor_reachable(&self, surface: SurfaceId) -> bool {
        self.forest
            .ids()
            .filter(|id| id.starts_with("editor."))
            .any(|editor| self.forest.is_ancestor_or_self(surface, editor))
    }

    /// Validate and install a rule. The only entry point user input reaches.
    ///
    /// It no longer takes the registry: the old capability question it asked
    /// there ("can this verb's `requires` ever be satisfied on this
    /// surface?") is the pipeline gate's job now, on every surface alike, so
    /// registration validates structure only.
    pub(crate) fn try_insert(&mut self, rule: Rule) -> Result<(), RuleReject> {
        let Some(spec) = self.forest.get(rule.surface) else {
            return Err(RuleReject::UnknownSurface(rule.surface.into()));
        };
        if spec.seal == Seal::Barrier {
            return Err(RuleReject::BarrierSurface(rule.surface));
        }
        // V-DISPATCH: refuse at registration what no transport can honour.
        // `<Shortcut>(path)` is parsed, registered and printed as eligible,
        // and then `run_candidate` unconditionally declines it after a
        // `log::warn!` nobody sees — the default Log Level is Off. With
        // `<void>` that is a permanently dead key the introspector actively
        // confirms will work. Delegating to Godot's own shortcuts needs a
        // cycle audit and an injection budget it does not have yet; until
        // then the honest answer is at registration, not at dispatch.
        //
        // A capability-bearing verb on the overlay surface is NOT refused
        // here any more: the overlay resolves through the same pipeline as
        // every classified surface, so `hit_from`'s capability gate is real
        // there and such a rule loads, then goes inert with an explain line.
        if let RuleTarget::Shortcut(path) = &rule.target {
            return Err(RuleReject::UndispatchedTarget {
                surface: rule.surface,
                target: format!("<Shortcut>({path})").into(),
            });
        }
        if self.editor_reachable(rule.surface) {
            if rule.lhs.len() > 1 {
                return Err(RuleReject::MultiKeyOnEditorPath(rule.surface));
            }
            // `lhs` is non-empty by construction — `parse_lhs` rejects the
            // empty sequence — but reading it fallibly keeps a future caller
            // from turning a programming error into a panic in `_input()`.
            let Some(&first) = rule.lhs.first() else {
                return Err(RuleReject::Parse(PanelParseError::MissingOperand(
                    "key sequence",
                )));
            };
            if starts_vim_grammar_sequence(first) {
                return Err(RuleReject::VimGrammarPrefix(first));
            }
        }
        self.upsert(rule);
        Ok(())
    }

    /// Install a rule, last writer wins at one `(surface, lhs)`.
    ///
    /// Validation-free on purpose: [`Self::try_insert`] is the gate, and this
    /// is the primitive it and the tests share.
    pub(crate) fn upsert(&mut self, rule: Rule) {
        let slot = self.alloc_or_reuse_slot(rule.surface, &rule.lhs);
        let arena_index = self.arena.len() as u32;
        if let Some(cell) = self.slots.get_mut(slot.0 as usize) {
            *cell = Some(arena_index);
        }
        self.insert_at(rule.surface, &rule.lhs.clone(), slot, &rule);
        if rule.shift_tolerant {
            if let Some(shifted) = shift_variant(&rule.lhs) {
                // ONE rule, TWO left-hand sides, one slot. This is the entire
                // implementation of shift tolerance — dispatch never learns
                // that the second spelling exists.
                self.insert_at(rule.surface, &shifted, slot, &rule);
            }
        }
        self.arena.push(rule);
    }

    /// Remove the rule at `(surface, lhs)`, if there is one.
    ///
    /// This is `panelunmap`, and it is **not** `native`: the forest walk
    /// continues to the parent surface afterwards. Handing the key back to
    /// Godot is a different verb with a different target.
    pub(crate) fn remove(&mut self, surface: SurfaceId, lhs: &[KeyEvent]) -> bool {
        let Some(index) = self.surfaces.iter().position(|s| s.surface == surface) else {
            return false;
        };
        let Some(pos) = self.surfaces[index]
            .slot_of
            .iter()
            .position(|(k, _)| k.as_slice() == lhs)
        else {
            return false;
        };
        let (_, slot) = self.surfaces[index].slot_of.remove(pos);
        let shifted = self
            .slots
            .get(slot.0 as usize)
            .copied()
            .flatten()
            .and_then(|i| self.arena.get(i as usize))
            .filter(|rule| rule.shift_tolerant)
            .and_then(|_| shift_variant(lhs));
        if let Some(cell) = self.slots.get_mut(slot.0 as usize) {
            *cell = None;
        }
        let bindings = &mut self.surfaces[index];
        bindings.trie.remove(lhs);
        if let Some(shifted) = shifted {
            // The twin was never in `slot_of`, so it would otherwise survive
            // its own rule and fire on Shift+<key> alone.
            bindings.trie.remove(&shifted);
        }
        true
    }

    fn insert_at(&mut self, surface: SurfaceId, lhs: &[KeyEvent], slot: SlotId, rule: &Rule) {
        // The RHS is one opaque slot key and nothing else. Everything the
        // resolver needs is reached through `rule_at`.
        let rhs = vec![KeyEvent::action(slot.0)];
        let entry = if rule.nowait {
            MappingEntry::new_nowait(rhs, MappingKind::NonRecursive)
        } else {
            MappingEntry::new(rhs, MappingKind::NonRecursive)
        }
        .with_owner(rule.owner.clone())
        .with_description(Some(rule.desc.clone()));
        self.bindings_mut(surface).trie.insert(lhs, entry);
    }

    fn bindings_mut(&mut self, surface: SurfaceId) -> &mut SurfaceBindings {
        if let Some(pos) = self.surfaces.iter().position(|s| s.surface == surface) {
            return &mut self.surfaces[pos];
        }
        self.surfaces.push(SurfaceBindings {
            surface,
            ..SurfaceBindings::default()
        });
        self.surfaces.last_mut().unwrap_or_else(|| unreachable!())
    }

    fn bindings(&self, surface: SurfaceId) -> Option<&SurfaceBindings> {
        self.surfaces.iter().find(|s| s.surface == surface)
    }

    fn alloc_or_reuse_slot(&mut self, surface: SurfaceId, lhs: &[KeyEvent]) -> SlotId {
        if let Some((_, slot)) = self
            .bindings_mut(surface)
            .slot_of
            .iter()
            .find(|(k, _)| k.as_slice() == lhs)
        {
            return *slot;
        }
        let slot = SlotId(self.slots.len() as u32);
        self.slots.push(None);
        self.bindings_mut(surface)
            .slot_of
            .push((lhs.to_vec(), slot));
        slot
    }

    /// Three-way prefix lookup on one surface.
    pub(crate) fn lookup(&self, surface: SurfaceId, prefix: &[KeyEvent]) -> TrieLookup<'_> {
        self.bindings(surface)
            .map_or(TrieLookup::NoMatch, |b| b.trie.lookup(prefix))
    }

    /// The slot a trie entry carries, if it is one of ours.
    pub(crate) fn slot_in(entry: &MappingEntry) -> Option<SlotId> {
        match entry.sequence() {
            [key] => match key.key() {
                Key::Action(id) => Some(SlotId(id)),
                _ => None,
            },
            _ => None,
        }
    }

    /// The live rule at one exact `(surface, lhs)`, if there is one.
    ///
    /// Goes through the trie rather than scanning `slot_of`, so `<nowait>`
    /// and prefix semantics are the trie's answer here exactly as they are in
    /// the resolver's walk — the introspector must not be able to report a
    /// rule the walk would not reach.
    pub(crate) fn rule_for(&self, surface: SurfaceId, lhs: &[KeyEvent]) -> Option<&Rule> {
        match self.lookup(surface, lhs) {
            TrieLookup::ExactOnly(entry) => Self::slot_in(entry).and_then(|s| self.rule_at(s)),
            _ => None,
        }
    }

    /// The live rule a slot names, if it has not been unmapped.
    pub(crate) fn rule_at(&self, slot: SlotId) -> Option<&Rule> {
        let index = (*self.slots.get(slot.0 as usize)?)?;
        self.arena.get(index as usize)
    }

    /// Whether `key` is the bare first key of some multi-key rule on `surface`.
    ///
    /// Derived from the live slot table rather than stored beside it, so a
    /// `panelunmap` cannot leave a reservation behind. Reservation is opt-in
    /// and user-visible — never speculative.
    pub(crate) fn is_reserved(&self, surface: SurfaceId, key: KeyEvent) -> bool {
        self.bindings(surface).is_some_and(|b| {
            b.slot_of
                .iter()
                .any(|(lhs, _)| lhs.len() > 1 && lhs.first() == Some(&key))
        })
    }

    /// Whether `surface` reserves any bare key at all.
    ///
    /// The predicate that scopes `Tree::set_allow_search(false)`: a control
    /// whose surface stack reserves nothing keeps Godot's incremental
    /// type-to-search untouched, which is every control in the shipped
    /// zero-config keyset.
    pub(crate) fn reserves_any(&self, surface: SurfaceId) -> bool {
        self.bindings(surface)
            .is_some_and(|b| b.slot_of.iter().any(|(lhs, _)| lhs.len() > 1))
    }

    /// Every bare key `surface` reserves, in slot-allocation order, without
    /// repeats.
    ///
    /// The introspector's source: a reservation that `:panelmap` cannot print
    /// is exactly the silent dead key this whole design exists to prevent.
    pub(crate) fn reservations(&self, surface: SurfaceId) -> Vec<KeyEvent> {
        let mut out: Vec<KeyEvent> = Vec::new();
        let Some(bindings) = self.bindings(surface) else {
            return out;
        };
        for (lhs, _) in &bindings.slot_of {
            if lhs.len() <= 1 {
                continue;
            }
            let Some(&first) = lhs.first() else { continue };
            if !out.contains(&first) {
                out.push(first);
            }
        }
        out
    }

    /// Every live multi-key rule on `surface` whose LHS starts with `first`.
    ///
    /// What `:panelmap <key>` prints under a reservation, so "this key is
    /// consumed and waits" always comes with "…for these".
    pub(crate) fn sequences_from(
        &self,
        surface: SurfaceId,
        first: KeyEvent,
    ) -> impl Iterator<Item = &Rule> + '_ {
        self.rules_on(surface)
            .filter(move |rule| rule.lhs.len() > 1 && rule.lhs.first() == Some(&first))
    }

    /// Every live rule, in slot-allocation order.
    pub(crate) fn rules(&self) -> impl Iterator<Item = &Rule> + '_ {
        self.slots
            .iter()
            .filter_map(|slot| self.arena.get((*slot)? as usize))
    }

    /// Whether any live rule on `surface` opted into the US-QWERTY positional
    /// probe.
    ///
    /// This is where `<physical>`'s scoping is *enforced*: probes 1 and 2 are
    /// offered on every surface, probe 3 only where a rule asked for it. It
    /// is what keeps a Dvorak `Ctrl+d` from becoming panel-left on a surface
    /// that binds nothing positionally, and what confines the QWERTZ `z`
    /// alias to the fourteen rules that carry the flag.
    ///
    /// Computed rather than cached: `surfaces` is a linear scan over ~8
    /// entries and `slot_of` over ~5, both once per surface per keystroke,
    /// and a cache is a second source of truth that a `panelunmap` can
    /// desynchronize.
    pub(crate) fn has_physical_rule(&self, surface: SurfaceId) -> bool {
        self.rules()
            .any(|rule| rule.surface == surface && rule.physical)
    }

    /// Every live rule on one surface, in slot-allocation order. For the
    /// introspector, which lists per surface in forest order.
    pub(crate) fn rules_on(&self, surface: SurfaceId) -> impl Iterator<Item = &Rule> + '_ {
        self.rules().filter(move |rule| rule.surface == surface)
    }

    pub(crate) fn len(&self) -> usize {
        self.rules().count()
    }

    /// Test-only, and load-bearing there: `index.is_empty()` is the whole
    /// assertion for six distinct registration-rejection tests, each of which
    /// would go vacuous without it.
    #[allow(
        dead_code,
        reason = "the assertion for six registration-rejection tests"
    )]
    pub(crate) fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Whether any rule points at `id` — the "this verb is registered but
    /// unreachable" question. No production caller: `:checkhealth`, which
    /// would have been the one, was never built.
    #[allow(
        dead_code,
        reason = "asked only by tests; the :checkhealth command it was written for was never built"
    )]
    pub(crate) fn any_rule_targets(&self, id: super::action::ActionId) -> bool {
        self.rules()
            .any(|rule| rule.target == RuleTarget::Action(id))
    }
}

/// The SHIFT-set twin of a single-key LHS, when one is meaningful.
///
/// Meaningful only for a single **named** key with no Ctrl/Alt/Meta.
/// `bridge::input::translate_key` keeps SHIFT for named keys but strips it for
/// printables, so a `Key::Char` LHS never needs a shifted twin — `R` already
/// *is* the shifted spelling of `r`, and synthesizing `Char('r')+SHIFT` would
/// add a key the runtime cannot produce.
fn shift_variant(lhs: &[KeyEvent]) -> Option<Vec<KeyEvent>> {
    let [key] = lhs else { return None };
    if matches!(key.key(), Key::Char(_)) {
        return None;
    }
    if key.modifiers().intersects(CMD_MODS) {
        return None;
    }
    if key.modifiers().contains(Modifiers::SHIFT) {
        return None;
    }
    Some(vec![KeyEvent::new(
        key.key(),
        key.modifiers() | Modifiers::SHIFT,
    )])
}

/// Turn a parsed `panelmap` line into a rule, resolving its target.
///
/// The unknown-action check lives here rather than in the parser because only
/// this layer knows what is registered — and it must exist, because a typo
/// that loaded as a rule with no verb would be a key that consumes and does
/// nothing, which is indistinguishable from a broken keyboard.
fn rule_from(
    map: &PanelMap,
    registry: &ActionRegistry,
    forest: &Forest,
    owner: &MappingOwner,
) -> Result<Rule, RuleReject> {
    let Some(surface) = forest.ids().find(|id| *id == map.surface.as_str()) else {
        return Err(RuleReject::UnknownSurface(map.surface.clone()));
    };
    let (target, desc) = match &map.target {
        TargetSpec::Action(name) => {
            let Some(id) = registry.id_of(name) else {
                return Err(RuleReject::UnknownAction(name.clone()));
            };
            let desc = registry
                .get(id)
                .map_or_else(|| name.clone(), |spec| CompactString::from(spec.desc));
            (RuleTarget::Action(id), desc)
        }
        TargetSpec::Native => (
            RuleTarget::Native,
            CompactString::from("give the key back to Godot"),
        ),
        TargetSpec::Shortcut(path) => (
            RuleTarget::Shortcut(path.clone()),
            CompactString::from(format!("editor shortcut {path}")),
        ),
    };
    let consume = if map.flags.void {
        // `<void>` on the rule always wins over the verb default, so a user
        // can raise consumption. There is no spelling that lowers it back to
        // `Elastic`; that is a stated residual.
        Consumption::Void
    } else {
        match &target {
            RuleTarget::Action(id) => registry.get(*id).and_then(|s| s.default_consume),
            _ => None,
        }
        .unwrap_or(Consumption::Elastic)
    };
    Ok(Rule {
        surface,
        lhs: map.lhs.clone(),
        target,
        params: map.params.clone(),
        consume,
        repeat: if map.flags.norepeat {
            Repeat::Suppress
        } else {
            Repeat::Allow
        },
        physical: map.flags.physical,
        shift_tolerant: map.flags.shift,
        nowait: map.flags.nowait,
        owner: owner.clone(),
        desc,
    })
}

/// Apply a block of `panelmap` text to `index`.
///
/// The single code path for both layers: provider defaults and the user's
/// vimrc are the *same sentences* read by the *same* parser, which is what
/// stops the shipped keyset from drifting into a dialect the documented
/// grammar does not describe.
///
/// Severity follows `provenance`, and that split is the point: a malformed
/// user line is a diagnostic, a malformed builtin default is a programming
/// error.
///
/// # Panics (debug only)
/// If `provenance` is [`Provenance::Builtin`] and a line does not load.
pub(crate) fn apply_text(
    index: &mut BindingIndex,
    registry: &ActionRegistry,
    text: &str,
    owner: &MappingOwner,
    source: &str,
    provenance: Provenance,
    diagnostics: &mut Vec<PanelDiagnostic>,
) {
    for (offset, line) in text.lines().enumerate() {
        let lineno = (offset + 1) as u32;
        let outcome = match parse_panel_line(line) {
            Ok(None) => continue,
            Err(error) => Err(RuleReject::Parse(error)),
            Ok(Some(PanelLine::Map(map))) => {
                rule_from(&map, registry, index.forest(), owner).and_then(|r| index.try_insert(r))
            }
            Ok(Some(PanelLine::Unmap { surface, lhs })) => {
                let declared = index.forest().ids().find(|id| *id == surface.as_str());
                match declared {
                    Some(declared) => {
                        index.remove(declared, &lhs);
                        Ok(())
                    }
                    None => Err(RuleReject::UnknownSurface(surface)),
                }
            }
        };
        let Err(reject) = outcome else { continue };
        let diagnostic = PanelDiagnostic {
            line: Some(lineno),
            source: source.into(),
            reject,
        };
        match provenance {
            Provenance::Builtin => {
                log::error!("panelmap: shipped default failed to load — {diagnostic}");
                debug_assert!(
                    false,
                    "a shipped panelmap default must always load: {diagnostic}"
                );
            }
            Provenance::User => {
                log::warn!("panelmap: {diagnostic}");
                diagnostics.push(diagnostic);
            }
        }
    }
}

/// Build the builtin layer: every provider's defaults, in `PROVIDERS` order.
///
/// Layer 0 of the two-layer precedence of §6.1. The user's vimrc is applied
/// on top by the caller, last-writer-wins, and is deliberately *not* this
/// function's business — builtin defaults must load whether or not a vimrc
/// exists, or a security setting would destroy the zero-config keyset.
pub(crate) fn builtin_index(registry: &ActionRegistry) -> BindingIndex {
    let mut index = BindingIndex::new(super::providers::forest());
    // Discarded rather than returned: under `Provenance::Builtin` nothing can
    // reach it — a failure is a `debug_assert!` before it gets here.
    let mut diagnostics = Vec::new();
    for provider in super::providers::PROVIDERS {
        apply_text(
            &mut index,
            registry,
            provider.defaults,
            &MappingOwner::Host(provider.tag.into()),
            provider.tag,
            Provenance::Builtin,
            &mut diagnostics,
        );
    }
    index
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::actions::action::{ActionId, ActionSpec};
    use crate::actions::caps::Caps;
    use crate::actions::outcome::Outcome;
    use crate::actions::specs;

    /// The whole shipped registry — `specs::SHIPPED` **plus** every
    /// `Provider::actions` table. Looping `SHIPPED` alone here would leave a
    /// provider's own verbs unregistered, and `builtin_index` would then
    /// reject that provider's defaults with `UnknownAction` — a
    /// `debug_assert!` under `Provenance::Builtin`, so the failure is loud
    /// but the cause reads as unrelated.
    fn registry() -> ActionRegistry {
        specs::registry()
    }

    fn empty_index() -> BindingIndex {
        BindingIndex::new(crate::actions::providers::forest())
    }

    fn ch(c: char) -> KeyEvent {
        KeyEvent::new(Key::Char(c), Modifiers::NONE)
    }

    fn ctrl(c: char) -> KeyEvent {
        KeyEvent::new(Key::Char(c), Modifiers::CTRL)
    }

    /// Register `line` as if it came from a user vimrc, returning the reject.
    fn user_reject(index: &mut BindingIndex, line: &str) -> Option<RuleReject> {
        let mut diagnostics = Vec::new();
        apply_text(
            index,
            &registry(),
            line,
            &MappingOwner::User,
            "test",
            Provenance::User,
            &mut diagnostics,
        );
        diagnostics.pop().map(|d| d.reject)
    }

    fn user_ok(index: &mut BindingIndex, text: &str) {
        let mut diagnostics = Vec::new();
        apply_text(
            index,
            &registry(),
            text,
            &MappingOwner::User,
            "test",
            Provenance::User,
            &mut diagnostics,
        );
        assert_eq!(diagnostics, Vec::new(), "expected '{text}' to load");
    }

    /// The rule at `(surface, lhs)`, reached the way the resolver will.
    fn resolve<'a>(
        index: &'a BindingIndex,
        surface: SurfaceId,
        lhs: &[KeyEvent],
    ) -> Option<&'a Rule> {
        match index.lookup(surface, lhs) {
            TrieLookup::ExactOnly(entry) | TrieLookup::Prefix { exact: Some(entry) } => {
                index.rule_at(BindingIndex::slot_in(entry)?)
            }
            _ => None,
        }
    }

    // ── The shipped default set ──────────────────────────────────────

    /// Every default the plugin ships, spelled out independently of the
    /// provider files. Columns: surface, LHS, action id, `<physical>`,
    /// consumption, repeat, `<shift>`.
    #[allow(clippy::type_complexity, reason = "a golden table is a table")]
    const SHIPPED_DEFAULTS: &[(SurfaceId, &str, &str, bool, Consumption, Repeat, bool)] = &[
        // Cross-panel focus. `<void>` reproduces input.rs, where
        // handle_window_nav's result is discarded and set_input_as_handled()
        // fires even when nothing was found. `<norepeat>` keeps a held Ctrl+J
        // from queueing ~20 deferred grab_focus calls a second.
        (
            "panel",
            "<C-h>",
            "godotvim.focus.left",
            true,
            Consumption::Void,
            Repeat::Suppress,
            false,
        ),
        (
            "panel",
            "<C-j>",
            "godotvim.focus.down",
            true,
            Consumption::Void,
            Repeat::Suppress,
            false,
        ),
        (
            "panel",
            "<C-k>",
            "godotvim.focus.up",
            true,
            Consumption::Void,
            Repeat::Suppress,
            false,
        ),
        (
            "panel",
            "<C-l>",
            "godotvim.focus.right",
            true,
            Consumption::Void,
            Repeat::Suppress,
            false,
        ),
        // The autocomplete popup (P9). Every one is elastic and none carries
        // `<physical>`: the verdict on the `gui_input` transport IS the
        // action's outcome, and `<CR>` consuming with no popup up would stop
        // Enter inserting a newline. `<C-@>` and not `<C-Space>` — the bridge
        // folds Ctrl+Space into `Char('@') + CTRL` before anything sees it, so
        // the other spelling would load cleanly and never fire.
        (
            "editor.completion",
            "<C-@>",
            "godotvim.completion.trigger",
            false,
            Consumption::Elastic,
            Repeat::Allow,
            false,
        ),
        (
            "editor.completion",
            "<C-n>",
            "godotvim.completion.next",
            false,
            Consumption::Elastic,
            Repeat::Allow,
            false,
        ),
        (
            "editor.completion",
            "<C-p>",
            "godotvim.completion.prev",
            false,
            Consumption::Elastic,
            Repeat::Allow,
            false,
        ),
        (
            "editor.completion",
            "<Tab>",
            "godotvim.completion.confirm",
            false,
            Consumption::Elastic,
            Repeat::Allow,
            false,
        ),
        (
            "editor.completion",
            "<CR>",
            "godotvim.completion.confirm",
            false,
            Consumption::Elastic,
            Repeat::Allow,
            false,
        ),
        (
            "editor.completion",
            "<Esc>",
            "godotvim.completion.dismiss",
            false,
            Consumption::Elastic,
            Repeat::Allow,
            false,
        ),
        // `navigate` declares `default_consume: Handoff`, folded into the
        // rule at registration: acceptance skips the engine without
        // consuming, so CodeEdit's own popup handling moves the selection.
        (
            "editor.completion",
            "<Up>",
            "godotvim.completion.navigate",
            false,
            Consumption::Handoff,
            Repeat::Allow,
            false,
        ),
        (
            "editor.completion",
            "<Down>",
            "godotvim.completion.navigate",
            false,
            Consumption::Handoff,
            Repeat::Allow,
            false,
        ),
        // Dock item navigation. Elastic: `j` at the end of a list declines and
        // the key falls through, exactly as dock.rs does today.
        (
            "dock",
            "h",
            "godotvim.item.collapse",
            true,
            Consumption::Elastic,
            Repeat::Allow,
            false,
        ),
        (
            "dock",
            "j",
            "godotvim.item.next",
            true,
            Consumption::Elastic,
            Repeat::Allow,
            false,
        ),
        (
            "dock",
            "k",
            "godotvim.item.prev",
            true,
            Consumption::Elastic,
            Repeat::Allow,
            false,
        ),
        (
            "dock",
            "l",
            "godotvim.item.expand",
            true,
            Consumption::Elastic,
            Repeat::Allow,
            false,
        ),
        (
            "dock",
            "/",
            "godotvim.dock.search",
            true,
            Consumption::Elastic,
            Repeat::Allow,
            false,
        ),
        // Enter and Escape complete the dock keyset — `dock_action_for` binds
        // seven keys, not five. Neither carries `<physical>`: a named key
        // never receives a positional probe, so the flag would be inert.
        (
            "dock",
            "<CR>",
            "godotvim.item.activate",
            false,
            Consumption::Elastic,
            Repeat::Allow,
            false,
        ),
        (
            "dock",
            "<Esc>",
            "godotvim.focus.editor",
            false,
            Consumption::Elastic,
            Repeat::Allow,
            false,
        ),
        // The filter box. Shift-tolerant, and the ONLY two rules that are:
        // handle_search_input rejects ctrl/alt/meta but not shift, while a
        // dock rejects every modifier including shift.
        (
            "searchbox",
            "<CR>",
            "godotvim.search.accept",
            false,
            Consumption::Elastic,
            Repeat::Allow,
            true,
        ),
        (
            "searchbox",
            "<Esc>",
            "godotvim.search.accept",
            false,
            Consumption::Elastic,
            Repeat::Allow,
            true,
        ),
        // nvim-tree-flavoured file operations. `R` refreshes while `r`
        // renames: Shift is a discriminant here, carried by the character
        // itself because `bridge::input` folds it in.
        (
            "dock.filesystem",
            "a",
            "godotvim.fs.create",
            true,
            Consumption::Elastic,
            Repeat::Allow,
            false,
        ),
        (
            "dock.filesystem",
            "d",
            "godotvim.fs.delete",
            true,
            Consumption::Elastic,
            Repeat::Allow,
            false,
        ),
        (
            "dock.filesystem",
            "r",
            "godotvim.fs.rename",
            true,
            Consumption::Elastic,
            Repeat::Allow,
            false,
        ),
        (
            "dock.filesystem",
            "y",
            "godotvim.fs.yank_path",
            true,
            Consumption::Elastic,
            Repeat::Allow,
            false,
        ),
        (
            "dock.filesystem",
            "R",
            "godotvim.fs.refresh",
            true,
            Consumption::Elastic,
            Repeat::Allow,
            false,
        ),
        // The debugger provider (P9). Transcribed from `providers/debugger.rs`
        // independently, which is the point of this table: a provider that
        // silently stops loading its own defaults is invisible in its own file
        // and visible here. No `<physical>` — these keys are mnemonic, not
        // positional — so the "exactly fourteen" count below still holds.
        (
            "dock.debugger",
            "J",
            "godotvim.debugger.frame_next",
            false,
            Consumption::Elastic,
            Repeat::Allow,
            false,
        ),
        (
            "dock.debugger",
            "K",
            "godotvim.debugger.frame_prev",
            false,
            Consumption::Elastic,
            Repeat::Allow,
            false,
        ),
        (
            "dock.debugger",
            "G",
            "godotvim.debugger.frame_last",
            false,
            Consumption::Elastic,
            Repeat::Allow,
            false,
        ),
        (
            "dock.debugger",
            "y",
            "godotvim.debugger.yank_frame",
            false,
            Consumption::Elastic,
            Repeat::Allow,
            false,
        ),
    ];

    #[test]
    fn the_full_default_set_loads_exactly() {
        // THE headline gate of this phase. A silently dropped default is a
        // keyset regression with a green suite, so this asserts three ways:
        // the count, every row's presence and every field on it, and that
        // nothing extra crept in.
        let registry = registry();
        let index = builtin_index(&registry);

        assert_eq!(
            index.len(),
            SHIPPED_DEFAULTS.len(),
            "shipped defaults: {:?}",
            index
                .rules()
                .map(|r| (r.surface, r.lhs.clone()))
                .collect::<Vec<_>>()
        );

        for (surface, notation, action, physical, consume, repeat, shift) in SHIPPED_DEFAULTS {
            let lhs = crate::actions::keys::parse_lhs(notation).expect(notation);
            let rule = resolve(&index, surface, &lhs)
                .unwrap_or_else(|| panic!("no rule at {surface} {notation}"));
            let id = registry.id_of(action).expect(action);
            assert_eq!(rule.target, RuleTarget::Action(id), "{surface} {notation}");
            assert_eq!(rule.physical, *physical, "{surface} {notation} <physical>");
            assert_eq!(rule.consume, *consume, "{surface} {notation} consumption");
            assert_eq!(rule.repeat, *repeat, "{surface} {notation} repeat");
            assert_eq!(rule.shift_tolerant, *shift, "{surface} {notation} <shift>");
            assert!(
                !rule.nowait,
                "{surface} {notation} — nothing ships <nowait>"
            );
            assert_eq!(
                rule.owner,
                MappingOwner::Host(expected_owner(surface).into()),
                "{surface} {notation}"
            );
        }
    }

    /// Which provider owns each surface's defaults.
    fn expected_owner(surface: SurfaceId) -> &'static str {
        match surface {
            "panel" => "godotvim.panel",
            "dock" => "godotvim.dock",
            "dock.filesystem" => "godotvim.filesystem",
            "dock.debugger" => "godotvim.debugger",
            "editor.completion" => "godotvim.completion",
            "searchbox" => "godotvim.searchbox",
            other => unreachable!("no provider ships defaults for '{other}'"),
        }
    }

    #[test]
    fn exactly_fourteen_defaults_opt_into_the_positional_probe() {
        // This flag is the whole reason `Probes::iter` and `iter_typed` are
        // two functions. Generalizing it would convert a Dvorak `Ctrl+d` into
        // panel navigation; withholding it would deny a Cyrillic user the
        // shipped keyset. Fourteen is the answer, and it is asserted rather
        // than commented.
        let index = builtin_index(&registry());
        assert_eq!(index.rules().filter(|r| r.physical).count(), 14);
    }

    #[test]
    fn exactly_two_defaults_are_shift_tolerant() {
        // Three distinct shift regimes exist today and only the filter box
        // tolerates it. A third shift-tolerant rule would start firing
        // `godotvim.item.activate` on Shift+Enter in a dock.
        let index = builtin_index(&registry());
        let tolerant: Vec<_> = index
            .rules()
            .filter(|r| r.shift_tolerant)
            .map(|r| r.surface)
            .collect();
        assert_eq!(tolerant, vec!["searchbox", "searchbox"]);
    }

    #[test]
    fn every_default_target_resolves_and_every_surface_is_declared() {
        // V9, referential integrity, over the layer that ships.
        let registry = registry();
        let index = builtin_index(&registry);
        for rule in index.rules() {
            assert!(
                index.forest().get(rule.surface).is_some(),
                "{}",
                rule.surface
            );
            let RuleTarget::Action(id) = rule.target else {
                panic!("no shipped default uses a non-action target");
            };
            assert!(registry.get(id).is_some());
            assert!(!rule.desc.is_empty(), "a rule with no description");
        }
    }

    #[test]
    fn no_default_lands_on_a_barrier_surface_or_reserves_a_prefix() {
        let index = builtin_index(&registry());
        for rule in index.rules() {
            let spec = index.forest().get(rule.surface).expect("declared");
            assert_ne!(spec.seal, Seal::Barrier, "{}", rule.surface);
            assert_eq!(rule.lhs.len(), 1, "no shipped default is multi-key");
        }
    }

    #[test]
    fn the_shift_tolerant_defaults_match_their_shifted_spelling_too() {
        // The expansion is invisible in `rules()` — one rule, two LHS — so
        // this checks the trie directly. Shift+Esc must leave the filter box.
        let index = builtin_index(&registry());
        for key in [Key::Enter, Key::Escape] {
            let bare = KeyEvent::new(key, Modifiers::NONE);
            let shifted = KeyEvent::new(key, Modifiers::SHIFT);
            let a = resolve(&index, "searchbox", &[bare]).expect("bare");
            let b = resolve(&index, "searchbox", &[shifted]).expect("shifted");
            assert_eq!(a, b, "{key:?} — both spellings must reach ONE rule");
        }
    }

    #[test]
    fn a_dock_rule_is_not_shift_tolerant() {
        // Half of the asymmetry the old dock handler had, pinned at the index
        // level: Shift+j must not navigate. It is `J`, a different key. The
        // other half — the filter box tolerating exactly this — is
        // `the_shift_tolerant_defaults_match_their_shifted_spelling_too`.
        let index = builtin_index(&registry());
        assert!(resolve(&index, "dock", &[ch('j')]).is_some());
        assert!(resolve(
            &index,
            "dock",
            &[KeyEvent::new(Key::Char('j'), Modifiers::SHIFT)]
        )
        .is_none());
    }

    #[test]
    fn building_the_index_twice_yields_the_same_thing() {
        // Registration order is iteration order, and the introspector's
        // golden snapshots depend on it. A hashed surface map would pass
        // every other test here and fail this one.
        let registry = registry();
        let a = builtin_index(&registry);
        let b = builtin_index(&registry);
        let names = |i: &BindingIndex| {
            i.rules()
                .map(|r| (r.surface, r.lhs.clone(), r.target.clone()))
                .collect::<Vec<_>>()
        };
        assert_eq!(names(&a), names(&b));
    }

    // ── Registration-time validation ─────────────────────────────────

    #[test]
    fn an_unknown_action_id_is_rejected_and_binds_nothing() {
        // A typo must never become a silent dead key: a rule that loaded with
        // no verb would consume the keystroke and do nothing, which the user
        // experiences as a broken keyboard.
        let mut index = empty_index();
        let reject = user_reject(&mut index, "panelmap dock j godotvim.item.nextt");
        assert_eq!(
            reject,
            Some(RuleReject::UnknownAction("godotvim.item.nextt".into()))
        );
        assert!(index.is_empty(), "nothing may be inserted");
        assert!(resolve(&index, "dock", &[ch('j')]).is_none());
    }

    #[test]
    fn the_unknown_action_diagnostic_names_the_line_and_the_action() {
        let mut index = empty_index();
        let mut diagnostics = Vec::new();
        apply_text(
            &mut index,
            &registry(),
            "\" a comment\npanelmap dock j godotvim.item.next\npanelmap dock k godotvim.nope",
            &MappingOwner::User,
            "user://.godot-vimrc",
            Provenance::User,
            &mut diagnostics,
        );
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].line, Some(3));
        let text = diagnostics[0].to_string();
        assert!(text.contains("user://.godot-vimrc:3"), "{text}");
        assert!(text.contains("godotvim.nope"), "{text}");
        // ...and the good line on either side of it still loaded.
        assert!(resolve(&index, "dock", &[ch('j')]).is_some());
    }

    #[test]
    fn an_unknown_surface_is_rejected() {
        // `dock.profiler` is deliberately a *plausible* name: it is exactly
        // what a user would guess after reading about `dock.debugger`, and
        // guessing must produce a diagnostic rather than a silently ignored
        // line. (This test used to name `dock.debugger` itself, which stopped
        // being undeclared the moment P9 shipped the provider.)
        let mut index = empty_index();
        assert_eq!(
            user_reject(&mut index, "panelmap dock.profiler j godotvim.item.next"),
            Some(RuleReject::UnknownSurface("dock.profiler".into()))
        );
        assert_eq!(
            user_reject(&mut index, "panelunmap dock.profiler j"),
            Some(RuleReject::UnknownSurface("dock.profiler".into()))
        );
    }

    #[test]
    fn barrier_surfaces_take_no_rules() {
        // Dispatch returns `Ignore` before any lookup on these, so a rule is
        // unreachable by construction — and accepting one would let a project
        // vimrc claim keys inside a Project Settings LineEdit or in Insert
        // mode, where Ctrl+H is backspace.
        let mut index = empty_index();
        for line in [
            "panelmap foreign <Esc> godotvim.focus.editor",
            "panelmap editor.insert <C-h> godotvim.focus.left",
        ] {
            assert_eq!(
                user_reject(&mut index, line),
                Some(RuleReject::BarrierSurface(if line.contains("foreign") {
                    "foreign"
                } else {
                    "editor.insert"
                })),
                "{line}"
            );
        }
        assert!(index.is_empty());
    }

    #[test]
    fn every_barrier_surface_in_the_forest_is_covered_by_that_rule() {
        // Stated over the forest rather than over two literals, so a new
        // Barrier surface inherits the ban instead of quietly escaping it.
        let mut index = empty_index();
        let barriers: Vec<SurfaceId> = crate::actions::providers::surfaces()
            .iter()
            .filter(|s| s.seal == Seal::Barrier)
            .map(|s| s.id)
            .collect();
        assert_eq!(barriers, vec!["editor.insert", "foreign"]);
        for surface in barriers {
            assert_eq!(
                user_reject(
                    &mut index,
                    &format!("panelmap {surface} x godotvim.item.next")
                ),
                Some(RuleReject::BarrierSurface(surface))
            );
        }
    }

    #[test]
    fn editor_reachable_surfaces_are_single_key_only() {
        // `panel` is `editor.nav`'s declared parent, so a rule there is live
        // while the script editor has focus. "Reject on editor.* surfaces"
        // alone would miss it, which is why the predicate asks the FOREST.
        let mut index = empty_index();
        assert_eq!(
            user_reject(&mut index, "panelmap panel <C-w>h godotvim.focus.left"),
            Some(RuleReject::MultiKeyOnEditorPath("panel"))
        );
        assert_eq!(
            user_reject(&mut index, "panelmap editor.nav gg godotvim.focus.left"),
            Some(RuleReject::MultiKeyOnEditorPath("editor.nav"))
        );
        assert!(index.is_empty());
    }

    #[test]
    fn editor_reachability_is_a_forest_property_not_a_name_prefix() {
        // The falsifiable form of the previous test: `panel` does not start
        // with `editor.`, and it must still be caught. `dock` is not an
        // ancestor of any editor surface, and must not be.
        let index = empty_index();
        // `editor.completion` is reachable by its own name and by nothing
        // else — it is a root with no probe, dispatched by direct lookup from
        // `gui_input`. It must still be caught, because V8's multi-key and
        // grammar-prefix rejections are exactly what stop a user binding
        // `<C-w>` there and breaking `<C-w>s` inside the editor.
        for surface in ["panel", "editor.nav", "editor.insert", "editor.completion"] {
            assert!(index.editor_reachable(surface), "{surface}");
        }
        for surface in [
            "dock",
            "dock.filesystem",
            "dock.debugger",
            "searchbox",
            "prompt",
            "foreign",
            "unknown",
        ] {
            assert!(!index.editor_reachable(surface), "{surface}");
        }
    }

    #[test]
    fn multi_key_is_legal_off_the_editor_path() {
        // §6.4 example 4b. The same LHS that `panel` refuses is fine on
        // `dock`, and it reserves the bare first key there.
        let mut index = empty_index();
        user_ok(
            &mut index,
            "panelmap dock <C-w>h godotvim.focus.left\npanelmap dock <C-w>l godotvim.focus.right",
        );
        assert_eq!(index.len(), 2);
        assert!(index.is_reserved("dock", ctrl('w')));
        assert!(!index.is_reserved("dock", ctrl('h')));
        assert!(matches!(
            index.lookup("dock", &[ctrl('w')]),
            TrieLookup::Prefix { exact: None }
        ));
    }

    #[test]
    fn a_vim_grammar_prefix_is_refused_on_an_editor_reachable_surface() {
        // The real `<C-w>` hole, closed at registration by vim-core's own
        // parser rather than by a denylist that rots. Consuming `<C-w>` at
        // `_input()` would turn `<C-w>s` into a bare `s` — a destructive edit.
        let mut index = empty_index();
        assert_eq!(
            user_reject(&mut index, "panelmap panel <C-w> godotvim.focus.left"),
            Some(RuleReject::VimGrammarPrefix(ctrl('w')))
        );
        assert_eq!(
            user_reject(&mut index, "panelmap panel <C-\\> godotvim.focus.left"),
            Some(RuleReject::VimGrammarPrefix(ctrl('\\')))
        );
        // ...and the diagnostic says which key and why.
        let text = RuleReject::VimGrammarPrefix(ctrl('w')).to_string();
        assert!(text.contains("<C-w>"), "{text}");
        assert!(text.contains("destroy the key that follows"), "{text}");
    }

    #[test]
    fn the_shipped_panel_chords_survive_the_grammar_guard() {
        // The other half, and the reason the guard cannot simply refuse every
        // Ctrl chord: these four ARE the plugin's default keyset.
        let mut index = empty_index();
        user_ok(
            &mut index,
            "panelmap panel <C-h> godotvim.focus.left\n\
             panelmap panel <C-j> godotvim.focus.down\n\
             panelmap panel <C-k> godotvim.focus.up\n\
             panelmap panel <C-l> godotvim.focus.right",
        );
        assert_eq!(index.len(), 4);
    }

    #[test]
    fn a_grammar_prefix_is_legal_off_the_editor_path() {
        // `dock` is not an ancestor of any editor surface, so `<C-w>` there
        // can never reach the vim grammar. The guard must be scoped, not
        // global, or §6.4 example 4b would be unrepresentable.
        let mut index = empty_index();
        user_ok(&mut index, "panelmap dock <C-w> godotvim.focus.left");
        assert!(resolve(&index, "dock", &[ctrl('w')]).is_some());
    }

    #[test]
    fn a_count_out_of_range_never_reaches_the_index() {
        let mut index = empty_index();
        assert!(matches!(
            user_reject(&mut index, "panelmap dock j godotvim.item.next count=5000"),
            Some(RuleReject::Parse(PanelParseError::CountOutOfRange(5000)))
        ));
        assert!(index.is_empty());
    }

    // ── Provenance ───────────────────────────────────────────────────

    #[test]
    #[should_panic(expected = "a shipped panelmap default must always load")]
    fn a_malformed_builtin_default_is_a_programming_error() {
        // Warn-and-skip is the policy for USER text. A shipped default that
        // does not load is a keyset regression that would otherwise ship with
        // a green test suite.
        let mut index = empty_index();
        let mut diagnostics = Vec::new();
        apply_text(
            &mut index,
            &registry(),
            "panelmap dock j godotvim.typo",
            &MappingOwner::Host("godotvim.test".into()),
            "godotvim.test",
            Provenance::Builtin,
            &mut diagnostics,
        );
    }

    #[test]
    fn a_malformed_user_line_is_warn_and_skip() {
        // The same text, the other provenance: no panic, one diagnostic, and
        // every other line in the file still loads.
        let mut index = empty_index();
        let mut diagnostics = Vec::new();
        apply_text(
            &mut index,
            &registry(),
            "panelmap dock j godotvim.typo\npanelmap dock k godotvim.item.prev",
            &MappingOwner::User,
            "test",
            Provenance::User,
            &mut diagnostics,
        );
        assert_eq!(diagnostics.len(), 1);
        assert!(resolve(&index, "dock", &[ch('k')]).is_some());
    }

    // ── Upsert, unmap and last-writer-wins ───────────────────────────

    #[test]
    fn the_last_writer_wins_at_one_surface_and_lhs() {
        // Forced by the trie, not chosen — and it is what makes rebinding
        // work at all.
        let mut index = empty_index();
        user_ok(
            &mut index,
            "panelmap dock j godotvim.item.next\npanelmap dock j godotvim.item.prev",
        );
        assert_eq!(index.len(), 1, "one rule per (surface, lhs)");
        let registry = registry();
        assert_eq!(
            resolve(&index, "dock", &[ch('j')]).map(|r| r.target.clone()),
            Some(RuleTarget::Action(
                registry.id_of("godotvim.item.prev").unwrap()
            ))
        );
    }

    #[test]
    fn re_inserting_reuses_the_slot_rather_than_orphaning_the_old_rule() {
        // Without slot reuse the superseded rule would still be listed by
        // `:panelmap`, and the user would see two bindings on one key.
        let mut index = empty_index();
        user_ok(&mut index, "panelmap dock j godotvim.item.next");
        let before: Vec<_> = index.rules().map(|r| r.lhs.clone()).collect();
        user_ok(&mut index, "panelmap dock j godotvim.item.prev");
        let after: Vec<_> = index.rules().map(|r| r.lhs.clone()).collect();
        assert_eq!(before, after);
    }

    #[test]
    fn the_same_lhs_on_two_surfaces_is_two_rules() {
        // §6.4 example 9: in the FileSystem dock `r` renames, everywhere else
        // it moves to the previous item. Depth is the specificity mechanism.
        let mut index = empty_index();
        user_ok(
            &mut index,
            "panelmap dock r godotvim.item.prev\npanelmap dock.filesystem r godotvim.fs.rename",
        );
        assert_eq!(index.len(), 2);
        assert_ne!(
            resolve(&index, "dock", &[ch('r')]).map(|r| r.target.clone()),
            resolve(&index, "dock.filesystem", &[ch('r')]).map(|r| r.target.clone())
        );
    }

    #[test]
    fn unmapping_removes_the_rule_and_leaves_no_trie_entry() {
        let mut index = empty_index();
        user_ok(
            &mut index,
            "panelmap dock.filesystem y godotvim.fs.yank_path\npanelunmap dock.filesystem y",
        );
        assert!(index.is_empty());
        assert_eq!(
            index.lookup("dock.filesystem", &[ch('y')]),
            TrieLookup::NoMatch
        );
    }

    #[test]
    fn unmapping_a_shift_tolerant_rule_removes_its_twin_too() {
        // The twin lives only in the trie, so a naive removal would leave
        // Shift+Esc bound to a rule that no longer exists.
        let mut index = empty_index();
        user_ok(
            &mut index,
            "panelmap <shift> searchbox <Esc> godotvim.search.accept\npanelunmap searchbox <Esc>",
        );
        assert_eq!(
            index.lookup("searchbox", &[KeyEvent::new(Key::Escape, Modifiers::SHIFT)]),
            TrieLookup::NoMatch
        );
    }

    #[test]
    fn unmapping_drops_the_prefix_reservation_with_the_rule() {
        // Reservations are derived from the live slot table rather than
        // stored beside it, so this cannot go stale.
        let mut index = empty_index();
        user_ok(&mut index, "panelmap dock <C-w>h godotvim.focus.left");
        assert!(index.is_reserved("dock", ctrl('w')));
        user_ok(&mut index, "panelunmap dock <C-w>h");
        assert!(!index.is_reserved("dock", ctrl('w')));
    }

    #[test]
    fn unmapping_something_that_was_never_bound_is_a_no_op() {
        let mut index = empty_index();
        user_ok(&mut index, "panelunmap dock q");
        assert!(index.is_empty());
    }

    #[test]
    fn a_rebind_is_an_unmap_followed_by_a_map() {
        // §6.4 example 2, end to end: `panelunmap` and `panelmap` are separate
        // verbs and rebinding needs both.
        let registry = registry();
        let mut index = builtin_index(&registry);
        user_ok(
            &mut index,
            "panelunmap panel <C-h>\npanelmap <physical> <void> <norepeat> panel <M-h> godotvim.focus.left",
        );
        assert!(resolve(&index, "panel", &[ctrl('h')]).is_none());
        let alt_h = KeyEvent::new(Key::Char('h'), Modifiers::ALT);
        assert_eq!(
            resolve(&index, "panel", &[alt_h]).map(|r| r.target.clone()),
            Some(RuleTarget::Action(
                registry.id_of("godotvim.focus.left").unwrap()
            ))
        );
        assert_eq!(index.len(), SHIPPED_DEFAULTS.len());
    }

    // ── Targets that are not actions ─────────────────────────────────

    #[test]
    fn native_is_legal_on_every_non_barrier_surface() {
        // It can only REDUCE what the plugin consumes, so it is permitted at
        // every trust tier and on every surface that takes rules at all.
        let mut index = empty_index();
        for surface in [
            "panel",
            "dock",
            "dock.filesystem",
            "searchbox",
            "editor.nav",
        ] {
            user_ok(&mut index, &format!("panelmap {surface} <C-h> native"));
            assert_eq!(
                resolve(&index, surface, &[ctrl('h')]).map(|r| r.target.clone()),
                Some(RuleTarget::Native),
                "{surface}"
            );
        }
    }

    #[test]
    fn a_shortcut_target_is_refused_because_nothing_dispatches_it() {
        // `run_candidate` returns `Outcome::Declined` for every `<Shortcut>`
        // after a `log::warn!` nobody sees — the default Log Level is Off — so
        // the rule loaded cleanly, printed as "eligible" in `:panelmap`, and
        // was a permanently dead key the introspector actively confirmed would
        // work. With `<void>` it swallowed the key as well. The parser still
        // understands the syntax (see `config::panelmap`); what changed is
        // that the plane refuses to pretend it can honour it.
        let mut index = empty_index();
        assert_eq!(
            user_reject(
                &mut index,
                "panelmap dock.filesystem <C-r> <Shortcut>(filesystem_dock/rename)"
            ),
            Some(RuleReject::UndispatchedTarget {
                surface: "dock.filesystem",
                target: "<Shortcut>(filesystem_dock/rename)".into(),
            })
        );
        assert!(
            resolve(&index, "dock.filesystem", &[ctrl('r')]).is_none(),
            "nothing may be inserted"
        );
        // The example the debugger provider's own docs use, from the surface
        // the docs use it on.
        assert!(user_reject(
            &mut index,
            "panelmap dock.debugger s <Shortcut>(debugger/step_over)"
        )
        .is_some());
        assert!(resolve(&index, "dock.debugger", &[ch('s')]).is_none());
    }

    #[test]
    fn a_capability_bearing_verb_bound_on_an_overlay_is_gated_by_the_pipeline() {
        // THE file-deleting one, same force, new wall. `godotvim.fs.delete`
        // is `run: |_cx| filesystem_explorer::delete_selected()`: it never
        // reads its ctx and drives Godot's own FileSystem delete. On the old
        // transport-only surface the capability gate structurally could not
        // run, so this line was refused at registration. The overlay resolves
        // through the one pipeline now, so the line LOADS, and the gate that
        // stops it is `hit_from`'s: the overlay's caps are `Caps::POPUP` at
        // most, never `FILEOPS`, so the walk exhausts and nothing runs.
        let mut index = empty_index();
        user_ok(
            &mut index,
            "panelmap editor.completion <C-y> godotvim.fs.delete",
        );
        assert!(
            resolve(&index, "editor.completion", &[ctrl('y')]).is_some(),
            "the rule installs; the pipeline is the gate"
        );

        let forest = crate::actions::providers::forest();
        let path = forest
            .overlay(&crate::actions::surface::OverlayFacts {
                at_attached_editor: true,
                mode: Some(vim_core::primitives::Mode::Insert),
                selected_index: 0,
            })
            .expect("popup-open facts activate the overlay");
        let reg = registry();
        let probes = crate::actions::keys::Probes::from_key(ctrl('y'));
        let claims = |_: KeyEvent| false;
        let resolution = crate::actions::resolve::resolve(&crate::actions::resolve::ResolveInput {
            probes: &probes,
            path: &path,
            index: &index,
            registry: &reg,
            vim_claims: &claims,
        });
        assert_eq!(
            resolution,
            crate::actions::resolve::Resolution::None(crate::actions::resolve::Stop::Exhausted),
            "nothing runs and no file is deleted"
        );

        // …and the verbs that surface is FOR still install and resolve.
        user_ok(
            &mut index,
            "panelmap editor.completion <C-y> godotvim.completion.confirm",
        );
        assert!(resolve(&index, "editor.completion", &[ctrl('y')]).is_some());
    }

    #[test]
    fn a_capability_bearing_verb_is_still_fine_on_a_classified_surface() {
        // The guard on the guard, retargeted: `panel` has no parent, exactly
        // as the overlay has none, and its rules must keep installing and
        // resolving through the classified pipeline.
        let mut index = empty_index();
        user_ok(&mut index, "panelmap panel <C-y> godotvim.item.next");
        assert!(resolve(&index, "panel", &[ctrl('y')]).is_some());
    }

    #[test]
    fn every_shipped_completion_default_still_loads() {
        // The count the reject must not move: eight rules on
        // `editor.completion`, six distinct verbs, every one `Caps::empty()`.
        let index = builtin_index(&registry());
        assert_eq!(index.rules_on("editor.completion").count(), 8);
        let reg = registry();
        for rule in index.rules_on("editor.completion") {
            let RuleTarget::Action(id) = rule.target else {
                panic!("a completion default must target an action");
            };
            assert!(
                reg.get(id).is_some_and(|s| s.requires.is_empty()),
                "{:?} needs capabilities this surface cannot grant",
                rule.lhs
            );
        }
    }

    // ── The trie contract this index is built on ─────────────────────

    #[test]
    fn the_trie_payload_is_an_opaque_slot_and_nothing_else() {
        // Metadata in the trie would make `remove_by_owner` able to delete a
        // builtin when a third party unregisters, and would make
        // `entries()` — which renders a slot as literally `<Action>(7)` —
        // the listing source of truth.
        let mut index = empty_index();
        user_ok(&mut index, "panelmap dock j godotvim.item.next");
        let TrieLookup::ExactOnly(entry) = index.lookup("dock", &[ch('j')]) else {
            panic!("expected an exact match");
        };
        assert_eq!(entry.sequence().len(), 1);
        assert!(matches!(entry.sequence()[0].key(), Key::Action(_)));
        assert!(BindingIndex::slot_in(entry).is_some());
    }

    #[test]
    fn nowait_reaches_the_trie_and_promotes_prefix_to_exact() {
        // The flag's only consumer. Without `Rule::nowait` the documented
        // token would be parseable and unstorable, and `new_nowait` would have
        // no caller anywhere in the crate.
        let mut index = empty_index();
        user_ok(
            &mut index,
            "panelmap dock dd godotvim.fs.delete\npanelmap dock d godotvim.item.next",
        );
        assert!(matches!(
            index.lookup("dock", &[ch('d')]),
            TrieLookup::Prefix { exact: Some(_) }
        ));

        let mut index = empty_index();
        user_ok(
            &mut index,
            "panelmap dock dd godotvim.fs.delete\npanelmap <nowait> dock d godotvim.item.next",
        );
        assert!(
            matches!(index.lookup("dock", &[ch('d')]), TrieLookup::ExactOnly(_)),
            "<nowait> must make the shorter mapping fire immediately"
        );
    }

    #[test]
    fn a_lookup_on_an_unbound_surface_is_a_miss_rather_than_a_panic() {
        let index = empty_index();
        assert_eq!(index.lookup("dock", &[ch('j')]), TrieLookup::NoMatch);
        assert!(index.rule_at(SlotId(99)).is_none());
        assert!(!index.is_reserved("dock", ch('j')));
    }

    // ── shift_variant ────────────────────────────────────────────────

    #[test]
    fn only_a_bare_single_named_key_gets_a_shifted_twin() {
        // `translate_key` strips SHIFT for printables, so `Char('r')+SHIFT` is
        // an event the runtime cannot produce — synthesizing it would add a
        // trie entry nothing can ever match.
        assert_eq!(
            shift_variant(&[KeyEvent::new(Key::Enter, Modifiers::NONE)]),
            Some(vec![KeyEvent::new(Key::Enter, Modifiers::SHIFT)])
        );
        assert_eq!(shift_variant(&[ch('r')]), None, "printables fold instead");
        assert_eq!(
            shift_variant(&[KeyEvent::new(Key::Enter, Modifiers::CTRL)]),
            None,
            "a command chord is already distinct"
        );
        assert_eq!(
            shift_variant(&[KeyEvent::new(Key::Enter, Modifiers::SHIFT)]),
            None,
            "already shifted"
        );
        assert_eq!(
            shift_variant(&[KeyEvent::new(Key::Enter, Modifiers::NONE), ch('x')]),
            None,
            "multi-key sequences have no twin"
        );
    }

    // ── Introspection helpers P6 relies on ───────────────────────────

    #[test]
    fn any_rule_targets_answers_for_bound_and_unbound_verbs() {
        // The `:checkhealth` line for "this verb exists but no key reaches
        // it" — `godotvim.focus.cycle_next` ships no default on purpose,
        // because nothing is free on `panel`.
        let registry = registry();
        let index = builtin_index(&registry);
        assert!(index.any_rule_targets(registry.id_of("godotvim.item.next").unwrap()));
        assert!(!index.any_rule_targets(registry.id_of("godotvim.focus.cycle_next").unwrap()));
        assert!(!index.any_rule_targets(ActionId(9999)));
    }

    #[test]
    fn a_capability_impossible_binding_is_still_registered() {
        // Stated so it is not mistaken for an omission: the capability gate
        // lives on the RESOLVER, not here. `godotvim.fs.create` on `dock` is a
        // legal rule that simply never fires, and turning it into a
        // registration error would break §6.4 example 10.
        let mut index = empty_index();
        user_ok(&mut index, "panelmap dock a godotvim.fs.create");
        let rule = resolve(&index, "dock", &[ch('a')]).expect("registered");
        let registry = registry();
        let RuleTarget::Action(id) = rule.target else {
            unreachable!()
        };
        assert_eq!(registry.get(id).map(|s| s.requires), Some(Caps::FILEOPS));
    }

    /// A spec that exists nowhere in `SHIPPED`, so the tests below cannot be
    /// satisfied by an accidental name collision with a real verb.
    static THIRD_PARTY: ActionSpec = ActionSpec {
        id: "thirdparty.debugger.step",
        desc: "step over",
        requires: Caps::empty(),
        host_invocable: false,
        default_consume: None,
        run: |_| Outcome::Declined,
    };

    /// A verb that declares a consumption default, so the fold in `rule_from`
    /// is observable without any shipped verb carrying one.
    static HANDS_OFF: ActionSpec = ActionSpec {
        id: "thirdparty.test.handoff",
        desc: "declares Handoff as its default consumption",
        requires: Caps::empty(),
        host_invocable: false,
        default_consume: Some(Consumption::Handoff),
        run: |_| Outcome::Handled,
    };

    #[test]
    fn a_verb_default_consumption_reaches_the_rule() {
        // The verb declares what its rules default to, folded into
        // `Rule.consume` at REGISTRATION, so `dispose` still reads one field.
        let mut registry = registry();
        registry.register(&HANDS_OFF);
        let mut index = empty_index();
        let mut diagnostics = Vec::new();
        apply_text(
            &mut index,
            &registry,
            "panelmap dock <Up> thirdparty.test.handoff",
            &MappingOwner::User,
            "test",
            Provenance::User,
            &mut diagnostics,
        );
        assert_eq!(diagnostics, Vec::new());
        let rule = resolve(&index, "dock", &[KeyEvent::new(Key::Up, Modifiers::NONE)])
            .expect("registered");
        assert_eq!(rule.consume, Consumption::Handoff);
    }

    #[test]
    fn void_on_the_rule_beats_the_verb_default() {
        // `<void>` always raises consumption; the verb default never lowers a
        // flag the user wrote.
        let mut registry = registry();
        registry.register(&HANDS_OFF);
        let mut index = empty_index();
        let mut diagnostics = Vec::new();
        apply_text(
            &mut index,
            &registry,
            "panelmap <void> dock <Up> thirdparty.test.handoff",
            &MappingOwner::User,
            "test",
            Provenance::User,
            &mut diagnostics,
        );
        assert_eq!(diagnostics, Vec::new());
        let rule = resolve(&index, "dock", &[KeyEvent::new(Key::Up, Modifiers::NONE)])
            .expect("registered");
        assert_eq!(rule.consume, Consumption::Void);
    }

    #[test]
    fn a_rule_with_no_verb_default_is_elastic() {
        // A shipped verb with no flag and no `default_consume` stays exactly
        // what it always was.
        let mut index = empty_index();
        user_ok(&mut index, "panelmap dock j godotvim.item.next");
        let rule = resolve(&index, "dock", &[ch('j')]).expect("registered");
        assert_eq!(rule.consume, Consumption::Elastic);
    }

    #[test]
    fn a_newly_registered_action_becomes_bindable_with_no_index_changes() {
        // The extensibility claim, in its smallest falsifiable form: a verb
        // the binding plane has never heard of binds through the same parser.
        let mut registry = registry();
        let mut index = empty_index();
        let mut diagnostics = Vec::new();
        let line = "panelmap dock <F5> thirdparty.debugger.step";

        apply_text(
            &mut index,
            &registry,
            line,
            &MappingOwner::User,
            "test",
            Provenance::User,
            &mut diagnostics,
        );
        assert_eq!(diagnostics.len(), 1, "unregistered — must be rejected");

        let id = registry.register(&THIRD_PARTY);
        diagnostics.clear();
        apply_text(
            &mut index,
            &registry,
            line,
            &MappingOwner::Host("thirdparty".into()),
            "thirdparty",
            Provenance::User,
            &mut diagnostics,
        );
        assert_eq!(diagnostics, Vec::new());
        assert!(index.any_rule_targets(id));
    }
}
