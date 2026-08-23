//! `editor.nav` and `editor.insert` — the attached `CodeEdit`, split by mode.
//!
//! One surface would not do. The same widget must be a *navigable* place where
//! Ctrl+hjkl moves between panels, and an *insert-like* place where Ctrl+H is
//! backspace and Ctrl+J is a newline. The old dispatcher made that split with
//! a mode test inline in its intercept predicate; here it is two surfaces with
//! different seals, and the dispatcher never mentions a mode.
//!
//! # Why `editor.insert` is written as a negation
//!
//! `vim_core::primitives::Mode` is `#[non_exhaustive]`. Two positive
//! enumerations over it cannot be shown total: a future variant would match
//! *neither* editor surface, fall through to `foreign` — a `Barrier` — and
//! leak Ctrl+hjkl into the script editor. Written as a complement within
//! "focus is the attached CodeEdit", totality is a tautology (`A ∧ P` xor
//! `A ∧ ¬P`) rather than a test. The test exists anyway, so that a future edit
//! to either probe fails the suite instead of a user's Ctrl+H.

use vim_core::primitives::Mode;

use crate::actions::caps::Caps;
use crate::actions::surface::{Anchor, Seal, SurfaceSpec};

use super::Provider;

/// The modes in which panel navigation wins over the engine.
///
/// Verbatim from `src/plugin/input.rs`. `Select` is deliberately
/// **excluded**: it is insert-like, so Ctrl+H/J/K/L must reach the engine.
const fn is_nav_mode(mode: Mode) -> bool {
    matches!(
        mode,
        Mode::Normal | Mode::Visual(_) | Mode::OperatorPending(_)
    )
}

pub(crate) static EDITOR_NAV: SurfaceSpec = SurfaceSpec {
    id: "editor.nav",
    parent: Some("panel"),
    seal: Seal::Open,
    grants: |_| Caps::empty(),
    // `editor_mode == None` maps HERE, not to the barrier: the `is_none_or` at
    // input.rs makes "no controller attached" mean INTERCEPT.
    probe: |chain| {
        (chain.attached_editor_focused() && chain.editor_mode.is_none_or(is_nav_mode))
            .then_some(Anchor::Node(0))
    },
    overlay: None,
    on_key: None,
    // The one surface that declares it, and it carries ZERO rules of its own:
    // the `<C-h>` binding lives on `panel`, which is this surface's declared
    // parent. That is what makes the editor/panel duplication gap
    // unrepresentable rather than assertion-checked.
    yields_to_engine: true,
    // The other half of "the editor already has a meaning for every chord".
    // `panel`'s four rules carry `<physical>`, and this surface's declared
    // parent IS `panel`, so without this flag a Dvorak `Ctrl+d` would reach
    // `panel <C-h>` by position and become panel-left instead of half-page
    // down. This is the surface-plane form of the typed-probes-only lookup
    // the old dispatcher performed inside the editor.
    refuses_positional: true,
};

pub(crate) static EDITOR_INSERT: SurfaceSpec = SurfaceSpec {
    id: "editor.insert",
    // A root, not a child of `panel`. Reaching `panel` from here is exactly
    // what must not happen — its Ctrl+hjkl rules would consume backspace.
    parent: None,
    seal: Seal::Barrier,
    grants: |_| Caps::empty(),
    probe: |chain| {
        (chain.attached_editor_focused() && chain.editor_mode.is_some_and(|m| !is_nav_mode(m)))
            .then_some(Anchor::Node(0))
    },
    overlay: None,
    on_key: None,
    yields_to_engine: false,
    // Moot — a Barrier resolves nothing — but stated rather than inherited.
    refuses_positional: true,
};

pub(crate) const PROVIDER: Provider = Provider {
    tag: "godotvim.editor",
    surfaces: &[&EDITOR_NAV, &EDITOR_INSERT],
    // Verbs stayed in `specs::SHIPPED` when P2 extracted them; see `Provider`.
    actions: &[],
    // Deliberately none, and this is the design's sharpest structural claim:
    // `editor.nav` carries ZERO rules of its own, because `<C-h>` lives on
    // `panel`, its declared parent. Duplicating the panel keyset here is what
    // the surface forest makes unrepresentable rather than merely discouraged.
    // `editor.insert` is a Barrier and cannot take a rule at all.
    defaults: "",
};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::actions::surface::fixtures::*;
    use crate::actions::surface::FocusChain;
    use vim_core::primitives::{Operator, VisualType};

    const ATTACHED: i64 = 7;

    fn editor(mode: Option<Mode>) -> FocusChain {
        FocusChain {
            nodes: vec![code_edit(ATTACHED), plain("CodeTextEditor", 8)],
            attached_editor: Some(id(ATTACHED)),
            editor_mode: mode,
            ..Default::default()
        }
    }

    /// Every `Mode` variant this vim-core version has, with one payload per
    /// payload-carrying variant. Pinned to the version: a new variant does not
    /// appear here automatically, which is exactly why the probes are written
    /// as complements rather than trusting this list to be complete.
    fn every_mode() -> Vec<Mode> {
        let mut modes = vec![
            Mode::Normal,
            Mode::Insert,
            Mode::Replace,
            Mode::VirtualReplace,
            Mode::CommandLine,
        ];
        for vt in [VisualType::Char, VisualType::Line, VisualType::Block] {
            modes.push(Mode::Visual(vt));
            modes.push(Mode::Select(vt));
        }
        for op in [Operator::Delete, Operator::Change, Operator::Yank] {
            modes.push(Mode::OperatorPending(op));
        }
        modes
    }

    #[test]
    fn navigation_modes_anchor_at_editor_nav() {
        for mode in [
            Mode::Normal,
            Mode::Visual(VisualType::Char),
            Mode::Visual(VisualType::Line),
            Mode::Visual(VisualType::Block),
            Mode::OperatorPending(Operator::Delete),
        ] {
            assert_eq!(
                (EDITOR_NAV.probe)(&editor(Some(mode))),
                Some(Anchor::Node(0)),
                "{mode:?}"
            );
            assert_eq!((EDITOR_INSERT.probe)(&editor(Some(mode))), None, "{mode:?}");
        }
    }

    #[test]
    fn insert_like_modes_anchor_at_the_barrier() {
        for mode in [
            Mode::Insert,
            Mode::Replace,
            Mode::VirtualReplace,
            Mode::CommandLine,
            Mode::Select(VisualType::Char),
        ] {
            assert_eq!(
                (EDITOR_INSERT.probe)(&editor(Some(mode))),
                Some(Anchor::Node(0)),
                "{mode:?}"
            );
            assert_eq!((EDITOR_NAV.probe)(&editor(Some(mode))), None, "{mode:?}");
        }
    }

    #[test]
    fn select_mode_is_insert_like() {
        // Called out on its own because it is the one that reads wrong: Select
        // is a *visual* mode in Vim's taxonomy but typing replaces the
        // selection, so Ctrl+H must reach the engine. input.rs says so
        // in a comment; here it is a test.
        for vt in [VisualType::Char, VisualType::Line, VisualType::Block] {
            assert_eq!(
                (EDITOR_INSERT.probe)(&editor(Some(Mode::Select(vt)))),
                Some(Anchor::Node(0))
            );
        }
    }

    #[test]
    fn no_controller_means_intercept() {
        // The `is_none_or` polarity at input.rs. A CodeEdit we are
        // attached to but have no controller for is still navigable, or
        // Ctrl+hjkl would break during attach.
        assert_eq!((EDITOR_NAV.probe)(&editor(None)), Some(Anchor::Node(0)));
        assert_eq!((EDITOR_INSERT.probe)(&editor(None)), None);
    }

    #[test]
    fn exactly_one_editor_surface_claims_any_mode() {
        // The partition tautology. `editor.insert` is the complement of
        // `editor.nav`, so this cannot fail without an edit to one of the two
        // probes — which is the point of asserting it.
        for mode in every_mode().into_iter().map(Some).chain([None]) {
            let chain = editor(mode);
            let nav = (EDITOR_NAV.probe)(&chain).is_some();
            let ins = (EDITOR_INSERT.probe)(&chain).is_some();
            assert!(nav ^ ins, "mode {mode:?} claimed by nav={nav} insert={ins}");
        }
    }

    #[test]
    fn a_code_edit_that_is_not_ours_is_neither() {
        // It falls through to `foreign`. Attachment is instance identity,
        // not class identity.
        let theirs = FocusChain {
            nodes: vec![code_edit(9)],
            attached_editor: Some(id(ATTACHED)),
            editor_mode: Some(Mode::Normal),
            ..Default::default()
        };
        assert_eq!((EDITOR_NAV.probe)(&theirs), None);
        assert_eq!((EDITOR_INSERT.probe)(&theirs), None);
    }

    #[test]
    fn with_nothing_attached_no_editor_surface_claims() {
        let orphan = FocusChain {
            nodes: vec![code_edit(9)],
            attached_editor: None,
            editor_mode: Some(Mode::Normal),
            ..Default::default()
        };
        assert_eq!((EDITOR_NAV.probe)(&orphan), None);
        assert_eq!((EDITOR_INSERT.probe)(&orphan), None);
    }

    #[test]
    fn the_barrier_is_a_root_and_the_navigable_surface_is_not() {
        // If `editor.insert` had `panel` as its parent, the barrier would be
        // pointless: the walk would reach panel's Ctrl+hjkl rules anyway.
        assert_eq!(EDITOR_INSERT.parent, None);
        assert_eq!(EDITOR_INSERT.seal, Seal::Barrier);
        assert_eq!(EDITOR_NAV.parent, Some("panel"));
        assert_eq!(EDITOR_NAV.seal, Seal::Open);
    }

    #[test]
    fn only_editor_nav_yields_to_the_engine() {
        assert!(EDITOR_NAV.yields_to_engine);
        assert!(!EDITOR_INSERT.yields_to_engine);
    }
}
