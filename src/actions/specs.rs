//! The shipped keyset, as named verbs.
//!
//! One `ActionSpec` per thing the plugin can do outside the editor. Naming
//! them is the whole point: inside the editor `<C-w>h` already had a name and
//! was therefore remappable, while the identical action from a dock was an
//! anonymous match arm. Once both address the same spec, one binding table
//! can serve both.
//!
//! Specs are gathered in a plain `const` array rather than registered by
//! link-time magic. A `cdylib` that Godot dlopens and hot-reloads under
//! `lto = "fat"` is the wrong place for life-before-main constructors, and an
//! array is compile-time checked, reviewable in a diff, and costs exactly one
//! line to extend.

use super::action::{ActionCtx, ActionSpec};
use super::caps::Caps;
use super::outcome::Outcome;
use crate::navigation::dock_nav::{HierarchyAction, NavDirection};
use crate::navigation::window::{WindowNavDirection, WindowNavResult};

/// Run a nav executor against the context's target, translating its `bool`
/// into the tri-state. `false` means "nothing to move to" — the end of a
/// list — which is a declination, not a failure: the key falls through and
/// Godot's own handling proceeds.
///
/// The target is reached through [`super::action::PanelOps`] rather than as a
/// `Gd<Control>`, which is what makes the four decisions below — the
/// direction, the `count` repeat, the break on the first refusal, and the
/// polarity of the result — assertable without a running editor.
fn nav(cx: &mut ActionCtx<'_>, direction: NavDirection) -> Outcome {
    let count = cx.params.count();
    let Some(ops) = cx.panel_ops() else {
        return Outcome::Declined;
    };
    let mut moved = false;
    for _ in 0..count {
        if ops.nav_step(direction) {
            moved = true;
        } else {
            // The end of the list. Stopping here rather than retrying is what
            // keeps `10j` at the bottom of a Tree from spinning ten times.
            break;
        }
    }
    if moved {
        Outcome::Handled
    } else {
        Outcome::Declined
    }
}

fn hierarchy(cx: &mut ActionCtx<'_>, action: HierarchyAction) -> Outcome {
    let Some(ops) = cx.panel_ops() else {
        return Outcome::Declined;
    };
    if ops.hierarchy_step(action) {
        Outcome::Handled
    } else {
        Outcome::Declined
    }
}

// ── Item navigation ──────────────────────────────────────────────────────

pub(crate) static ITEM_NEXT: ActionSpec = ActionSpec {
    id: "godotvim.item.next",
    desc: "Move to the next item",
    // VNAV, not "has a list": Tree, ItemList AND RichTextLabel all answer to
    // j/k today, and the docs panel scrolls rather than selecting.
    requires: Caps::VNAV,
    host_invocable: false,
    default_consume: None,
    run: |cx| nav(cx, NavDirection::Next),
};

pub(crate) static ITEM_PREV: ActionSpec = ActionSpec {
    id: "godotvim.item.prev",
    desc: "Move to the previous item",
    requires: Caps::VNAV,
    host_invocable: false,
    default_consume: None,
    run: |cx| nav(cx, NavDirection::Prev),
};

pub(crate) static ITEM_COLLAPSE: ActionSpec = ActionSpec {
    id: "godotvim.item.collapse",
    desc: "Collapse the current item",
    // The capability gate that replaces `matches!(dock_kind, DockKind::Tree)`.
    // An ItemList does not offer HIERARCHY, so this is skipped there without
    // the dispatcher naming a widget class.
    requires: Caps::HIERARCHY,
    host_invocable: false,
    default_consume: None,
    run: |cx| hierarchy(cx, HierarchyAction::Collapse),
};

pub(crate) static ITEM_EXPAND: ActionSpec = ActionSpec {
    id: "godotvim.item.expand",
    desc: "Expand the current item",
    requires: Caps::HIERARCHY,
    host_invocable: false,
    default_consume: None,
    run: |cx| hierarchy(cx, HierarchyAction::Expand),
};

pub(crate) static ITEM_ACTIVATE: ActionSpec = ActionSpec {
    id: "godotvim.item.activate",
    desc: "Open or activate the current item",
    requires: Caps::ACTIVATE,
    host_invocable: false,
    default_consume: None,
    run: |cx| {
        // Delegates rather than re-emitting by hand, and that is not
        // fastidiousness. The two widgets have DIFFERENT signal contracts:
        // `Tree::item_activated` takes no parameters and `item_selected` is
        // not emitted at all, while `ItemList` emits both WITH an index and
        // only when something is selected (godot scene/gui/tree.cpp:7522,7534
        // vs item_list.cpp:2482,2486). Passing an argument to Tree's zero-arg
        // signal is CALL_ERROR_TOO_MANY_ARGUMENTS — the editor's handler
        // silently does not run, so Enter on the Scene tree stops working.
        // A hand-written "equivalent" got exactly that wrong; this cannot.
        let Some(target) = cx.target().cloned() else {
            return Outcome::Declined;
        };
        let Some(kind) = crate::navigation::dock::dock_kind_of(&target) else {
            return Outcome::Declined;
        };
        crate::navigation::dock::handle_enter(&target, kind)
    },
};

// ── Cross-panel focus ────────────────────────────────────────────────────
//
// These require NO capability. That is what lets them still fire when there
// is no focus owner at all — the case the dispatcher must consume for. The
// `cx.target()` guard below is the verbatim transcription of the old
// `input.rs`, where `handle_window_nav` was skipped with no focus
// owner and `set_input_as_handled()` fired anyway; `Consumption::Void` on the
// four `panel` rules supplies the consume that `:132` supplied.

/// Directional cross-panel movement.
///
/// The `Declined` on a miss is not busywork even though every shipped binding
/// for it is `Void`: a user who writes `panelmap panel <M-h> godotvim.focus.left`
/// gets an elastic rule, and then "no panel that way" must leave the chord to
/// Godot rather than swallowing it.
fn focus_dir(cx: &mut ActionCtx<'_>, direction: WindowNavDirection) -> Outcome {
    let Some(ops) = cx.panel_ops() else {
        return Outcome::Declined;
    };
    outcome_of_nav(ops.move_focus(direction))
}

/// The `WindowNavResult` → `Outcome` mapping, on its own.
///
/// Two arms, and inverting them is not cosmetic: `dispose` branches on
/// `outcome.is_consumed()`, so a `Focused` reported as `Declined` hands
/// `<C-h>` back to Godot *after* focus has already moved, and an `Ignored`
/// reported as `FocusChanged` swallows the chord at the edge of the layout
/// where the user most expects it to fall through.
fn outcome_of_nav(result: WindowNavResult) -> Outcome {
    match result {
        WindowNavResult::Focused => Outcome::FocusChanged,
        WindowNavResult::Ignored => Outcome::Declined,
    }
}

/// Cycling reports `FocusChanged` unconditionally, and that is a decision
/// rather than an oversight: `handle_window_nav_action` returns nothing, so
/// there is no miss to observe, and the chord is `Void` on `panel` anyway.
fn focus_cycle(cx: &mut ActionCtx<'_>, action: crate::effects::WindowNavAction) -> Outcome {
    let Some(ops) = cx.panel_ops() else {
        return Outcome::Declined;
    };
    ops.cycle_focus(action);
    Outcome::FocusChanged
}

pub(crate) static FOCUS_LEFT: ActionSpec = ActionSpec {
    id: "godotvim.focus.left",
    desc: "Move focus to the panel on the left",
    requires: Caps::empty(),
    host_invocable: true,
    default_consume: None,
    run: |cx| focus_dir(cx, WindowNavDirection::Left),
};

pub(crate) static FOCUS_RIGHT: ActionSpec = ActionSpec {
    id: "godotvim.focus.right",
    desc: "Move focus to the panel on the right",
    requires: Caps::empty(),
    host_invocable: true,
    default_consume: None,
    run: |cx| focus_dir(cx, WindowNavDirection::Right),
};

pub(crate) static FOCUS_UP: ActionSpec = ActionSpec {
    id: "godotvim.focus.up",
    desc: "Move focus to the panel above",
    requires: Caps::empty(),
    host_invocable: true,
    default_consume: None,
    run: |cx| focus_dir(cx, WindowNavDirection::Up),
};

pub(crate) static FOCUS_DOWN: ActionSpec = ActionSpec {
    id: "godotvim.focus.down",
    desc: "Move focus to the panel below",
    requires: Caps::empty(),
    host_invocable: true,
    default_consume: None,
    run: |cx| focus_dir(cx, WindowNavDirection::Down),
};

pub(crate) static FOCUS_CYCLE_NEXT: ActionSpec = ActionSpec {
    id: "godotvim.focus.cycle_next",
    desc: "Cycle focus to the next panel",
    requires: Caps::empty(),
    host_invocable: true,
    default_consume: None,
    run: |cx| focus_cycle(cx, crate::effects::WindowNavAction::CycleNext),
};

pub(crate) static FOCUS_CYCLE_PREV: ActionSpec = ActionSpec {
    id: "godotvim.focus.cycle_prev",
    desc: "Cycle focus to the previous panel",
    requires: Caps::empty(),
    host_invocable: true,
    default_consume: None,
    run: |cx| focus_cycle(cx, crate::effects::WindowNavAction::CyclePrev),
};

pub(crate) static FOCUS_EDITOR: ActionSpec = ActionSpec {
    id: "godotvim.focus.editor",
    desc: "Return focus to the script editor",
    requires: Caps::empty(),
    host_invocable: true,
    default_consume: None,
    // Needs no target: it locates the script editor itself, and declines when
    // there is none — which is why `Caps::ESCAPE` was deleted as a gate with
    // no possible grantor.
    run: |_cx| crate::navigation::dock::handle_escape_from_dock(),
};

// ── Dock filter box ──────────────────────────────────────────────────────

pub(crate) static DOCK_SEARCH: ActionSpec = ActionSpec {
    id: "godotvim.dock.search",
    desc: "Focus the dock's filter box",
    // No capability: the depth-20 sibling DFS runs once per `/` press and
    // declines when it finds nothing, which is a better gate than any bit —
    // it asks the actual scene tree rather than the widget class.
    requires: Caps::empty(),
    host_invocable: false,
    default_consume: None,
    run: |cx| {
        let Some(target) = cx.target().cloned() else {
            return Outcome::Declined;
        };
        crate::navigation::dock::handle_slash(&target)
    },
};

pub(crate) static SEARCH_ACCEPT: ActionSpec = ActionSpec {
    id: "godotvim.search.accept",
    desc: "Leave the filter box, keeping the filter",
    requires: Caps::TEXTENTRY,
    host_invocable: false,
    default_consume: None,
    run: |cx| {
        let Some(target) = cx.target().cloned() else {
            return Outcome::Declined;
        };
        crate::navigation::dock::leave_search(&target)
    },
};

// ── FileSystem operations ────────────────────────────────────────────────
//
// `host_invocable` because each can locate its own target, so `:action
// godotvim.fs.create` works from the editor as well as from the dock.
//
// Only `create` needs `&mut FileSystemExplorer` — it owns the prompt — and
// only the key transports lend one (`ActionCtx::with_fs`). The other four are
// free functions over the target, which is what makes `host_invocable: true`
// honest for them rather than aspirational.

/// The focused control and the signal contract it follows.
///
/// `Declined` on a missing `DockKind` reproduces the old dispatch
/// precondition exactly: the classifier that preceded the surface forest
/// could only produce its dock answer for a Tree, ItemList or RichTextLabel,
/// so the dock handler was unreachable for anything else.
fn dock_target(
    cx: &mut ActionCtx<'_>,
) -> Option<(
    godot::prelude::Gd<godot::classes::Control>,
    crate::navigation::dock::DockKind,
)> {
    let target = cx.target().cloned()?;
    let kind = crate::navigation::dock::dock_kind_of(&target)?;
    Some((target, kind))
}

pub(crate) static FS_CREATE: ActionSpec = ActionSpec {
    id: "godotvim.fs.create",
    desc: "Create a file or folder",
    requires: Caps::FILEOPS,
    host_invocable: true,
    default_consume: None,
    run: |cx| {
        let Some((target, kind)) = dock_target(cx) else {
            return Outcome::Declined;
        };
        let Some(fs) = cx.fs() else {
            // The host transport lends no explorer. Decline loudly rather
            // than half-running: `begin_create` without the prompt would
            // report success and show nothing.
            log::warn!("godotvim.fs.create: no FileSystem explorer on this transport");
            return Outcome::Declined;
        };
        fs.begin_create(&target, kind)
    },
};

pub(crate) static FS_DELETE: ActionSpec = ActionSpec {
    id: "godotvim.fs.delete",
    desc: "Delete the selected path",
    requires: Caps::FILEOPS,
    host_invocable: true,
    default_consume: None,
    run: |_cx| crate::navigation::filesystem_explorer::delete_selected(),
};

pub(crate) static FS_RENAME: ActionSpec = ActionSpec {
    id: "godotvim.fs.rename",
    desc: "Rename the selected path",
    requires: Caps::FILEOPS,
    host_invocable: true,
    default_consume: None,
    run: |_cx| crate::navigation::filesystem_explorer::rename_selected(),
};

pub(crate) static FS_YANK_PATH: ActionSpec = ActionSpec {
    id: "godotvim.fs.yank_path",
    desc: "Copy the selected path to the clipboard",
    requires: Caps::FILEOPS,
    host_invocable: true,
    default_consume: None,
    run: |cx| {
        let Some((target, kind)) = dock_target(cx) else {
            return Outcome::Declined;
        };
        crate::navigation::filesystem_explorer::yank_selected_path(&target, kind)
    },
};

pub(crate) static FS_REFRESH: ActionSpec = ActionSpec {
    id: "godotvim.fs.refresh",
    desc: "Rescan the filesystem",
    requires: Caps::FILEOPS,
    host_invocable: true,
    default_consume: None,
    run: |_cx| crate::navigation::filesystem_explorer::scan_filesystem(),
};

/// Every shipped action.
///
/// Adding one here is the entire cost of adding a verb — no dispatcher edit,
/// no match arm, no widget taxonomy.
///
/// This array is **not** the only registration point, and deliberately so:
/// `Provider::actions` is the other, and it is the one a new subsystem uses.
/// Build the registry through [`registry`] rather than looping this directly,
/// or a provider's verbs go missing and its shipped defaults fail to load.
pub(crate) const SHIPPED: &[&ActionSpec] = &[
    &ITEM_NEXT,
    &ITEM_PREV,
    &ITEM_COLLAPSE,
    &ITEM_EXPAND,
    &ITEM_ACTIVATE,
    &FOCUS_LEFT,
    &FOCUS_RIGHT,
    &FOCUS_UP,
    &FOCUS_DOWN,
    &FOCUS_CYCLE_NEXT,
    &FOCUS_CYCLE_PREV,
    &FOCUS_EDITOR,
    &DOCK_SEARCH,
    &SEARCH_ACCEPT,
    &FS_CREATE,
    &FS_DELETE,
    &FS_RENAME,
    &FS_YANK_PATH,
    &FS_REFRESH,
];

/// The whole registry: the core keyset above, then every provider's own verbs.
///
/// The single seam. Before it existed the registry was assembled by five
/// separate `for spec in SHIPPED` loops — one in `GodotVimCore::init` and four
/// in test modules — and none of them consulted `PROVIDERS`, so a new provider
/// that shipped a verb had to edit all five. That is exactly the per-subsystem
/// cost §7.1 claims not to charge, which is why it is one function now.
///
/// Order is `SHIPPED` first, then `PROVIDERS` order. Ids are minted by an
/// append-only interner, so that order is what makes `ActionId`s stable across
/// a rebuild — the introspector's golden snapshots depend on it.
pub(crate) fn registry() -> super::action::ActionRegistry {
    let mut r = super::action::ActionRegistry::new();
    for spec in SHIPPED {
        r.register(spec);
    }
    for spec in super::providers::actions() {
        r.register(spec);
    }
    r
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::actions::action::{
        is_valid_action_id, ActionCtx, ActionRegistry, PanelOps, Params, MAX_ACTION_COUNT,
    };

    // ── The executor bodies, behind the port ─────────────────────────
    //
    // Replicating `providers::completion`'s `CompletionOps` + `FakePopup`. The
    // reason is the same one stated there: the only real `PanelOps` is a
    // `Gd<Control>`, which cannot be constructed in a `cdylib` under
    // `cargo test`, so before this fake existed every body below short-circuited
    // at its `let Some(target) … else { return Declined }` and the tests could
    // only observe a `Declined` produced for three indistinguishable reasons.
    // Direction, repeat count, break-on-refusal and outcome polarity were all
    // freely mutable under a green suite.

    /// A focused control with no Godot in it.
    ///
    /// Records the LOG rather than a final state, because the interesting
    /// failures are "moved twice when asked once" and "moved, then moved back"
    /// — neither of which a final position can show.
    #[derive(Debug, Default)]
    struct FakePanel {
        /// How many steps the widget has left before it refuses. Refusal is
        /// the end of a list, which is a declination and not a failure.
        steps_available: u32,
        /// Whether expand/collapse finds something to do.
        hierarchy_ok: bool,
        /// Whether the cross-panel walk finds a panel that way.
        focus_found: bool,
        log: Vec<String>,
    }

    impl FakePanel {
        fn with_steps(steps_available: u32) -> Self {
            Self {
                steps_available,
                ..Self::default()
            }
        }

        fn hierarchy(hierarchy_ok: bool) -> Self {
            Self {
                hierarchy_ok,
                ..Self::default()
            }
        }

        fn focus(focus_found: bool) -> Self {
            Self {
                focus_found,
                ..Self::default()
            }
        }
    }

    impl PanelOps for FakePanel {
        fn nav_step(&mut self, direction: NavDirection) -> bool {
            self.log.push(format!("nav({direction:?})"));
            if self.steps_available == 0 {
                return false;
            }
            self.steps_available -= 1;
            true
        }

        fn hierarchy_step(&mut self, action: HierarchyAction) -> bool {
            self.log.push(format!("hierarchy({action:?})"));
            self.hierarchy_ok
        }

        fn move_focus(&mut self, direction: WindowNavDirection) -> WindowNavResult {
            self.log.push(format!("focus({direction:?})"));
            if self.focus_found {
                WindowNavResult::Focused
            } else {
                WindowNavResult::Ignored
            }
        }

        fn cycle_focus(&mut self, action: crate::effects::WindowNavAction) {
            self.log.push(format!("cycle({action:?})"));
        }
    }

    /// Run one shipped verb against one fake widget.
    fn run_on(spec: &ActionSpec, panel: &mut FakePanel, params: Params) -> Outcome {
        let mut cx = ActionCtx::new(None, params).with_panel_ops(panel);
        (spec.run)(&mut cx)
    }

    fn with_count(count: i64) -> Params {
        let mut params = Params::new();
        params.set_int("count", count);
        params
    }

    #[test]
    fn item_next_and_item_prev_ask_for_opposite_directions() {
        // The direction is the whole verb. `ITEM_NEXT` walking `Prev` is `j`
        // moving up, and nothing else in the tree can tell.
        for (spec, want) in [(&ITEM_NEXT, "nav(Next)"), (&ITEM_PREV, "nav(Prev)")] {
            let mut panel = FakePanel::with_steps(1);
            assert_eq!(run_on(spec, &mut panel, Params::new()), Outcome::Handled);
            assert_eq!(panel.log, vec![want], "{}", spec.id);
        }
    }

    #[test]
    fn a_count_repeats_the_step_exactly_that_many_times() {
        // `count=` is plumbed from the binding through `Params` and into this
        // loop, and this is the only place the plumbing is observable: with
        // `for _ in 0..count` collapsed to `0..1` every other test still passes.
        let mut panel = FakePanel::with_steps(9);
        assert_eq!(
            run_on(&ITEM_NEXT, &mut panel, with_count(3)),
            Outcome::Handled
        );
        assert_eq!(panel.log.len(), 3);

        // …and the clamp that keeps `panelmap dock 9999j` from freezing the
        // editor is on the same path. The expected count is the literal 100
        // rather than `MAX_ACTION_COUNT`, so the assertion cannot move with the
        // constant it is supposed to pin.
        assert_eq!(MAX_ACTION_COUNT, 100, "the repeat ceiling");
        let mut panel = FakePanel::with_steps(10_000);
        assert_eq!(
            run_on(&ITEM_NEXT, &mut panel, with_count(9_999)),
            Outcome::Handled
        );
        assert_eq!(panel.log.len(), 100);
    }

    #[test]
    fn a_repeat_stops_at_the_first_refusal_instead_of_grinding() {
        // The `break`. Two steps are available and five are asked for: the
        // widget must be asked three times — two moves and the refusal that
        // ends the loop — not five.
        let mut panel = FakePanel::with_steps(2);
        assert_eq!(
            run_on(&ITEM_NEXT, &mut panel, with_count(5)),
            Outcome::Handled
        );
        assert_eq!(
            panel.log.len(),
            3,
            "two moves plus the one refusal that stopped it: {:?}",
            panel.log
        );
    }

    #[test]
    fn moving_reports_handled_and_moving_nowhere_reports_declined() {
        // The polarity, both ways round. It is not cosmetic: `dispose`
        // branches on `outcome.is_consumed()`, so inverting this decides
        // whether `j` at the bottom of a Tree is handed back to Godot or
        // silently eaten.
        let mut panel = FakePanel::with_steps(1);
        assert_eq!(
            run_on(&ITEM_NEXT, &mut panel, Params::new()),
            Outcome::Handled
        );

        let mut panel = FakePanel::with_steps(0);
        assert_eq!(
            run_on(&ITEM_NEXT, &mut panel, Params::new()),
            Outcome::Declined
        );
        assert_eq!(panel.log, vec!["nav(Next)"], "it must have asked once");
    }

    #[test]
    fn collapse_and_expand_ask_for_opposite_hierarchy_actions() {
        for (spec, want) in [
            (&ITEM_COLLAPSE, "hierarchy(Collapse)"),
            (&ITEM_EXPAND, "hierarchy(Expand)"),
        ] {
            let mut panel = FakePanel::hierarchy(true);
            assert_eq!(run_on(spec, &mut panel, Params::new()), Outcome::Handled);
            assert_eq!(panel.log, vec![want], "{}", spec.id);

            let mut panel = FakePanel::hierarchy(false);
            assert_eq!(
                run_on(spec, &mut panel, Params::new()),
                Outcome::Declined,
                "{} must decline when there is nothing to fold",
                spec.id
            );
        }
    }

    #[test]
    fn hierarchy_deliberately_ignores_the_repeat_count() {
        // `3h` is not "collapse three levels": each collapse also moves the
        // selection, so repeating would walk the tree rather than fold it.
        // Pinned so the asymmetry with `nav` reads as a decision.
        let mut panel = FakePanel::hierarchy(true);
        assert_eq!(
            run_on(&ITEM_COLLAPSE, &mut panel, with_count(3)),
            Outcome::Handled
        );
        assert_eq!(panel.log.len(), 1);
    }

    #[test]
    fn each_focus_verb_asks_for_its_own_direction() {
        for (spec, want) in [
            (&FOCUS_LEFT, "focus(Left)"),
            (&FOCUS_RIGHT, "focus(Right)"),
            (&FOCUS_UP, "focus(Up)"),
            (&FOCUS_DOWN, "focus(Down)"),
        ] {
            let mut panel = FakePanel::focus(true);
            assert_eq!(
                run_on(spec, &mut panel, Params::new()),
                Outcome::FocusChanged,
                "{}",
                spec.id
            );
            assert_eq!(panel.log, vec![want], "{}", spec.id);
        }
    }

    #[test]
    fn the_window_nav_result_mapping_holds_in_both_directions() {
        // Two arms and both are load-bearing. `Focused` → `Declined` hands the
        // chord back to Godot after focus has already moved; `Ignored` →
        // `FocusChanged` swallows it at the edge of the layout, where a user
        // most expects `<C-h>` to fall through.
        assert_eq!(
            outcome_of_nav(WindowNavResult::Focused),
            Outcome::FocusChanged
        );
        assert_eq!(outcome_of_nav(WindowNavResult::Ignored), Outcome::Declined);

        // …and through a real verb, so the mapping cannot be right here and
        // bypassed there.
        let mut panel = FakePanel::focus(false);
        assert_eq!(
            run_on(&FOCUS_LEFT, &mut panel, Params::new()),
            Outcome::Declined
        );
    }

    #[test]
    fn each_cycle_verb_carries_its_own_action_and_always_consumes() {
        use crate::effects::WindowNavAction;
        for (spec, want) in [
            (&FOCUS_CYCLE_NEXT, "cycle(CycleNext)"),
            (&FOCUS_CYCLE_PREV, "cycle(CyclePrev)"),
        ] {
            let mut panel = FakePanel::default();
            assert_eq!(
                run_on(spec, &mut panel, Params::new()),
                Outcome::FocusChanged,
                "{}",
                spec.id
            );
            assert_eq!(panel.log, vec![want], "{}", spec.id);
        }
        // Named rather than inferred: the fake's log is a `Debug` rendering,
        // so a rename of the enum variant would otherwise silently pass.
        assert_eq!(format!("{:?}", WindowNavAction::CycleNext), "CycleNext");
    }

    #[test]
    fn every_shipped_id_is_well_formed() {
        // P3 splits its namespace from Godot's shortcut paths on the dot, so
        // an id without one would be ambiguous.
        for spec in SHIPPED {
            assert!(is_valid_action_id(spec.id), "{} is malformed", spec.id);
            assert!(!spec.desc.is_empty(), "{} has no description", spec.id);
        }
    }

    #[test]
    fn ids_are_unique() {
        let mut seen: Vec<&str> = SHIPPED.iter().map(|s| s.id).collect();
        let total = seen.len();
        seen.sort_unstable();
        seen.dedup();
        assert_eq!(seen.len(), total, "duplicate action id");
    }

    #[test]
    fn registration_is_idempotent_and_round_trips() {
        let mut r = registry();
        let before = r.len();
        for spec in SHIPPED {
            r.register(spec);
        }
        assert_eq!(r.len(), before, "re-registering must not grow the registry");

        for spec in SHIPPED {
            let id = r.id_of(spec.id).expect("registered");
            assert_eq!(r.name_of(id), Some(spec.id));
            assert_eq!(r.get(id).map(|s| s.id), Some(spec.id));
        }
    }

    #[test]
    fn an_id_carries_itself_as_a_pseudo_key() {
        let r = registry();
        let id = r.id_of("godotvim.focus.left").unwrap();
        assert_eq!(id.as_key().key(), vim_core::keymap::Key::Action(id.0));
    }

    #[test]
    fn activation_declines_without_a_target() {
        // It delegates to `handle_enter`, which needs a real control. Pinned
        // here: it declines rather than consuming the key.
        let mut effects = Vec::new();
        let r = registry();
        let id = r.id_of("godotvim.item.activate").unwrap();
        let mut cx = ActionCtx::recording(&mut effects);
        assert_eq!(r.run(id, &mut cx), Outcome::Declined);
        assert!(effects.is_empty());
    }

    // ── The capability gate ──────────────────────────────────────────
    //
    // These assert the DECLARED requirement, not a runtime Outcome. That
    // distinction is the whole point: `ActionCtx::recording` has no Godot
    // target, so an executor declines for want of one — the same value a
    // capability miss produces. A test asserting `Declined` cannot tell the
    // two apart, and mutating `requires` to nonsense would not fail it.
    // Asserting the requirement does fail, which is what makes these guards.

    /// A spec whose body is observable without a Godot target, so the registry
    /// contract can be tested independently of any shipped action.
    static PROBE: ActionSpec = ActionSpec {
        id: "test.probe.ran",
        desc: "records that it ran",
        requires: Caps::FILEOPS,
        host_invocable: true,
        default_consume: None,
        run: |cx| {
            cx.emit("ran", None);
            Outcome::Handled
        },
    };

    #[test]
    fn run_does_not_consult_requires() {
        // The rule the design states three times: Caps gates BINDINGS, never
        // invocation. `:action godotvim.fs.refresh` from the command line has
        // no keystroke, no surface and no sampled widget, so it arrives with
        // no capabilities — and must still reach its body.
        let mut r = ActionRegistry::new();
        let id = r.register(&PROBE);
        let mut effects = Vec::new();
        let mut cx = ActionCtx::recording(&mut effects);
        assert_eq!(r.run(id, &mut cx), Outcome::Handled);
        assert_eq!(effects.len(), 1, "the body must actually have run");
        // ...while the binding path DOES gate, on the same spec.
        assert!(!r.caps_allow(id, Caps::empty()));
        assert!(r.caps_allow(id, Caps::FILEOPS));
    }

    #[test]
    fn hierarchy_actions_require_hierarchy() {
        let r = registry();
        for name in ["godotvim.item.collapse", "godotvim.item.expand"] {
            let id = r.id_of(name).unwrap();
            assert_eq!(r.get(id).unwrap().requires, Caps::HIERARCHY, "{name}");
            let item_list = Caps::VNAV | Caps::ACTIVATE;
            assert!(
                !r.caps_allow(id, item_list),
                "{name} must be inert on a list"
            );
            assert!(
                r.caps_allow(id, item_list | Caps::HIERARCHY),
                "{name} on a tree"
            );
        }
    }

    #[test]
    fn item_navigation_requires_only_vnav() {
        // THE regression the vocabulary exists to prevent: j/k work on
        // RichTextLabel today — the docs panel and the Output log scroll.
        let r = registry();
        for name in ["godotvim.item.next", "godotvim.item.prev"] {
            let id = r.id_of(name).unwrap();
            assert_eq!(r.get(id).unwrap().requires, Caps::VNAV, "{name}");
            for class in ["Tree", "ItemList", "RichTextLabel"] {
                assert!(
                    r.caps_allow(id, Caps::of_control(|c| c == class)),
                    "{name} must work on a {class}"
                );
            }
        }
    }

    #[test]
    fn activation_requires_activation() {
        let r = registry();
        let id = r.id_of("godotvim.item.activate").unwrap();
        assert_eq!(r.get(id).unwrap().requires, Caps::ACTIVATE);
        assert!(!r.caps_allow(id, Caps::of_control(|c| c == "RichTextLabel")));
        assert!(r.caps_allow(id, Caps::of_control(|c| c == "Tree")));
    }

    #[test]
    fn focus_movement_requires_no_capability() {
        let r = registry();
        for name in [
            "godotvim.focus.left",
            "godotvim.focus.right",
            "godotvim.focus.up",
            "godotvim.focus.down",
            "godotvim.focus.cycle_next",
            "godotvim.focus.cycle_prev",
            "godotvim.focus.editor",
        ] {
            let id = r.id_of(name).unwrap();
            assert_eq!(r.get(id).unwrap().requires, Caps::empty(), "{name}");
            assert!(r.caps_allow(id, Caps::empty()), "{name}");
        }
    }

    #[test]
    fn filesystem_actions_require_fileops() {
        let r = registry();
        for name in [
            "godotvim.fs.create",
            "godotvim.fs.delete",
            "godotvim.fs.rename",
            "godotvim.fs.yank_path",
            "godotvim.fs.refresh",
        ] {
            let id = r.id_of(name).unwrap();
            assert_eq!(r.get(id).unwrap().requires, Caps::FILEOPS, "{name}");
            assert!(!r.caps_allow(id, Caps::VNAV), "{name}");
            assert!(r.caps_allow(id, Caps::VNAV | Caps::FILEOPS), "{name}");
        }
    }

    #[test]
    fn an_action_with_no_target_declines() {
        // The no-focus-owner path, which is a real and mandatory state:
        // `Anchor::Rootless` reaches `panel`'s Ctrl+hjkl rules with
        // `cx.target()` at `None`. Every body that needs a control must open
        // with a `let ... else { return Declined }`, because `Handled` means
        // "consume the key AND report success" — which would destroy the
        // keystroke and do nothing.
        //
        // This asserts the GUARD and nothing past it. It used to be the only
        // test that ran these bodies at all, which made it read as coverage it
        // never had: with no `PanelOps` to lend, every one of them returned
        // `Declined` before reading its own direction. What each body then does
        // is asserted against `FakePanel` at the top of this module; this row
        // is the `None` case only.
        //
        // `Consumption::Void` is what still consumes there; the action's
        // honesty and the rule's policy are two separate decisions, and this
        // asserts the first.
        //
        // Excluded: the three targetless FileSystem verbs. They reach
        // `EditorInterface::singleton()` unconditionally, which panics
        // outside a running editor — that is what makes them
        // `host_invocable` and is asserted by shape, in `SHIPPED`, rather
        // than by running them.
        let mut effects = Vec::new();
        let r = registry();
        for name in [
            "godotvim.focus.left",
            "godotvim.focus.right",
            "godotvim.focus.up",
            "godotvim.focus.down",
            "godotvim.focus.cycle_next",
            "godotvim.focus.cycle_prev",
            "godotvim.dock.search",
            "godotvim.search.accept",
            "godotvim.fs.create",
            "godotvim.fs.yank_path",
            "godotvim.item.next",
            "godotvim.item.prev",
            "godotvim.item.collapse",
            "godotvim.item.expand",
            "godotvim.item.activate",
        ] {
            let id = r.id_of(name).unwrap();
            let mut cx = ActionCtx::recording(&mut effects);
            assert_eq!(r.run(id, &mut cx), Outcome::Declined, "{name} must decline");
        }
        assert!(
            effects.is_empty(),
            "a targetless action must not emit anything: {effects:?}"
        );
    }

    #[test]
    fn an_unknown_id_declines_instead_of_panicking() {
        let r = registry();
        let mut effects = Vec::new();
        let mut cx = ActionCtx::recording(&mut effects);
        let bogus = crate::actions::action::ActionId(9999);
        assert_eq!(r.run(bogus, &mut cx), Outcome::Declined);
    }

    /// `host_invocable` for **every** registered verb, spelled out.
    ///
    /// Exhaustive rather than a spot check, and that is the whole point: the
    /// four-name version of this test left ~15 of ~29 specs unpinned, so
    /// flipping `godotvim.focus.editor`'s `host_invocable` to `false` passed
    /// the suite while `:action godotvim.focus.editor` started failing loudly
    /// for a user. It is a hand-written table rather than a derived one for
    /// the same reason `PROVIDERS` is an array: a new verb must arrive with a
    /// *decision*, and a decision nobody wrote down is not one.
    ///
    /// The rule: `true` iff the body can locate its own target. `Caps` says
    /// nothing about it — `godotvim.dock.search` requires no capability and is
    /// still `false`, because it needs a focused control to run the sibling
    /// DFS from.
    const HOST_INVOCABLE: &[(&str, bool)] = &[
        // Item navigation — every one needs a focused control.
        ("godotvim.item.next", false),
        ("godotvim.item.prev", false),
        ("godotvim.item.collapse", false),
        ("godotvim.item.expand", false),
        ("godotvim.item.activate", false),
        // Cross-panel focus — `handle_window_nav` walks from the target, and
        // `focus.editor` locates the script editor with no target at all.
        ("godotvim.focus.left", true),
        ("godotvim.focus.right", true),
        ("godotvim.focus.up", true),
        ("godotvim.focus.down", true),
        ("godotvim.focus.cycle_next", true),
        ("godotvim.focus.cycle_prev", true),
        ("godotvim.focus.editor", true),
        // Filter box — both need the focused control they operate on.
        ("godotvim.dock.search", false),
        ("godotvim.search.accept", false),
        // FileSystem — each reaches `EditorInterface::singleton()` or the
        // dock's own selection, so `:action godotvim.fs.refresh` works from
        // the editor. `create` is the boundary case: it needs a target for
        // the directory but declines rather than half-running without one.
        ("godotvim.fs.create", true),
        ("godotvim.fs.delete", true),
        ("godotvim.fs.rename", true),
        ("godotvim.fs.yank_path", true),
        ("godotvim.fs.refresh", true),
        // Completion — meaningless without the popup, which only the
        // `gui_input` transport lends.
        ("godotvim.completion.trigger", false),
        ("godotvim.completion.next", false),
        ("godotvim.completion.prev", false),
        ("godotvim.completion.confirm", false),
        ("godotvim.completion.dismiss", false),
        ("godotvim.completion.navigate", false),
        // Debugger — meaningless without a focused debugger tree.
        ("godotvim.debugger.frame_next", false),
        ("godotvim.debugger.frame_prev", false),
        ("godotvim.debugger.frame_last", false),
        ("godotvim.debugger.yank_frame", false),
    ];

    #[test]
    fn only_self_locating_actions_are_host_invocable() {
        // `:action godotvim.fs.create` works from anywhere because the FS
        // actions can find their own target. `godotvim.item.next` cannot —
        // it needs a focused control — so a host request must fail loudly
        // rather than decline invisibly.
        let r = registry();
        for (name, want) in HOST_INVOCABLE {
            let id = r
                .id_of(name)
                .unwrap_or_else(|| panic!("{name} is not registered"));
            let spec = r.get(id).unwrap();
            assert_eq!(spec.host_invocable, *want, "{name}");
        }
    }

    #[test]
    fn the_host_invocable_table_covers_every_registered_verb() {
        // The half that makes the table above exhaustive rather than merely
        // long: a verb added to `SHIPPED` or to a `Provider::actions` table
        // without a line up there fails here, so "spot-checked" cannot quietly
        // come back.
        let mut listed: Vec<&str> = HOST_INVOCABLE.iter().map(|(name, _)| *name).collect();
        listed.sort_unstable();
        let before = listed.len();
        listed.dedup();
        assert_eq!(before, listed.len(), "duplicate entry in HOST_INVOCABLE");

        // Read off the REGISTRY rather than off `SHIPPED`, so the union of the
        // two registration points is what the table is held against — a
        // provider verb reaching the registry and not this table is exactly
        // the hole the four-name version had.
        let mut registered: Vec<&str> = registry().iter().map(|(_, spec)| spec.id).collect();
        registered.sort_unstable();
        assert_eq!(listed, registered);
    }

    #[test]
    fn the_registry_enumerates_everything_for_the_introspector() {
        let r = registry();
        let provider_verbs = crate::actions::providers::actions();
        assert_eq!(r.iter().count(), SHIPPED.len() + provider_verbs.len());
        let names: Vec<&str> = r.iter().map(|(_, s)| s.id).collect();
        assert!(names.contains(&"godotvim.fs.refresh"));
        // The union, asserted from the other side: a provider verb that never
        // reaches the registry is a shipped default that cannot load.
        for spec in provider_verbs {
            assert!(names.contains(&spec.id), "{} is not registered", spec.id);
        }
    }

    // ── The host bridge namespace split (P3) ─────────────────────────

    /// Mirrors the split at the `HostRequest::RunAction` arm: dotted and
    /// slash-free is ours, anything else falls through to Godot's shortcuts.
    fn is_registry_name(name: &str) -> bool {
        name.contains('.') && !name.contains('/')
    }

    #[test]
    fn every_shipped_action_is_claimed_by_the_registry_probe() {
        for spec in SHIPPED {
            assert!(is_registry_name(spec.id), "{} would fall through", spec.id);
        }
    }

    #[test]
    fn godot_editor_shortcut_paths_are_never_claimed() {
        // These are Godot's own namespace. Claiming one would shadow a real
        // editor shortcut and silently break it.
        for path in [
            "filesystem_dock/delete",
            "filesystem_dock/rename",
            "scene_tree/rename",
            "editor/save_scene",
            "debugger/step_over",
        ] {
            assert!(!is_registry_name(path), "{path} must reach Godot");
        }
    }

    #[test]
    fn a_bare_name_is_not_claimed_either() {
        // No dot means no namespace. Falling through is the safe direction:
        // Godot answers or nothing happens, rather than us guessing.
        for name in ["save", "quit", "ui_accept"] {
            assert!(!is_registry_name(name), "{name} must not be claimed");
        }
    }

    #[test]
    fn only_self_locating_actions_accept_a_host_invocation() {
        // `host_invocable: false` must FAIL LOUDLY rather than decline
        // invisibly — an action with nothing to act on is a user error worth
        // reporting, not a silent no-op.
        let r = registry();
        for (name, want) in [
            ("godotvim.fs.create", true),
            ("godotvim.fs.refresh", true),
            ("godotvim.focus.left", true),
            ("godotvim.item.next", false),
            ("godotvim.item.activate", false),
            ("godotvim.dock.search", false),
        ] {
            let spec = r.get(r.id_of(name).unwrap()).unwrap();
            assert_eq!(spec.host_invocable, want, "{name}");
        }
    }

    #[test]
    fn host_invocation_ignores_capabilities_entirely() {
        // `:action godotvim.fs.refresh` from the command line arrives with no
        // capabilities at all. If the host path gated on them it would
        // decline everything — which is exactly what host_invocable exists to
        // prevent. Proven against a probe spec so no shipped body is involved.
        let mut r = ActionRegistry::new();
        let id = r.register(&PROBE);
        let mut effects = Vec::new();
        let mut cx = ActionCtx::recording(&mut effects);
        assert_eq!(r.run(id, &mut cx), Outcome::Handled);
        assert!(
            !r.caps_allow(id, Caps::empty()),
            "the BINDING path still gates"
        );
    }
}
