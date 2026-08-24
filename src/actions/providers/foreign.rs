//! `foreign` — somebody else's text input. A total hard stop.
//!
//! A `CodeEdit` we are not attached to, a plain `TextEdit`, or a `LineEdit`
//! with no sibling navigable control: an addon's editor, a resource editor, a
//! Project Settings field. `Seal::Barrier` makes "never intercept here"
//! structural rather than conditional — dispatch returns `Ignore` before any
//! lookup, no ancestor is consulted, and `panel`'s Ctrl+hjkl rules are
//! unreachable. That is the structural form of the old dispatcher's flat
//! refusal to touch a foreign text input, and it is the reason typing Ctrl+H
//! in a Project Settings field still backspaces.
//!
//! # Position in `PROVIDERS` is load-bearing in both directions
//!
//! **Not first.** The predicate below claims a `LineEdit` with no sibling nav
//! control, and whether the plugin's own FileSystem prompt has one is not
//! determinable without a running editor. Ahead of `prompt` it could take the
//! prompt, and `<Esc>` would hit a `Barrier` instead of dismissing it.
//!
//! **Not last.** `unknown` probes unconditionally, so anything behind it is
//! unreachable: `foreign` would never match, a Project Settings `LineEdit`
//! would resolve to `unknown` → `panel`, and Ctrl+hjkl would be consumed
//! mid-word.
//!
//! # Why one arm covers two of the three cases
//!
//! `CodeEdit` derives from `TextEdit`, so "answers `TextEdit` and is not the
//! editor we drive" is exactly "a foreign CodeEdit, or a plain TextEdit". The
//! attachment test is instance identity, which is what makes it a *foreign*
//! CodeEdit rather than a class question — and the attached one is claimed by
//! `editor.nav`/`editor.insert` several probes earlier regardless.

use crate::actions::caps::Caps;
use crate::actions::surface::{Anchor, Seal, SurfaceSpec};

use super::Provider;

pub(crate) static FOREIGN: SurfaceSpec = SurfaceSpec {
    id: "foreign",
    // A root. Naming `panel` as parent would defeat the barrier's purpose.
    parent: None,
    seal: Seal::Barrier,
    grants: |_| Caps::empty(),
    probe: |chain| {
        let node = chain.focus()?;
        // A CodeEdit that is not ours, or a plain TextEdit.
        if node.is("TextEdit") && !chain.attached_editor_focused() {
            return Some(Anchor::Node(0));
        }
        // A LineEdit that is nobody's filter box.
        if node.is("LineEdit") && chain.sibling_nav_control.is_none() {
            return Some(Anchor::Node(0));
        }
        None
    },
    overlay: None,
    on_key: None,
    refuses_positional: false,
    yields_to_engine: false,
};

pub(crate) const PROVIDER: Provider = Provider {
    tag: "godotvim.foreign",
    surfaces: &[&FOREIGN],
    // Verbs stayed in `specs::SHIPPED` when P2 extracted them; see `Provider`.
    actions: &[],
    // A Barrier takes no rules, by validation and not merely by convention:
    // `try_insert` rejects them. Keys here belong to whatever text input has
    // focus, which is the entire point of the surface.
    defaults: "",
};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::actions::surface::fixtures::*;
    use crate::actions::surface::FocusChain;
    use vim_core::primitives::Mode;

    #[test]
    fn a_code_edit_that_is_not_ours_is_foreign() {
        let theirs = FocusChain {
            nodes: vec![code_edit(9), plain("AddonPanel", 10)],
            attached_editor: Some(id(7)),
            editor_mode: Some(Mode::Normal),
            ..Default::default()
        };
        assert_eq!((FOREIGN.probe)(&theirs), Some(Anchor::Node(0)));
    }

    #[test]
    fn our_own_editor_is_never_foreign() {
        // Not merely because `editor.*` probes first: the predicate itself
        // excludes it, so a reordering accident cannot barrier the editor.
        let ours = FocusChain {
            nodes: vec![code_edit(7)],
            attached_editor: Some(id(7)),
            editor_mode: Some(Mode::Insert),
            ..Default::default()
        };
        assert_eq!((FOREIGN.probe)(&ours), None);
    }

    #[test]
    fn a_plain_text_edit_is_foreign() {
        let chain = FocusChain {
            nodes: vec![text_edit(11)],
            ..Default::default()
        };
        assert_eq!((FOREIGN.probe)(&chain), Some(Anchor::Node(0)));
    }

    #[test]
    fn a_line_edit_with_no_sibling_nav_control_is_foreign() {
        // The Project Settings field, and the reason `foreign` must be probed
        // before `unknown`.
        let settings = FocusChain {
            nodes: vec![line_edit(12), plain("EditorSettingsDialog", 13)],
            sibling_nav_control: None,
            ..Default::default()
        };
        assert_eq!((FOREIGN.probe)(&settings), Some(Anchor::Node(0)));
    }

    #[test]
    fn a_dock_filter_box_is_not_foreign() {
        let filter = FocusChain {
            nodes: vec![line_edit(14)],
            sibling_nav_control: Some(id(15)),
            ..Default::default()
        };
        assert_eq!((FOREIGN.probe)(&filter), None);
    }

    #[test]
    fn a_prompt_without_a_sibling_would_be_claimed() {
        // Pinned deliberately: this is precisely why `prompt` is probed first.
        // The predicate cannot be made to miss the prompt, because whether the
        // prompt has a sibling nav control is an editor-runtime fact.
        let prompt = FocusChain {
            nodes: vec![line_edit(16)],
            sibling_nav_control: None,
            is_plugin_prompt: true,
            ..Default::default()
        };
        assert_eq!((FOREIGN.probe)(&prompt), Some(Anchor::Node(0)));
    }

    #[test]
    fn docks_and_unrecognized_controls_are_not_foreign() {
        for node in [
            tree("Tree", 17),
            item_list("ItemList", 18),
            rich_text("RichTextLabel", 19),
            plain("GraphEdit", 20),
            plain("Button", 21),
        ] {
            let chain = FocusChain {
                nodes: vec![node],
                ..Default::default()
            };
            assert_eq!((FOREIGN.probe)(&chain), None);
        }
    }

    #[test]
    fn no_focus_owner_is_not_foreign() {
        // It is `unknown`, which still consumes Ctrl+hjkl. A Barrier here
        // would silently kill cross-panel navigation whenever focus is lost.
        assert_eq!((FOREIGN.probe)(&no_focus_owner()), None);
    }

    #[test]
    fn the_barrier_is_a_root() {
        assert_eq!(FOREIGN.parent, None);
        assert_eq!(FOREIGN.seal, Seal::Barrier);
    }
}
