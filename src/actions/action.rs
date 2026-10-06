//! Named actions: identity and parameters.
//!
//! Every shell-side verb gets a stable dotted name (`godotvim.item.next`) and
//! an interned id. Naming is what dissolves the asymmetry the design set out
//! to fix: inside the editor `<C-w>h` already had a name and was therefore
//! remappable, while the identical action from a dock was an anonymous match
//! arm. Once both address one `ActionSpec`, one binding table serves both.
//!
//! Ids are minted by the shell's own [`vim_core::keymap::NameRegistry`], the
//! same interner vim-core uses for `<Plug>` and `<Action>` pseudo-keys, so an
//! id can be carried inside a `KeyEvent` as `Key::Action(u32)` and flow
//! through machinery that knows nothing about Godot.

use compact_str::CompactString;
use godot::classes::Control;
use godot::prelude::*;
use vim_core::keymap::KeyEvent;

use super::bind::Consumption;
use super::caps::Caps;
use super::outcome::Outcome;

/// A registered action, identified by its interned id.
///
/// `Copy` and pointer-width so it can live inside a `Key` without making
/// `Key` allocate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) struct ActionId(pub(crate) u32);

#[allow(
    dead_code,
    reason = "pseudo-key form for the shell's own use; the engine crossing is by NAME"
)]
impl ActionId {
    /// The pseudo-key carrying this id.
    ///
    /// **Not** the shell↔engine crossing. `ActionNames` wraps its own
    /// `NameRegistry`, a different instance from the engine's, so handing a
    /// raw id across would resolve against the wrong table — actions cross by
    /// NAME. This is for the shell's own internal use.
    pub(crate) const fn as_key(self) -> KeyEvent {
        KeyEvent::action(self.0)
    }
}

/// Upper bound on any repeat count.
///
/// Not a style choice. `find_navigable_target` walks up to `MAX_ATTEMPTS`
/// (1000) items per call, so an unbounded count is a frozen editor rather
/// than a slow keystroke. Always reach a count through [`Params::count`],
/// never by reading the raw integer.
pub(crate) const MAX_ACTION_COUNT: i64 = 100;

/// Scalar arguments attached to a binding.
///
/// Values are **decimal integers only**, and the grammar must not grow a
/// string or enum form. A closed integer vocabulary is what makes extending
/// the config sandbox provable: a parameter can never expand into `:!`,
/// `:source`, or a recursive mapping chain, so a binding read from a
/// project-level vimrc cannot become code execution.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Params(Vec<(CompactString, i64)>);

impl Params {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Read a parameter, or `default` if absent.
    pub(crate) fn int(&self, key: &str, default: i64) -> i64 {
        self.0
            .iter()
            .find(|(k, _)| k.as_str() == key)
            .map_or(default, |(_, v)| *v)
    }

    /// Set a parameter, replacing any existing value for `key`.
    ///
    /// Last writer wins, matching how a later config line overrides an
    /// earlier one.
    pub(crate) fn set_int(&mut self, key: &str, value: i64) {
        match self.0.iter_mut().find(|(k, _)| k.as_str() == key) {
            Some((_, v)) => *v = value,
            None => self.0.push((CompactString::from(key), value)),
        }
    }

    /// The repeat count, clamped to a survivable range.
    ///
    /// **Always use this for repeat loops.** Reading `int("count", 1)`
    /// directly re-opens the freeze that [`MAX_ACTION_COUNT`] exists to close,
    /// and a zero or negative count would silently mean "do nothing" rather
    /// than "do it once".
    pub(crate) fn count(&self) -> u32 {
        self.int("count", 1).clamp(1, MAX_ACTION_COUNT) as u32
    }

    /// Test-only: `panelmap`'s introspector prints parameters rather than
    /// asking whether there are any.
    #[allow(dead_code, reason = "asserted by tests; the introspector uses `iter`")]
    pub(crate) fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Parameters in insertion order, for the introspector.
    pub(crate) fn iter(&self) -> impl Iterator<Item = (&str, i64)> + '_ {
        self.0.iter().map(|(k, v)| (k.as_str(), *v))
    }
}

/// Whether `name` is a well-formed action id.
///
/// The dot is load-bearing, not cosmetic: the host bridge splits its own
/// namespace from Godot's editor-shortcut namespace on exactly this
/// character, so an id without one would be ambiguous with a shortcut path.
pub(crate) fn is_valid_action_id(name: &str) -> bool {
    name.contains('.')
        && !name.contains('/')
        && !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '_')
}

/// Interner mapping action names to [`ActionId`]s.
///
/// Wraps vim-core's `NameRegistry`, which is append-only and idempotent, so
/// an id stays stable for the life of the process even across config reloads.
#[derive(Debug, Default)]
pub(crate) struct ActionNames {
    inner: vim_core::keymap::NameRegistry,
}

impl ActionNames {
    /// Test-only: production interners arrive through `ActionRegistry`'s
    /// `Default`, which builds this field for them.
    #[allow(dead_code, reason = "production builds ActionNames via Default")]
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Intern `name`, returning its stable id.
    ///
    /// Idempotent: registering the same name twice yields the same id.
    ///
    /// # Panics (debug only)
    /// If `name` is not a valid action id. Registration is a compile-time-ish
    /// event driven by `const` provider tables, so a malformed id is a
    /// programming error, not user input.
    pub(crate) fn intern(&mut self, name: &str) -> ActionId {
        debug_assert!(
            is_valid_action_id(name),
            "action id '{name}' must be dotted, slash-free and alphanumeric"
        );
        ActionId(self.inner.register(name))
    }

    pub(crate) fn name_of(&self, id: ActionId) -> Option<&str> {
        self.inner.get_name(id.0)
    }

    pub(crate) fn id_of(&self, name: &str) -> Option<ActionId> {
        self.inner.get_id(name).map(ActionId)
    }
}

/// Where a binding sends a keystroke.
///
/// `Native` is the give-back: it means "this key is not mine", and is the one
/// target legal at every trust tier because it can only *reduce* what the
/// plugin consumes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum RuleTarget {
    /// Run one of the plugin's own registered actions.
    Action(ActionId),
    /// Hand the key back to Godot untouched.
    Native,
    /// Delegate to one of Godot's own registered editor shortcuts.
    Shortcut(CompactString),
}

/// A named verb: what it is called, what it needs, and what it does.
///
/// Values are `static`s. `Caps` combinators are `const fn` in bitflags 2, and
/// a non-capturing closure coerces to a `fn` pointer in a static initializer,
/// so `run: |cx| { … }` is legal there.
pub(crate) struct ActionSpec {
    pub(crate) id: &'static str,
    pub(crate) desc: &'static str,
    /// Consulted only when the binding resolver ranks candidates for a
    /// keystroke. Host-originated invocation does not consult it — `:action`
    /// names the verb directly, so there is no candidate list to rank.
    pub(crate) requires: Caps,
    /// False ⇒ a host request fails loudly ("requires panel focus") rather
    /// than declining invisibly. `godotvim.fs.*` can locate their own target
    /// and are true; `godotvim.item.*` need a focused control and are false.
    pub(crate) host_invocable: bool,
    /// The consumption policy a rule gets when it names this verb and
    /// declares no flag of its own. `None` means `Elastic`; `<void>` on the
    /// rule always wins.
    ///
    /// Folded into `Rule.consume` at REGISTRATION by `bind::rule_from`, so
    /// `dispose` still reads exactly one field and the invariant in
    /// `resolve.rs` holds: this is the verb declaring what its rules default
    /// to, never a body deciding at run time. Constrained by audit A9'.
    pub(crate) default_consume: Option<Consumption>,
    pub(crate) run: fn(&mut ActionCtx<'_>) -> Outcome,
}

impl std::fmt::Debug for ActionSpec {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ActionSpec")
            .field("id", &self.id)
            .field("requires", &self.requires)
            .finish_non_exhaustive()
    }
}

/// The autocomplete popup, as questions and commands.
///
/// A trait rather than a concrete struct for one reason that decides it: the
/// only real implementation holds `&mut VimSession<GodotHost>` and
/// `&mut Gd<CodeEdit>`, and neither can exist under `cargo test` in a
/// `cdylib`. Behind this seam the `godotvim.completion.*` bodies are pure
/// decision logic over one boolean, two integers and one provenance answer,
/// so the trichotomy they
/// produce — consume / hand to the control / let the engine have it — is
/// table-tested headlessly, which is the same trick `FocusChain` plays for the
/// surface plane.
///
/// It is deliberately narrow. Nothing here can reach the document, the mode or
/// the engine: a completion verb decides *routing*, and the text reconciliation
/// that `confirm` triggers stays on the transport side where the session lives.
pub(crate) trait CompletionOps {
    /// Godot returns -1 from `get_code_completion_selected_index` when no
    /// popup is up, which is the only "is it visible" answer it offers.
    ///
    /// Read LIVE, where `Caps::POPUP` is sampled once at the keystroke: the
    /// capability answers "was the popup up when the key arrived", a routing
    /// precondition; this answers "is it up now", an execution fact. `PREV`
    /// depends on the two disagreeing within one keystroke: it requests, then
    /// asks whether the popup came up so it can land on the last candidate.
    fn popup_visible(&self) -> bool;
    fn option_count(&self) -> i32;
    fn selected_index(&self) -> i32;
    /// Ask Godot to (re)build the candidate list. Synchronous: popup state and
    /// selected index are valid immediately after the call.
    fn request(&mut self, force: bool);
    fn select(&mut self, index: i32);
    /// The user pressed a key `CodeEdit` moves the selection on by itself.
    fn navigated(&mut self);
    /// Accept the selected candidate and reconcile the text delta with the
    /// engine, so dot-repeat and macro recording capture the completed text.
    fn confirm(&mut self);
    fn cancel(&mut self);
    /// Whether the current selection was chosen by the user rather than
    /// preselected by Godot.
    ///
    /// Answered from the provenance machine on the port. Sound because
    /// `effects::completion` re-requests on the raw editor and never touches
    /// this port, so a port `request` is reachable only from a user-initiated
    /// verb.
    fn selection_is_explicit(&self) -> bool;
}

/// The focused panel control, as commands.
///
/// The same seam as [`CompletionOps`], for the same reason, applied to the
/// executors that reach the scene tree. [`ActionCtx::new`] needs a
/// `Gd<Control>`, which cannot be constructed in a `cdylib` under `cargo test`,
/// so every body that opened with `let Some(target) = cx.target() else { return
/// Declined }` short-circuited before its first decision. `Declined` was then
/// produced for three indistinguishable reasons, and everything downstream of
/// the guard — the direction, the repeat count, the break-on-false, and the
/// polarity of the resulting [`Outcome`] — was freely mutable under a green
/// suite. Behind this trait those decisions are plain data over a `bool` and an
/// enum, and a recording fake asserts them.
///
/// It is **not** `navigation::dock_nav::NavigableItem`. That trait is a cursor
/// over one widget's visible-item chain (`next_visible` / `prev_visible` /
/// `is_selectable`), it knows nothing about selecting, scrolling or the
/// Tree/ItemList/RichTextLabel dispatch that `handle_navigation` performs, and
/// it is already faked by `dock_nav`'s own `MockItem`. This is the layer above
/// it: one keystroke's worth of effect on a whole control.
///
/// Production reaches it through the target itself — `Gd<Control>` is the only
/// shipped implementor, and [`ActionCtx::panel_ops`] hands it out — so there is
/// exactly one code path and the fake substitutes for the widget, never for the
/// executor.
pub(crate) trait PanelOps {
    /// One step in `direction`. `false` means "nothing to move to" — the end
    /// of a list — which the caller turns into a declination so the key falls
    /// through to Godot rather than dying here.
    fn nav_step(&mut self, direction: crate::navigation::dock_nav::NavDirection) -> bool;
    /// Expand or collapse, with the same `false`-is-a-declination contract.
    fn hierarchy_step(&mut self, action: crate::navigation::dock_nav::HierarchyAction) -> bool;
    /// Directional cross-panel focus movement.
    fn move_focus(
        &mut self,
        direction: crate::navigation::window::WindowNavDirection,
    ) -> crate::navigation::window::WindowNavResult;
    /// Spatial cycle-focus. Returns nothing: the scene-tree walk reports its
    /// own miss by leaving focus where it was.
    fn cycle_focus(&mut self, action: crate::effects::WindowNavAction);
}

impl PanelOps for Gd<Control> {
    fn nav_step(&mut self, direction: crate::navigation::dock_nav::NavDirection) -> bool {
        crate::navigation::dock_nav::handle_navigation(self, direction, 0)
    }

    fn hierarchy_step(&mut self, action: crate::navigation::dock_nav::HierarchyAction) -> bool {
        crate::navigation::dock_nav::handle_hierarchy(self, action)
    }

    fn move_focus(
        &mut self,
        direction: crate::navigation::window::WindowNavDirection,
    ) -> crate::navigation::window::WindowNavResult {
        crate::navigation::handle_window_nav(self, direction)
    }

    fn cycle_focus(&mut self, action: crate::effects::WindowNavAction) {
        crate::navigation::handle_window_nav_action(self, action);
    }
}

/// What an action can reach while it runs.
///
/// Ships **chain-less** at this phase: the sampled focus chain arrives with
/// the surface plane, and adding it later does not change this signature.
/// `target` is `Option` because the mandatory no-focus-owner case — cross-panel
/// navigation with nothing focused, which must still consume — would otherwise
/// be unconstructible.
pub(crate) struct ActionCtx<'a> {
    /// The focused control, when there is one.
    target: Option<Gd<Control>>,
    pub(crate) params: Params,
    /// Records side effects instead of performing them, under test.
    ///
    /// Set only by [`ActionCtx::recording`], which is `#[cfg(test)]`, so the
    /// field is `None` in every production context. It is not itself
    /// `#[cfg(test)]` only because every constructor would then need cfg-ing
    /// too.
    #[allow(
        dead_code,
        reason = "written only by the #[cfg(test)] `recording` constructor"
    )]
    pub(crate) recorder: Option<&'a mut Vec<Effect>>,
    /// The FileSystem dock's prompt/executor state.
    ///
    /// Borrowed field-by-field rather than as `&mut GodotVimCore`, and that
    /// is deliberate: `run` is called from inside `handle_input_impl`, which
    /// already holds `&mut self`, and handing an action the whole plugin
    /// would make every re-entrancy question a runtime one. `None` on the
    /// host transport, where an action that needs it declines.
    fs: Option<&'a mut crate::navigation::FileSystemExplorer>,
    /// The autocomplete popup, lent by the `gui_input` transport only.
    ///
    /// Same shape and same reasoning as `fs`: borrowed as one capability
    /// rather than as the whole plugin, and `None` everywhere else — the
    /// `_input` panel dispatcher and `:action` both leave it unset, and a
    /// completion verb reached from there declines instead of half-running.
    completion: Option<&'a mut dyn CompletionOps>,
    /// A stand-in for the focused control, set **only** under test.
    ///
    /// Unlike `fs` and `completion` this lane has no transport: production
    /// always reaches [`PanelOps`] through `target`, so there is one code path
    /// and a mutation to a body is a mutation the fake sees. `None` everywhere
    /// outside `#[cfg(test)]`.
    #[cfg(test)]
    panel: Option<&'a mut dyn PanelOps>,
    /// A stand-in for the focused debugger tree, set only under test. Same
    /// arrangement and same reasoning as `panel`.
    #[cfg(test)]
    debugger: Option<&'a mut dyn super::providers::debugger::DebuggerTreeOps>,
}

/// One observable side effect of running an action.
///
/// Exists so a test can assert what an action *did* without a Godot runtime,
/// and today exactly one test spec uses it. It is **not** what protects the
/// `ItemList` activation path's double emit — `item_selected` and
/// `item_activated` both matter because different editor docks listen to
/// different ones, but `handle_enter` emits them directly on the control, not
/// through [`ActionCtx::emit`]. What protects that pair is `handle_enter`
/// being an unmodified verbatim body, and nothing else.
#[allow(
    dead_code,
    reason = "constructed only by `emit`, whose only caller is a #[cfg(test)] spec"
)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Effect {
    EmitSignal {
        name: &'static str,
        arg: Option<i64>,
    },
}

impl<'a> ActionCtx<'a> {
    /// Capabilities are deliberately NOT a field: the gate lives on the
    /// binding path (see [`ActionRegistry::caps_allow`]), so by the time a
    /// context exists the decision is made. Carrying them here would invite a
    /// `run` body to re-derive — or rewrite — its own gate result.
    pub(crate) fn new(target: Option<Gd<Control>>, params: Params) -> Self {
        Self {
            target,
            params,
            recorder: None,
            fs: None,
            completion: None,
            #[cfg(test)]
            panel: None,
            #[cfg(test)]
            debugger: None,
        }
    }

    /// Lend the FileSystem explorer for the duration of one dispatch.
    pub(crate) fn with_fs(mut self, fs: &'a mut crate::navigation::FileSystemExplorer) -> Self {
        self.fs = Some(fs);
        self
    }

    /// Lend the autocomplete popup for the duration of one dispatch.
    pub(crate) fn with_completion(mut self, ops: &'a mut dyn CompletionOps) -> Self {
        self.completion = Some(ops);
        self
    }

    /// A context that records side effects rather than performing them.
    #[cfg(test)]
    pub(crate) fn recording(sink: &'a mut Vec<Effect>) -> Self {
        Self {
            target: None,
            params: Params::new(),
            recorder: Some(sink),
            fs: None,
            completion: None,
            panel: None,
            debugger: None,
        }
    }

    /// Stand a fake in for the focused control. Test-only, by construction:
    /// production has no second source of [`PanelOps`].
    #[cfg(test)]
    pub(crate) fn with_panel_ops(mut self, ops: &'a mut dyn PanelOps) -> Self {
        self.panel = Some(ops);
        self
    }

    /// Stand a fake in for the focused debugger tree. Test-only.
    #[cfg(test)]
    pub(crate) fn with_debugger_tree(
        mut self,
        ops: &'a mut dyn super::providers::debugger::DebuggerTreeOps,
    ) -> Self {
        self.debugger = Some(ops);
        self
    }

    pub(crate) fn target(&self) -> Option<&Gd<Control>> {
        self.target.as_ref()
    }

    /// What this keystroke may do to the focused control, when there is one.
    ///
    /// `None` is the no-focus-owner case — `Anchor::Rootless` reaches `panel`'s
    /// Ctrl+hjkl rules with nothing focused — and every caller must turn it
    /// into `Declined`: `Handled` there would consume the key and do nothing.
    pub(crate) fn panel_ops(&mut self) -> Option<&mut (dyn PanelOps + 'a)> {
        #[cfg(test)]
        {
            if self.panel.is_some() {
                return self.panel.as_deref_mut();
            }
        }
        self.target.as_mut().map(|t| t as &mut (dyn PanelOps + 'a))
    }

    /// The focused control as a debugger tree, when there is one.
    ///
    /// The cast to `Tree` lives inside the implementation rather than here, so
    /// a focused `Button` inside the debugger dock answers "no selection"
    /// instead of being unrepresentable.
    pub(crate) fn debugger_tree(
        &mut self,
    ) -> Option<&mut (dyn super::providers::debugger::DebuggerTreeOps + 'a)> {
        #[cfg(test)]
        {
            if self.debugger.is_some() {
                return self.debugger.as_deref_mut();
            }
        }
        self.target
            .as_mut()
            .map(|t| t as &mut (dyn super::providers::debugger::DebuggerTreeOps + 'a))
    }

    /// The FileSystem explorer, when this transport lends one.
    pub(crate) fn fs(&mut self) -> Option<&mut crate::navigation::FileSystemExplorer> {
        self.fs.as_deref_mut()
    }

    /// The autocomplete popup, when this transport lends one.
    ///
    /// The `+ 'a` is load-bearing rather than decorative: `&mut` is invariant,
    /// so eliding it would ask the compiler to shorten a trait object's
    /// lifetime behind a mutable reference, which it correctly refuses.
    pub(crate) fn completion(&mut self) -> Option<&mut (dyn CompletionOps + 'a)> {
        self.completion.as_deref_mut()
    }

    /// Emit a Godot signal, or record the intent under test.
    ///
    /// No shipped verb routes through here yet: `handle_enter` emits on the
    /// control directly. Kept because it is the only way an executor body can
    /// be asserted without a Godot runtime, and the test spec in `specs`
    /// exercises it.
    #[allow(
        dead_code,
        reason = "the only caller is the #[cfg(test)] PROBE spec in `specs`"
    )]
    pub(crate) fn emit(&mut self, name: &'static str, arg: Option<i64>) {
        if let Some(rec) = self.recorder.as_deref_mut() {
            rec.push(Effect::EmitSignal { name, arg });
            return;
        }
        let Some(target) = self.target.as_mut() else {
            return;
        };
        match arg {
            Some(v) => target.emit_signal(name, &[Variant::from(v as i32)]),
            None => target.emit_signal(name, &[]),
        };
    }
}

/// Every action the shell knows about, by id.
///
/// Registration is driven by `const` provider tables rather than link-time
/// magic: a `cdylib` that Godot dlopens and hot-reloads under `lto = "fat"` is
/// the wrong place for life-before-main constructors, and a plain array is
/// compile-time checked and reviewable in a diff.
#[derive(Debug, Default)]
pub(crate) struct ActionRegistry {
    names: ActionNames,
    specs: Vec<&'static ActionSpec>,
}

impl ActionRegistry {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Register a spec, returning its id. Idempotent by name.
    pub(crate) fn register(&mut self, spec: &'static ActionSpec) -> ActionId {
        let id = self.names.intern(spec.id);
        let idx = id.0 as usize;
        if idx == self.specs.len() {
            self.specs.push(spec);
        } else {
            debug_assert_eq!(
                self.specs[idx].id, spec.id,
                "action id {idx} reused for a different spec"
            );
        }
        id
    }

    pub(crate) fn get(&self, id: ActionId) -> Option<&'static ActionSpec> {
        self.specs.get(id.0 as usize).copied()
    }

    pub(crate) fn id_of(&self, name: &str) -> Option<ActionId> {
        self.names.id_of(name)
    }

    pub(crate) fn name_of(&self, id: ActionId) -> Option<&str> {
        self.names.name_of(id)
    }

    /// Test-only. Kept together with `iter` and `caps_allow` because each is
    /// the assertion a registry test is built on, and each would take its
    /// test's meaning with it.
    #[allow(dead_code, reason = "the registry tests assert against it")]
    pub(crate) fn len(&self) -> usize {
        self.specs.len()
    }

    /// Every registered action. Written for the introspector, which ended up
    /// enumerating the binding index instead; the registry-completeness tests
    /// are the callers today.
    #[allow(
        dead_code,
        reason = "the registry-completeness tests enumerate through it"
    )]
    pub(crate) fn iter(&self) -> impl Iterator<Item = (ActionId, &'static ActionSpec)> + '_ {
        self.specs
            .iter()
            .enumerate()
            .map(|(i, s)| (ActionId(i as u32), *s))
    }

    /// Run an action by id.
    ///
    /// **Deliberately does not consult `requires`.** `Caps` gates *bindings*,
    /// never invocation: the resolver filters candidates during the forest
    /// walk, so by the time an action runs the decision is made. That rule is
    /// what keeps `:action godotvim.fs.refresh` working from the command
    /// line, where there is no keystroke, no surface and no sampled widget to
    /// derive capabilities from. Actions that genuinely need a widget
    /// re-assert it in their own body.
    ///
    /// Call [`Self::caps_allow`] first on the binding path.
    pub(crate) fn run(&self, id: ActionId, cx: &mut ActionCtx<'_>) -> Outcome {
        let Some(spec) = self.get(id) else {
            log::warn!("run: unknown action id {}", id.0);
            return Outcome::Declined;
        };
        (spec.run)(cx)
    }

    /// Whether `caps` satisfies what `id` requires — the binding-plane gate.
    ///
    /// A miss is a **declination**: it is how `h`/`l` go inert on a list with
    /// no hierarchy without the dispatcher naming a widget class.
    ///
    /// The shipped gate is the same comparison inlined in `resolve::hit_from`,
    /// which reaches the spec it already looked up rather than re-resolving
    /// the id. This spelling is what the capability tests assert against.
    #[allow(
        dead_code,
        reason = "resolve::hit_from inlines this comparison; the capability tests use this form"
    )]
    pub(crate) fn caps_allow(&self, id: ActionId, caps: Caps) -> bool {
        let Some(spec) = self.get(id) else {
            return false;
        };
        let ok = caps.satisfies(spec.requires);
        if !ok {
            log::trace!(
                "caps: {} skipped — needs {:?}, target offers {:?}",
                spec.id,
                spec.requires,
                caps
            );
        }
        ok
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── Params ───────────────────────────────────────────────────────

    #[test]
    fn an_absent_parameter_yields_its_default() {
        assert_eq!(Params::new().int("count", 7), 7);
        assert!(Params::new().is_empty());
    }

    #[test]
    fn set_then_read_round_trips() {
        let mut p = Params::new();
        p.set_int("count", 5);
        assert_eq!(p.int("count", 1), 5);
        assert!(!p.is_empty());
    }

    #[test]
    fn setting_the_same_key_twice_replaces_rather_than_appends() {
        let mut p = Params::new();
        p.set_int("count", 3);
        p.set_int("count", 9);
        assert_eq!(p.int("count", 1), 9);
        assert_eq!(p.iter().count(), 1, "last writer wins, no duplicate key");
    }

    #[test]
    fn count_defaults_to_one() {
        assert_eq!(Params::new().count(), 1);
    }

    #[test]
    fn count_is_clamped_at_both_ends() {
        // The upper clamp is the freeze guard: find_navigable_target walks up
        // to 1000 items per call, so an unbounded count hangs the editor.
        let mut p = Params::new();
        p.set_int("count", 10_000);
        assert_eq!(p.count(), MAX_ACTION_COUNT as u32);

        // Zero or negative must mean "once", not "never" and not a panic on
        // the `as u32` cast.
        for v in [0, -1, i64::MIN] {
            let mut p = Params::new();
            p.set_int("count", v);
            assert_eq!(p.count(), 1, "count={v} should clamp to 1");
        }
        let mut p = Params::new();
        p.set_int("count", i64::MAX);
        assert_eq!(p.count(), MAX_ACTION_COUNT as u32);
    }

    #[test]
    fn iter_preserves_insertion_order_for_the_introspector() {
        let mut p = Params::new();
        p.set_int("count", 1);
        p.set_int("depth", 2);
        assert_eq!(
            p.iter().collect::<Vec<_>>(),
            vec![("count", 1), ("depth", 2)]
        );
    }

    // ── Action ids ───────────────────────────────────────────────────

    #[test]
    fn well_formed_ids_are_accepted() {
        for name in [
            "godotvim.focus.left",
            "godotvim.fs.create",
            "godotvim.item.next",
            "a.b",
            "plugin.some_action",
        ] {
            assert!(is_valid_action_id(name), "{name} should be valid");
        }
    }

    #[test]
    fn ids_without_a_dot_are_rejected() {
        // The host bridge splits its namespace from Godot's shortcut paths on
        // the dot, so a bare name would be ambiguous.
        for name in ["", "focus", "godotvim"] {
            assert!(!is_valid_action_id(name), "{name} should be invalid");
        }
    }

    #[test]
    fn ids_containing_a_slash_are_rejected() {
        // `filesystem_dock/delete` is a Godot editor-shortcut path, not an
        // action id; accepting it would make the two namespaces collide.
        assert!(!is_valid_action_id("filesystem_dock/delete"));
        assert!(!is_valid_action_id("godotvim.fs/create"));
    }

    #[test]
    fn ids_with_exotic_characters_are_rejected() {
        for name in ["godotvim.fs create", "godotvim.fs-create", "godotvim.fs!"] {
            assert!(!is_valid_action_id(name), "{name} should be invalid");
        }
    }

    // ── Interning ────────────────────────────────────────────────────

    #[test]
    fn interning_is_idempotent() {
        let mut names = ActionNames::new();
        let a = names.intern("godotvim.focus.left");
        let b = names.intern("godotvim.focus.left");
        assert_eq!(a, b, "the same name must always yield the same id");
    }

    #[test]
    fn distinct_names_get_distinct_ids() {
        let mut names = ActionNames::new();
        let a = names.intern("godotvim.focus.left");
        let b = names.intern("godotvim.focus.right");
        assert_ne!(a, b);
    }

    #[test]
    fn interning_round_trips_both_ways() {
        let mut names = ActionNames::new();
        let id = names.intern("godotvim.fs.create");
        assert_eq!(names.name_of(id), Some("godotvim.fs.create"));
        assert_eq!(names.id_of("godotvim.fs.create"), Some(id));
    }

    #[test]
    fn an_unregistered_name_has_no_id() {
        let names = ActionNames::new();
        assert_eq!(names.id_of("godotvim.nope"), None);
        assert_eq!(names.name_of(ActionId(0)), None);
    }

    #[test]
    fn an_id_becomes_a_pseudo_key_carrying_itself() {
        // This is what lets an action be a mapping right-hand side:
        // `nnoremap <leader>ff <Action>(godotvim.fs.create)`.
        let id = ActionId(42);
        assert_eq!(id.as_key().key(), vim_core::keymap::Key::Action(42));
    }
}
