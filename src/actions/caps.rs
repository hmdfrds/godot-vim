//! What a focused control can *do*, as opposed to what class it is.
//!
//! The dispatcher used to gate behaviour on widget identity — `h`/`l` worked
//! only when `matches!(dock_kind, DockKind::Tree)`. That reads fine for three
//! widget kinds and becomes a maintenance tax at ten: every new dock means new
//! match arms in code that has no business knowing about docks.
//!
//! Capabilities invert it. A control contributes the set of affordances it
//! offers; an action declares what it needs; the resolver skips candidates
//! whose needs are not met. Adding a dock means declaring its capabilities,
//! and nothing in the dispatcher changes.
//!
//! # Why the vocabulary is what it is
//!
//! The bits are chosen so that today's behaviour is expressible *exactly*.
//! [`VNAV`](Caps::VNAV) is the sharp case: `j`/`k` currently work on `Tree`,
//! `ItemList` **and** `RichTextLabel` — `handle_navigation` has no widget gate
//! at all, and on a `RichTextLabel` it scrolls 50px and reports `Handled`
//! (`src/navigation/dock_nav.rs`). A capability named "has a selectable list"
//! would silently drop the built-in docs panel and the Output log, both of
//! which are focusable `RichTextLabel`s. So the bit is named for the
//! affordance — *can move a vertical cursor* — not for the data structure.

use bitflags::bitflags;

bitflags! {
    /// Affordances a focused control offers, or an action requires.
    ///
    /// A control's set is contributed by its class; an action's set is
    /// declared on its `ActionSpec`. See [`Caps::satisfies`] for the gate.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
    pub(crate) struct Caps: u8 {
        /// Move a vertical cursor: next/previous item, or scroll.
        ///
        /// Held by `Tree`, `ItemList` **and** `RichTextLabel` — see the module
        /// docs for why this is not "has a list".
        const VNAV = 1 << 0;
        /// Expand and collapse nested items. `Tree` only.
        const HIERARCHY = 1 << 1;
        /// Open or activate the current item (`Enter`).
        const ACTIVATE = 1 << 2;
        /// Accept typed text — a filter box or a prompt. Bare keys must reach
        /// the control rather than being claimed as commands.
        const TEXTENTRY = 1 << 3;
        /// Create / delete / rename / yank paths. The FileSystem dock.
        const FILEOPS = 1 << 4;
        /// A completion popup is on screen at the moment this key arrived.
        ///
        /// Contributed ONLY by an overlay's `grants`; `Caps::of_control` can
        /// never produce it, which is what makes it sound to gate
        /// `Consumption::Handoff` on. The one bit whose lifetime is a
        /// keystroke rather than a focus.
        const POPUP = 1 << 5;
    }
}

impl Caps {
    /// Capabilities that name a state in which the focused control's OWN key
    /// handling is what the user asked for. Audit A9' is defined against it.
    #[allow(
        dead_code,
        reason = "the A9' audit test is the consumer; it is a test by design, \
                  like the golden fixture table it reads"
    )]
    pub(crate) const HANDOFF_SAFE: Self = Self::POPUP;

    /// Whether a control offering `self` can host an action requiring `needs`.
    ///
    /// Subset test: every required bit must be present. An action requiring
    /// nothing runs anywhere, which is what makes cross-panel focus movement
    /// work with no focus owner at all.
    pub(crate) const fn satisfies(self, needs: Self) -> bool {
        self.contains(needs)
    }

    /// Capabilities of a Godot control, by class name.
    ///
    /// String-based because Godot's hierarchy is only known at runtime, and
    /// walking it is what catches subclasses — `FileSystemTree` is a `Tree`,
    /// `FileSystemList` is an `ItemList`. Callers pass an `is_class` probe so
    /// this stays testable without a Godot runtime.
    pub(crate) fn of_control(is_class: impl Fn(&str) -> bool) -> Self {
        let mut caps = Self::empty();
        if is_class("Tree") {
            caps |= Self::VNAV | Self::HIERARCHY | Self::ACTIVATE;
        } else if is_class("ItemList") {
            caps |= Self::VNAV | Self::ACTIVATE;
        } else if is_class("RichTextLabel") {
            // Scrolls rather than selecting, but j/k work and are expected to.
            caps |= Self::VNAV;
        }
        if is_class("LineEdit") || is_class("TextEdit") {
            caps |= Self::TEXTENTRY;
        }
        caps
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Stand-in for Godot's `Node::is_class`, which walks the inheritance
    /// chain. `("FileSystemTree", &["Tree"])` models a subclass.
    fn classes(names: &'static [&'static str]) -> impl Fn(&str) -> bool {
        move |q| names.contains(&q)
    }

    #[test]
    fn a_tree_offers_navigation_hierarchy_and_activation() {
        let caps = Caps::of_control(classes(&["Tree"]));
        assert_eq!(caps, Caps::VNAV | Caps::HIERARCHY | Caps::ACTIVATE);
    }

    #[test]
    fn an_item_list_offers_no_hierarchy() {
        // This is what makes `h`/`l` inert on an ItemList — a capability miss,
        // not a `DockKind::Tree` match arm in the dispatcher.
        let caps = Caps::of_control(classes(&["ItemList"]));
        assert_eq!(caps, Caps::VNAV | Caps::ACTIVATE);
        assert!(!caps.satisfies(Caps::HIERARCHY));
    }

    #[test]
    fn a_rich_text_label_still_offers_vertical_navigation() {
        // The regression this vocabulary exists to prevent: j/k scroll the
        // built-in docs panel and the Output log today. A bit meaning "has a
        // selectable list" would have dropped both.
        let caps = Caps::of_control(classes(&["RichTextLabel"]));
        assert!(caps.satisfies(Caps::VNAV));
        assert!(!caps.satisfies(Caps::ACTIVATE));
        assert!(!caps.satisfies(Caps::HIERARCHY));
    }

    #[test]
    fn all_three_dock_widgets_share_vnav() {
        // `handle_navigation` has no widget gate; j/k work on all three.
        for names in [
            &["Tree"] as &[&str],
            &["ItemList"] as &[&str],
            &["RichTextLabel"] as &[&str],
        ] {
            let caps = Caps::of_control(|q| names.contains(&q));
            assert!(caps.satisfies(Caps::VNAV), "{names:?} must keep j/k");
        }
    }

    #[test]
    fn subclasses_inherit_through_the_is_class_probe() {
        // Godot's is_class walks the chain, so FileSystemTree answers "Tree".
        let caps = Caps::of_control(classes(&["FileSystemTree", "Tree", "Control"]));
        assert!(caps.satisfies(Caps::HIERARCHY));
    }

    #[test]
    fn text_inputs_are_text_entry() {
        assert!(Caps::of_control(classes(&["LineEdit"])).satisfies(Caps::TEXTENTRY));
        assert!(Caps::of_control(classes(&["TextEdit"])).satisfies(Caps::TEXTENTRY));
        assert!(!Caps::of_control(classes(&["Tree"])).satisfies(Caps::TEXTENTRY));
    }

    #[test]
    fn an_unknown_control_offers_nothing() {
        assert_eq!(Caps::of_control(classes(&["GraphEdit"])), Caps::empty());
    }

    /// Every class name either table names, plus the subclasses the editor
    /// really produces and a control that is in neither.
    ///
    /// Written out rather than derived, because deriving it from one of the
    /// two tables would make the agreement test read that table twice.
    const SHARED_CLASSES: &[&[&str]] = &[
        &["Tree"],
        &["ItemList"],
        &["RichTextLabel"],
        &["FileSystemTree", "Tree"],
        &["FileSystemList", "ItemList"],
        &["SceneTreeEditor", "Tree"],
        &["EditorLog", "RichTextLabel"],
        &["LineEdit"],
        &["TextEdit"],
        &["CodeEdit", "TextEdit"],
        &["Button"],
        &["GraphEdit"],
    ];

    #[test]
    fn granting_vnav_and_having_a_dock_kind_are_the_same_question() {
        // Two hand-maintained class tables over Tree/ItemList/RichTextLabel,
        // in two modules, with nothing pinning their agreement. A fourth class
        // added to `Caps::of_control` and not to `dock_kind_of` manufactures a
        // SILENT DEAD KEY: the surface grants VNAV, so the resolver admits
        // `godotvim.item.next` and `:panelmap` reports it eligible, and then
        // `handle_navigation` has no signal contract to drive and the body
        // declines. Added to the other table only, the mirror: the widget
        // navigates but no rule can ever reach it.
        for names in SHARED_CLASSES {
            let is_class = |q: &str| names.contains(&q);
            let vnav = Caps::of_control(is_class).satisfies(Caps::VNAV);
            let kind = crate::navigation::dock::dock_kind_for(is_class);
            assert_eq!(
                vnav,
                kind.is_some(),
                "{names:?}: Caps grants VNAV={vnav} but dock_kind_of answers {kind:?}"
            );
        }
    }

    #[test]
    fn the_two_tables_agree_on_which_kind_carries_hierarchy() {
        // The narrower half of the same property. `HIERARCHY` is what makes
        // `h`/`l` live, and it is `Tree` and nothing else on both sides — an
        // `ItemList` that grew HIERARCHY here would send `godotvim.item.expand`
        // into `handle_hierarchy` with no tree to expand.
        use crate::navigation::dock::DockKind;
        for names in SHARED_CLASSES {
            let is_class = |q: &str| names.contains(&q);
            let hierarchy = Caps::of_control(is_class).satisfies(Caps::HIERARCHY);
            let is_tree = crate::navigation::dock::dock_kind_for(is_class) == Some(DockKind::Tree);
            assert_eq!(hierarchy, is_tree, "{names:?}");
        }
    }

    #[test]
    fn no_widget_class_can_ever_offer_a_handoff_safe_capability() {
        // An all-true `is_class` probe yields the union of every bit
        // `of_control` can produce, so this holds for every real control too.
        // It is the soundness argument for gating `Consumption::Handoff` on
        // `Caps::POPUP`: a focused widget alone can never open the gate.
        assert!(Caps::of_control(|_| true)
            .intersection(Caps::HANDOFF_SAFE)
            .is_empty());
    }

    #[test]
    fn popup_is_absent_from_every_shared_class() {
        for names in SHARED_CLASSES {
            let is_class = |q: &str| names.contains(&q);
            assert!(
                !Caps::of_control(is_class).contains(Caps::POPUP),
                "{names:?} must not contribute POPUP"
            );
        }
    }

    #[test]
    fn requiring_nothing_is_satisfied_by_anything() {
        // Cross-panel focus movement requires no capability, which is how it
        // still works when there is no focus owner at all.
        assert!(Caps::empty().satisfies(Caps::empty()));
        assert!(Caps::VNAV.satisfies(Caps::empty()));
    }

    #[test]
    fn satisfies_needs_every_required_bit() {
        let tree = Caps::VNAV | Caps::HIERARCHY | Caps::ACTIVATE;
        assert!(tree.satisfies(Caps::VNAV | Caps::HIERARCHY));
        assert!(!tree.satisfies(Caps::VNAV | Caps::FILEOPS));
    }

    // Compile-time proof that `satisfies` stays const-evaluable, so a provider
    // can assert its own capability invariants without a runtime test.
    const _: () = assert!(Caps::VNAV.union(Caps::ACTIVATE).satisfies(Caps::ACTIVATE));
    const _: () = assert!(!Caps::VNAV.satisfies(Caps::HIERARCHY));
}
