//! `dock` — any focusable Tree, ItemList or RichTextLabel in the editor.
//!
//! The generic panel surface: `j`/`k` move, `h`/`l` expand and collapse where
//! there is a hierarchy, `<CR>` activates, `/` focuses the filter box, `<Esc>`
//! returns to the script editor. Every more specific dock — `dock.filesystem`
//! today, a debugger dock later — inherits all of it by naming `dock` as its
//! parent, which is the entire cost of a new panel.
//!
//! # Why the predicate is `VNAV`, not a class list
//!
//! `Caps::VNAV` is held by exactly `Tree`, `ItemList` and `RichTextLabel`
//! (`src/actions/caps.rs`), which is exactly the three arms of
//! `handle_navigation` and exactly the three arms of `classify_focus`. Writing
//! the probe against the affordance keeps those three lists from drifting: a
//! class added to `Caps::of_control` becomes a dock here with no edit, and a
//! class that cannot answer `j`/`k` cannot become one by accident.
//!
//! # Why it must stay narrow
//!
//! A focused `Button` inside a dock samples to `unknown`, not to `dock`. That
//! is deliberate and load-bearing: widening this probe would wake the
//! currently-dead `find_best_nav_target` recursion in
//! `src/navigation/dock_nav.rs`, and `j`/`k` would start moving a `Tree` the
//! user is not focused on.

use crate::actions::caps::Caps;
use crate::actions::surface::{Anchor, FocusChain, Seal, SurfaceSpec};

use super::Provider;

/// Whether the focus owner is one of the three widgets a dock surface knows
/// how to drive. Shared with `dock.filesystem`, which must claim the same
/// widgets and only those.
pub(super) fn focuses_nav_widget(chain: &FocusChain) -> bool {
    chain
        .focus()
        .is_some_and(|node| node.widget_caps().contains(Caps::VNAV))
}

pub(crate) static DOCK: SurfaceSpec = SurfaceSpec {
    id: "dock",
    parent: Some("panel"),
    seal: Seal::Open,
    // Nothing. The widget contributes VNAV/HIERARCHY/ACTIVATE itself; a dock
    // adds no affordance merely by being a dock.
    grants: |_| Caps::empty(),
    probe: |chain| focuses_nav_widget(chain).then_some(Anchor::Node(0)),
    overlay: None,
    on_key: None,
    refuses_positional: false,
    yields_to_engine: false,
};

/// Dock item navigation, plus `/` for the filter box.
///
/// Consumption is **elastic** — the default — which is what preserves `j` at
/// the end of a list (`dock.rs`): the action declines, the key is not
/// consumed, and Godot's own handling proceeds.
///
/// `h` and `l` are bound unconditionally and go inert on an `ItemList` through
/// the capability gate rather than through a `DockKind` match arm: their verbs
/// require `Caps::HIERARCHY`, which only a `Tree` contributes. That is the
/// whole replacement for `matches!(dock_kind, DockKind::Tree)`.
///
/// `/` carries `<physical>` for the same reason the rest do, and it is the key
/// whose shadowing bug (`dock.rs`, unreachable behind the hjkl arm on a
/// physical-J layout) motivated the one-probe-list-per-keyset rule.
/// Today's dock keyset, verbatim.
///
/// `dock_action_for` binds seven keys, not five: hjkl and `/` carry the
/// positional probe because their meaning is a QWERTY *position*, while
/// `<CR>` and `<Esc>` do not — a named key never receives one, so flagging
/// them would be inert at best and misleading at worst.
const DEFAULTS: &str = "\
panelmap <physical> dock h godotvim.item.collapse
panelmap <physical> dock j godotvim.item.next
panelmap <physical> dock k godotvim.item.prev
panelmap <physical> dock l godotvim.item.expand
panelmap <physical> dock / godotvim.dock.search
panelmap dock <CR> godotvim.item.activate
panelmap dock <Esc> godotvim.focus.editor
";

pub(crate) const PROVIDER: Provider = Provider {
    tag: "godotvim.dock",
    surfaces: &[&DOCK],
    // Verbs stayed in `specs::SHIPPED` when P2 extracted them; see `Provider`.
    actions: &[],
    defaults: DEFAULTS,
};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::actions::surface::fixtures::*;

    fn focused(node: crate::actions::surface::ChainNode) -> FocusChain {
        FocusChain {
            nodes: vec![
                node,
                plain("VBoxContainer", 99),
                plain("SceneTreeDock", 100),
            ],
            ..Default::default()
        }
    }

    #[test]
    fn all_three_navigable_widgets_are_docks() {
        // Including RichTextLabel: the built-in docs panel and the Output log
        // are both focusable RichTextLabels, and j/k scroll them today.
        for node in [
            tree("Tree", 1),
            item_list("ItemList", 2),
            rich_text("RichTextLabel", 3),
        ] {
            assert_eq!((DOCK.probe)(&focused(node)), Some(Anchor::Node(0)));
        }
    }

    #[test]
    fn subclasses_are_docks_too() {
        // FileSystemTree, FileSystemList, SceneTreeEditor's Tree — the editor
        // is built from these, so an exact class comparison would classify
        // almost nothing.
        for node in [tree("FileSystemTree", 1), item_list("FileSystemList", 2)] {
            assert_eq!((DOCK.probe)(&focused(node)), Some(Anchor::Node(0)));
        }
    }

    #[test]
    fn a_focused_button_inside_a_dock_is_not_a_dock() {
        // The fixture the design calls out by name. Widening the probe to
        // "anything inside a dock" would wake the dead find_best_nav_target
        // recursion and move a Tree the user is not focused on.
        assert_eq!((DOCK.probe)(&focused(plain("Button", 4))), None);
    }

    #[test]
    fn text_inputs_and_graph_edits_are_not_docks() {
        for node in [
            line_edit(5),
            text_edit(6),
            code_edit(7),
            plain("GraphEdit", 8),
        ] {
            assert_eq!((DOCK.probe)(&focused(node)), None);
        }
    }

    #[test]
    fn no_focus_owner_is_not_a_dock() {
        assert_eq!((DOCK.probe)(&no_focus_owner()), None);
    }
}
