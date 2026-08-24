//! `unknown` — the catch-all, and the only surface that may anchor rootless.
//!
//! Everything the other probes did not recognize: a `GraphEdit`, a focused
//! `Button`, a container, a third-party control — and the case with no focus
//! owner at all. Its parent is `panel`, so Ctrl+hjkl still works from all of
//! them — the behaviour the old dispatcher got by classifying anything
//! unrecognized as interceptable. There is deliberately no `graph` surface: a
//! `GraphEdit` lands here and inherits panel navigation, which is the
//! behaviour that already ships.
//!
//! # The total probe
//!
//! This is the only surface whose probe returns `Some` unconditionally, and
//! that fact fixes the array order: it must be the **last probing entry**, or
//! every surface behind it is unreachable. `panel` may follow it only because
//! `panel` never probes at all.
//!
//! # Why `Rootless` exists
//!
//! `viewport.gui_get_focus_owner()` returning `None` is a real, mandatory
//! state — click the editor's empty background and it happens. The old
//! dispatcher intercepted Ctrl+hjkl there, found no focus owner, skipped the
//! window-nav walk entirely and called `set_input_as_handled()` anyway; the
//! `<void>` consumption on `panel`'s four rules is what preserves that. With
//! an empty `nodes` vector there is no chain index to anchor at, so
//! `Option<usize>` could not express it and the case would silently become
//! "no surface", i.e. a key given back to Godot.

use crate::actions::caps::Caps;
use crate::actions::surface::{Anchor, Seal, SurfaceSpec};

use super::Provider;

pub(crate) static UNKNOWN: SurfaceSpec = SurfaceSpec {
    id: "unknown",
    parent: Some("panel"),
    seal: Seal::Open,
    grants: |_| Caps::empty(),
    probe: |chain| {
        Some(match chain.focus() {
            Some(_) => Anchor::Node(0),
            None => Anchor::Rootless,
        })
    },
    overlay: None,
    on_key: None,
    refuses_positional: false,
    yields_to_engine: false,
};

pub(crate) const PROVIDER: Provider = Provider {
    tag: "godotvim.unknown",
    surfaces: &[&UNKNOWN],
    // Verbs stayed in `specs::SHIPPED` when P2 extracted them; see `Provider`.
    actions: &[],
    // None. Everything reachable from `unknown` is inherited from `panel`,
    // which is what keeps Ctrl+hjkl working with a focused Button, a
    // GraphEdit, or no focus owner at all.
    defaults: "",
};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::actions::surface::fixtures::*;
    use crate::actions::surface::FocusChain;
    use vim_core::primitives::Mode;

    #[test]
    fn the_probe_is_total() {
        // Every chain shape in the plane, including the ones other surfaces
        // claim first. Totality is what makes classification a total function;
        // exclusivity comes from array order, not from this probe missing.
        let chains = [
            no_focus_owner(),
            FocusChain {
                nodes: vec![plain("GraphEdit", 1)],
                ..Default::default()
            },
            FocusChain {
                nodes: vec![tree("FileSystemTree", 2)],
                in_filesystem_dock: true,
                ..Default::default()
            },
            FocusChain {
                nodes: vec![code_edit(3)],
                attached_editor: Some(id(3)),
                editor_mode: Some(Mode::Insert),
                ..Default::default()
            },
            FocusChain {
                nodes: vec![line_edit(4)],
                sibling_nav_control: Some(id(5)),
                ..Default::default()
            },
        ];
        for chain in &chains {
            assert!((UNKNOWN.probe)(chain).is_some(), "{chain:?}");
        }
    }

    #[test]
    fn no_focus_owner_anchors_rootless() {
        assert_eq!((UNKNOWN.probe)(&no_focus_owner()), Some(Anchor::Rootless));
    }

    #[test]
    fn a_focus_owner_anchors_at_node_zero() {
        let chain = FocusChain {
            nodes: vec![plain("GraphEdit", 1)],
            ..Default::default()
        };
        assert_eq!((UNKNOWN.probe)(&chain), Some(Anchor::Node(0)));
    }

    #[test]
    fn the_catch_all_still_reaches_panel() {
        // If this parent link were dropped, Ctrl+hjkl would stop working from
        // a GraphEdit and from an empty focus — two cases that work today.
        assert_eq!(UNKNOWN.parent, Some("panel"));
        assert_eq!(UNKNOWN.seal, Seal::Open);
    }
}
