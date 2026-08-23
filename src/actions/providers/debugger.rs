//! `dock.debugger` — the debugger's stack-frame and breakpoint trees.
//!
//! This file is the design's extensibility claim, executed. §7.1 promises that
//! a new shell-side subsystem costs **one file here plus one line in
//! [`PROVIDERS`](super::PROVIDERS)** — no edit to `src/plugin/input.rs`, to
//! `src/actions/resolve.rs`, to `src/actions/caps.rs`, or to any match arm
//! anywhere. Everything below is surface declaration, verb declaration and
//! default bindings written in the same `panelmap` text a user types. Nothing
//! in the dispatcher knows this file exists.
//!
//! # What it claims, and why the probe is narrow
//!
//! Godot's debugger dock is `EditorDebuggerNode`
//! (`editor/debugger/editor_debugger_node.h:47`, `GDCLASS(EditorDebuggerNode,
//! EditorDock)` in 4.8-dev). It holds a `TabContainer` of `ScriptEditorDebugger`
//! tabs, and the Stack Trace tab builds two focusable `Tree`s:
//! `stack_dump` — column title "Stack Frames" — and `breakpoints_tree`
//! (`editor/debugger/script_editor_debugger.cpp:2230` and `:2269`). Both class
//! strings were read from the checked-out engine source rather than guessed: a
//! wrong one produces a probe that silently never matches, and no headless test
//! can catch that.
//!
//! The probe is the conjunction "inside `EditorDebuggerNode` **and** focused on
//! a `Tree`", which is the same shape `dock.filesystem` uses and for the same
//! reason: `index_of_ancestor` alone would claim the tab bar, the thread
//! `OptionButton` and the variable filter `LineEdit`, and `y` would then try to
//! yank a stack frame from a focused button.
//!
//! The two trees are **not** discriminated from one another. They could be —
//! `breakpoints_tree`'s parent is an `HSplitContainer` while `stack_dump`'s is
//! a `VBoxContainer` — but that is editor-internal layout with no stable
//! guarantee across versions, and a probe built on it would fail silently the
//! first time someone reorders a container. So the verbs are named and
//! described for what they actually do: move within the focused debugger tree.
//!
//! # What this surface gets for free
//!
//! Naming `dock` as the parent is the entire cost of inheriting `j`/`k`
//! (`godotvim.item.next`/`prev`), `h`/`l`, `/` (`godotvim.dock.search`), `<CR>`
//! (`godotvim.item.activate` — which is what jumps to a breakpoint's source,
//! since `breakpoints_tree` connects `item_activated`), and `<Esc>`
//! (`godotvim.focus.editor`). Ctrl+hjkl still moves between panels, because
//! `panel` is `dock`'s ancestor and this seal is `Open`. All of it is listed by
//! `:panelmap`, explained by `:panelmap <lhs>`, and rebindable:
//!
//! ```vim
//! panelunmap dock.debugger J
//! panelmap   dock.debugger <C-n> godotvim.debugger.frame_next
//! panelmap   dock.debugger 3K    godotvim.debugger.frame_prev count=3
//! nnoremap   <leader>dy   <Action>(godotvim.debugger.yank_frame)
//! ```
//!
//! # What it deliberately does NOT ship
//!
//! §7.2 sketches `godotvim.debugger.step_over` delegating to Godot's own
//! `debugger/step_over` shortcut (F10, registered at
//! `editor/debugger/debugger_editor_plugin.cpp:49`). That verb is **not** here,
//! and the omission is the honest one. `RuleTarget::Shortcut` parses, registers
//! and lists today, but `GodotVimCore::run_candidate` declines it
//! (`src/plugin/input.rs`): delegating means re-injecting an event into
//! the same `_input` flush that dispatched it, and the injection-cycle audit
//! and per-frame budget that make that safe are a phase of their own. Shipping
//! `panelmap dock.debugger s <Shortcut>(debugger/step_over)` as a default would
//! be a key that consumes nothing and does nothing — the exact silent dead key
//! the whole design exists to prevent. F10/F11/F12 keep working because we
//! never claim them.

use godot::classes::{DisplayServer, Tree};
use godot::prelude::*;

use crate::actions::action::ActionSpec;
use crate::actions::caps::Caps;
use crate::actions::outcome::Outcome;
use crate::actions::surface::{Anchor, Seal, SurfaceSpec};
use crate::navigation::dock_nav::{handle_navigation, NavDirection};

use super::Provider;

/// Godot's debugger dock, and the ancestor the probe looks for.
///
/// Verified against `editor/debugger/editor_debugger_node.h:47-48`. A
/// `ScriptEditorDebugger` tab is always a descendant of one
/// (`editor_debugger_node.cpp:115-140`), so anchoring on the dock rather than
/// on the tab covers every tab this surface might grow into.
const DEBUGGER_DOCK_CLASS: &str = "EditorDebuggerNode";

/// Upper bound on a walk to the end of a debugger tree.
///
/// `handle_navigation` is itself bounded at 1000 items per call
/// (`dock_nav.rs`), so an unbounded outer loop would be a frozen editor
/// rather than a slow keystroke. A call stack deeper than this is pathological
/// and stopping short of the true end is a far better failure than a hang.
const MAX_WALK: u32 = 200;

pub(crate) static DOCK_DEBUGGER: SurfaceSpec = SurfaceSpec {
    id: "dock.debugger",
    parent: Some("dock"),
    seal: Seal::Open,
    // Nothing. `Tree` already contributes VNAV/HIERARCHY/ACTIVATE, and being a
    // debugger adds no affordance — which is the point §7.3 makes about
    // `Caps` being a closed vocabulary: this surface self-restricts in its
    // probe instead of inventing a `STACKFRAME` bit.
    grants: |_| Caps::empty(),
    probe: |chain| {
        (chain.index_of_ancestor(DEBUGGER_DOCK_CLASS).is_some() && chain.focus_is("Tree"))
            .then_some(Anchor::Node(0))
    },
    overlay: None,
    on_key: None,
    refuses_positional: false,
    yields_to_engine: false,
};

/// The debugger tree, as questions and commands.
///
/// The same seam as [`crate::actions::action::CompletionOps`] and
/// [`crate::actions::action::PanelOps`], and here for the same reason: the
/// bodies below used to open with `cx.target().cloned()?`, so under `cargo
/// test` — where no `Gd<Control>` exists — every one of them returned
/// `Declined` before reading its own direction. `FRAME_LAST`'s `Next`, the
/// [`MAX_WALK`] bound and `YANK_FRAME`'s **column 0** were all invisible to the
/// suite. Behind this trait they are a direction, a loop count and an integer,
/// asserted against a plain-data fake.
///
/// Narrow on purpose: four methods, no `Gd<T>` in the signature, and nothing
/// that can reach the editor, the document or the engine.
pub(crate) trait DebuggerTreeOps {
    /// One step, with `handle_navigation`'s `false`-means-nothing-to-move-to
    /// contract preserved.
    fn navigate(&mut self, direction: NavDirection) -> bool;
    /// Whether a row is selected at all — Godot's `Tree::get_selected()`
    /// returning `Some`.
    fn has_selection(&self) -> bool;
    /// The selected row's text in `column`, or empty when there is no
    /// selection. The column is a parameter and not a constant inside the
    /// implementation precisely so the caller's `0` is a decision a test can
    /// see.
    fn selected_text(&self, column: i32) -> String;
    fn clipboard_set(&mut self, text: &str);
}

/// The one shipped implementation.
///
/// The `Tree` cast lives here rather than in the caller so a focused `Button`
/// inside the debugger dock answers "no selection" instead of being
/// unrepresentable — which is the same declination the old `tree_target`
/// produced, reached one layer down.
impl DebuggerTreeOps for Gd<godot::classes::Control> {
    fn navigate(&mut self, direction: NavDirection) -> bool {
        handle_navigation(self, direction, 0)
    }

    fn has_selection(&self) -> bool {
        self.as_tree().and_then(|t| t.get_selected()).is_some()
    }

    fn selected_text(&self, column: i32) -> String {
        self.as_tree()
            .and_then(|t| t.get_selected())
            .map(|item| item.get_text(column).to_string())
            .unwrap_or_default()
    }

    fn clipboard_set(&mut self, text: &str) {
        DisplayServer::singleton().clipboard_set(&GString::from(text));
    }
}

/// The focused control as a `Tree`, or `None`.
trait AsTree {
    fn as_tree(&self) -> Option<Gd<Tree>>;
}

impl AsTree for Gd<godot::classes::Control> {
    fn as_tree(&self) -> Option<Gd<Tree>> {
        self.clone().try_cast::<Tree>().ok()
    }
}

/// Walk the focused tree, translating "nothing to move to" into a declination.
///
/// Declining is how this composes with Godot: at the end of the stack the key
/// falls through to the `Tree`'s own handling instead of dying here. An action
/// that cannot act and consumes anyway is a key sink on its surface.
fn walk(
    cx: &mut crate::actions::action::ActionCtx<'_>,
    direction: NavDirection,
    steps: u32,
) -> Outcome {
    let Some(tree) = cx.debugger_tree() else {
        return Outcome::Declined;
    };
    let mut moved = false;
    for _ in 0..steps {
        if tree.navigate(direction) {
            moved = true;
        } else {
            break;
        }
    }
    if moved {
        Outcome::Handled
    } else {
        Outcome::Declined
    }
}

pub(crate) static FRAME_NEXT: ActionSpec = ActionSpec {
    id: "godotvim.debugger.frame_next",
    desc: "Debugger: select the next stack frame or breakpoint",
    // The affordance, not the widget class — exactly as `godotvim.item.next`
    // does. A future debugger panel that is an ItemList inherits this for free.
    requires: Caps::VNAV,
    // Meaningless without a focused debugger tree, so a `:action` invocation
    // must fail loudly rather than decline invisibly.
    host_invocable: false,
    default_consume: None,
    run: |cx| {
        let steps = cx.params.count();
        walk(cx, NavDirection::Next, steps)
    },
};

pub(crate) static FRAME_PREV: ActionSpec = ActionSpec {
    id: "godotvim.debugger.frame_prev",
    desc: "Debugger: select the previous stack frame or breakpoint",
    requires: Caps::VNAV,
    host_invocable: false,
    default_consume: None,
    run: |cx| {
        let steps = cx.params.count();
        walk(cx, NavDirection::Prev, steps)
    },
};

pub(crate) static FRAME_LAST: ActionSpec = ActionSpec {
    id: "godotvim.debugger.frame_last",
    desc: "Debugger: select the deepest stack frame",
    requires: Caps::VNAV,
    host_invocable: false,
    default_consume: None,
    run: |cx| walk(cx, NavDirection::Next, MAX_WALK),
};

pub(crate) static YANK_FRAME: ActionSpec = ActionSpec {
    id: "godotvim.debugger.yank_frame",
    desc: "Debugger: copy the selected row to the clipboard",
    // VNAV plus a cast, rather than a new capability bit. `Caps` has no "is a
    // Tree" bit and must not grow one: §7.3's rule is that a widget constraint
    // belongs in the probe (which already demands a `Tree`) and in the body,
    // not in the closed vocabulary every other surface has to carry.
    requires: Caps::VNAV,
    host_invocable: false,
    default_consume: None,
    run: |cx| {
        let Some(tree) = cx.debugger_tree() else {
            return Outcome::Declined;
        };
        if !tree.has_selection() {
            return Outcome::Declined;
        }
        // Column 0. The debugger's stack-frame Tree is single-column
        // (`script_editor_debugger.cpp:2230`), and yanking column 1 would copy
        // an empty string over the user's clipboard.
        let text = tree.selected_text(0);
        if text.is_empty() {
            return Outcome::Declined;
        }
        tree.clipboard_set(&text);
        log::info!("debugger: yanked '{text}'");
        Outcome::Handled
    },
};

/// The verbs this provider contributes to the registry.
///
/// They live here rather than in `actions::specs::SHIPPED` on purpose: that is
/// what makes "one file plus one manifest line" true. `specs::SHIPPED` holds
/// the core keyset extracted in P2 and is not a registration point a third
/// party has to touch.
const ACTIONS: &[&ActionSpec] = &[&FRAME_NEXT, &FRAME_PREV, &FRAME_LAST, &YANK_FRAME];

/// Authored in the same text a user types and read by the same parser, so a
/// shipped default cannot drift into a dialect the documented grammar does not
/// describe.
///
/// No `<physical>` anywhere. The FileSystem and dock keysets carry it because
/// their meaning is a QWERTY *position* (hjkl, `/`); `J`/`K`/`G`/`y` here are
/// mnemonic — frame down, frame up, deepest, yank — so a positional probe
/// would be a guess about intent with nothing to gain.
///
/// Single-key throughout. A multi-key LHS such as `gg` is legal on this surface
/// (it is not editor-reachable, so V8 does not apply), but it would reserve `g`
/// on `dock.debugger` and arm P8's pending-prefix timer for a keyset this small
/// — cost with no payoff.
const DEFAULTS: &str = "\
panelmap dock.debugger J godotvim.debugger.frame_next
panelmap dock.debugger K godotvim.debugger.frame_prev
panelmap dock.debugger G godotvim.debugger.frame_last
panelmap dock.debugger y godotvim.debugger.yank_frame
";

pub(crate) const PROVIDER: Provider = Provider {
    tag: "godotvim.debugger",
    surfaces: &[&DOCK_DEBUGGER],
    actions: ACTIONS,
    defaults: DEFAULTS,
};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::actions::action::{ActionCtx, Params};
    use crate::actions::surface::fixtures::*;
    use crate::actions::surface::{ChainNode, FocusChain};

    // ── The executor bodies, behind the port ─────────────────────────

    /// A debugger tree with no Godot in it.
    ///
    /// The same harness shape as `providers::completion`'s `FakePopup`: plain
    /// data, an ordered command log, and no `Gd<T>` anywhere. Without it the
    /// four verbs below were `Declined` before their first decision, so
    /// `FRAME_LAST`'s direction, [`MAX_WALK`] and the yank column were pinned
    /// by nothing.
    #[derive(Debug, Default)]
    struct FakeTree {
        /// Rows left before the walk reaches the end of the stack.
        steps_available: u32,
        selected: bool,
        /// The selected row, column by column. `y` must copy column 0.
        columns: Vec<String>,
        log: Vec<String>,
        clipboard: Option<String>,
    }

    impl FakeTree {
        fn with_steps(steps_available: u32) -> Self {
            Self {
                steps_available,
                ..Self::default()
            }
        }

        fn selecting(columns: &[&str]) -> Self {
            Self {
                selected: true,
                columns: columns.iter().map(|s| (*s).to_string()).collect(),
                ..Self::default()
            }
        }
    }

    impl DebuggerTreeOps for FakeTree {
        fn navigate(&mut self, direction: NavDirection) -> bool {
            self.log.push(format!("navigate({direction:?})"));
            if self.steps_available == 0 {
                return false;
            }
            self.steps_available -= 1;
            true
        }

        fn has_selection(&self) -> bool {
            self.selected
        }

        fn selected_text(&self, column: i32) -> String {
            usize::try_from(column)
                .ok()
                .and_then(|c| self.columns.get(c))
                .cloned()
                .unwrap_or_default()
        }

        fn clipboard_set(&mut self, text: &str) {
            self.log.push(format!("clipboard_set({text})"));
            self.clipboard = Some(text.to_string());
        }
    }

    fn run_on(spec: &ActionSpec, tree: &mut FakeTree, params: Params) -> Outcome {
        let mut cx = ActionCtx::new(None, params).with_debugger_tree(tree);
        (spec.run)(&mut cx)
    }

    fn with_count(count: i64) -> Params {
        let mut params = Params::new();
        params.set_int("count", count);
        params
    }

    #[test]
    fn frame_next_and_frame_prev_walk_in_opposite_directions() {
        for (spec, want) in [
            (&FRAME_NEXT, "navigate(Next)"),
            (&FRAME_PREV, "navigate(Prev)"),
        ] {
            let mut tree = FakeTree::with_steps(1);
            assert_eq!(run_on(spec, &mut tree, Params::new()), Outcome::Handled);
            assert_eq!(tree.log, vec![want], "{}", spec.id);
        }
    }

    #[test]
    fn frame_last_walks_forward_all_the_way_to_the_max_walk_bound() {
        // Both halves are the verb: `G` means "the DEEPEST frame", so the
        // direction is `Next` and the loop count is the bound. Flipping the
        // direction turns `G` into `gg`; lowering the bound turns it into `J`.
        //
        // The expected step count is the LITERAL 200 and not `MAX_WALK`.
        // Writing `MAX_WALK` here would make the assertion a tautology that
        // moves with the constant — which is exactly how `MAX_WALK: 200 -> 1`
        // survived a green suite once already.
        assert_eq!(MAX_WALK, 200, "the depth `G` is willing to walk");
        let mut tree = FakeTree::with_steps(250);
        assert_eq!(
            run_on(&FRAME_LAST, &mut tree, Params::new()),
            Outcome::Handled
        );
        assert_eq!(
            tree.log.len(),
            200,
            "`G` must walk the whole bound, not one step"
        );
        assert!(
            tree.log.iter().all(|l| l == "navigate(Next)"),
            "{:?}",
            tree.log.first()
        );
    }

    #[test]
    fn a_frame_walk_stops_at_the_end_of_the_stack_and_declines_when_it_moved_nothing() {
        // The `break`, and the polarity. A call stack of one frame asked to
        // move four times must be asked twice — one move and the refusal.
        let mut tree = FakeTree::with_steps(1);
        assert_eq!(
            run_on(&FRAME_NEXT, &mut tree, with_count(4)),
            Outcome::Handled
        );
        assert_eq!(tree.log.len(), 2, "{:?}", tree.log);

        // Nowhere to go at all: the key falls through to the `Tree`'s own
        // handling rather than dying here.
        let mut tree = FakeTree::with_steps(0);
        assert_eq!(
            run_on(&FRAME_NEXT, &mut tree, Params::new()),
            Outcome::Declined
        );
        assert_eq!(tree.log.len(), 1);
    }

    #[test]
    fn a_count_repeats_a_frame_walk() {
        let mut tree = FakeTree::with_steps(9);
        assert_eq!(
            run_on(&FRAME_PREV, &mut tree, with_count(3)),
            Outcome::Handled
        );
        assert_eq!(tree.log.len(), 3);
    }

    #[test]
    fn yank_copies_column_zero_and_nothing_else() {
        // The stack-frame Tree is single-column, so column 1 is the empty
        // string — and yanking it would silently wipe the user's clipboard
        // while reporting success.
        let mut tree = FakeTree::selecting(&["res://player.gd:42 @ _process", "(unused)"]);
        assert_eq!(
            run_on(&YANK_FRAME, &mut tree, Params::new()),
            Outcome::Handled
        );
        assert_eq!(
            tree.clipboard.as_deref(),
            Some("res://player.gd:42 @ _process")
        );
    }

    #[test]
    fn yank_declines_without_a_selection_and_without_text() {
        let mut tree = FakeTree::default();
        assert_eq!(
            run_on(&YANK_FRAME, &mut tree, Params::new()),
            Outcome::Declined,
            "nothing selected"
        );
        assert!(tree.clipboard.is_none());

        let mut tree = FakeTree::selecting(&[""]);
        assert_eq!(
            run_on(&YANK_FRAME, &mut tree, Params::new()),
            Outcome::Declined,
            "an empty row"
        );
        assert!(
            tree.clipboard.is_none(),
            "an empty row must not overwrite the clipboard"
        );
    }

    /// The real shape: focus owner, the tab's containers, the tab itself, the
    /// `TabContainer`, then the dock.
    fn in_debugger(node: ChainNode) -> FocusChain {
        FocusChain {
            nodes: vec![
                node,
                plain("VBoxContainer", 70),
                plain("HSplitContainer", 71),
                plain("ScriptEditorDebugger", 72),
                plain("TabContainer", 73),
                plain("EditorDebuggerNode", 74),
            ],
            ..Default::default()
        }
    }

    #[test]
    fn the_stack_and_breakpoint_trees_both_anchor_here() {
        // `stack_dump` and `breakpoints_tree` are both plain `Tree`s
        // (script_editor_debugger.cpp:2230, :2269), so neither carries a
        // distinguishing class name and both must land on this surface.
        for node in [tree("Tree", 1), tree("SomeTreeSubclass", 2)] {
            assert_eq!(
                (DOCK_DEBUGGER.probe)(&in_debugger(node)),
                Some(Anchor::Node(0))
            );
        }
    }

    #[test]
    fn the_dock_ancestor_alone_is_not_enough() {
        // The thread OptionButton, the step buttons and the variable filter box
        // all live inside EditorDebuggerNode. Claiming them would hand `y` to a
        // focused button and `J` to a LineEdit the user is typing in.
        for node in [
            plain("Button", 3),
            plain("OptionButton", 4),
            line_edit(5),
            item_list("ItemList", 6),
            rich_text("RichTextLabel", 7),
        ] {
            assert_eq!((DOCK_DEBUGGER.probe)(&in_debugger(node)), None);
        }
    }

    #[test]
    fn a_tree_outside_the_debugger_is_not_ours() {
        // The Scene tree is a `Tree` too. Without the ancestor test this
        // surface would claim every dock in the editor and steal `y` from
        // `dock.filesystem`'s yank.
        let elsewhere = FocusChain {
            nodes: vec![tree("SceneTreeEditor", 8), plain("SceneTreeDock", 75)],
            ..Default::default()
        };
        assert_eq!((DOCK_DEBUGGER.probe)(&elsewhere), None);
    }

    #[test]
    fn no_focus_owner_is_not_a_debugger_tree() {
        assert_eq!((DOCK_DEBUGGER.probe)(&no_focus_owner()), None);
    }

    #[test]
    fn the_ancestor_class_string_is_the_one_godot_registers() {
        // A typo here is invisible: the probe simply never matches, every
        // binding on this surface is silently dead, and no other test can tell
        // the difference. Pinned against the literal read out of
        // editor/debugger/editor_debugger_node.h:47.
        assert_eq!(DEBUGGER_DOCK_CLASS, "EditorDebuggerNode");
    }

    #[test]
    fn it_is_declared_deeper_than_the_generic_dock() {
        // The whole of "inherits j/k/h/l, /, <CR> and <Esc>", as one link.
        assert_eq!(DOCK_DEBUGGER.parent, Some("dock"));
        assert_eq!(DOCK_DEBUGGER.seal, Seal::Open);
        assert!(!DOCK_DEBUGGER.yields_to_engine);
        assert!(!DOCK_DEBUGGER.refuses_positional);
    }

    #[test]
    fn the_surface_grants_nothing_and_the_widget_supplies_everything() {
        let chain = in_debugger(tree("Tree", 1));
        assert_eq!((DOCK_DEBUGGER.grants)(&chain), Caps::empty());
        assert_eq!(
            chain.widget_caps(),
            Caps::VNAV | Caps::HIERARCHY | Caps::ACTIVATE
        );
    }

    #[test]
    fn every_verb_is_gated_on_an_affordance_and_none_is_host_invocable() {
        // Both halves matter. `requires` is what makes a verb inert rather
        // than mis-firing on a widget that cannot answer it, and
        // `host_invocable: false` is what makes `:action` say "requires panel
        // focus" instead of declining invisibly.
        for spec in ACTIONS {
            assert_eq!(spec.requires, Caps::VNAV, "{}", spec.id);
            assert!(!spec.host_invocable, "{}", spec.id);
            assert!(!spec.desc.is_empty(), "{}", spec.id);
            assert!(
                crate::actions::action::is_valid_action_id(spec.id),
                "{}",
                spec.id
            );
            assert!(
                spec.id.starts_with("godotvim.debugger."),
                "{} escapes this provider's namespace",
                spec.id
            );
        }
    }

    #[test]
    fn every_verb_declines_without_a_target() {
        // `Anchor::Rootless` and the host transport both deliver a
        // target-less context. `Handled` there would consume the key and do
        // nothing, which is indistinguishable from a broken keyboard.
        let mut effects = Vec::new();
        for spec in ACTIONS {
            let mut cx = crate::actions::action::ActionCtx::recording(&mut effects);
            assert_eq!((spec.run)(&mut cx), Outcome::Declined, "{}", spec.id);
        }
        assert!(effects.is_empty(), "{effects:?}");
    }

    #[test]
    fn the_defaults_name_only_verbs_this_provider_declares() {
        // The anti-drift check that does not need the whole plane built: every
        // action id mentioned in the default text must be one of ours, so a
        // rename that misses the string fails here rather than at load.
        for line in DEFAULTS.lines().filter(|l| !l.is_empty()) {
            let id = line.rsplit(' ').next().expect("a target");
            assert!(
                ACTIONS.iter().any(|s| s.id == id),
                "'{id}' is not declared by this provider"
            );
        }
        assert_eq!(DEFAULTS.lines().count(), ACTIONS.len());
    }

    #[test]
    fn the_provider_tag_matches_the_verb_namespace() {
        assert_eq!(PROVIDER.tag, "godotvim.debugger");
        assert_eq!(PROVIDER.surfaces.len(), 1);
        assert_eq!(PROVIDER.actions.len(), 4);
    }
}
