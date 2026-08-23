//! `panel` — the forest root, and the only surface that never probes.
//!
//! Everything reachable by Ctrl+hjkl lives under `panel`: it is the declared
//! parent of `dock`, `searchbox`, `prompt`, `editor.nav` and `unknown`. It is
//! reached **only** by following parent links, never by classification, which
//! is why its probe is `|_| None` and why it sits last in `PROVIDERS` where a
//! total probe could never shadow it.
//!
//! It grants nothing. An earlier draft gave it a `PANEL` capability; that was a
//! tautology — the root is on every non-`Barrier` path, so `requires: PANEL`
//! could never fail — and the `godotvim.focus.*` actions carry
//! `Caps::empty()` instead. Requiring nothing is exactly what lets cross-panel
//! movement still fire when there is no focus owner at all.

use crate::actions::caps::Caps;
use crate::actions::surface::{Seal, SurfaceSpec};

use super::Provider;

pub(crate) static PANEL: SurfaceSpec = SurfaceSpec {
    id: "panel",
    parent: None,
    seal: Seal::Open,
    grants: |_| Caps::empty(),
    // Never classified directly. A surface with no probe is not a surface with
    // no bindings: the panel keyset (Ctrl+hjkl, Ctrl+w cycling, Esc back to
    // the editor) all lives here, and reaches every descendant through the
    // upward walk.
    probe: |_| None,
    overlay: None,
    on_key: None,
    refuses_positional: false,
    yields_to_engine: false,
};

/// Cross-panel focus, in the exact form a user would write it.
///
/// `<void>` reproduces `src/plugin/input.rs`, where
/// `handle_window_nav`'s result is discarded at `:129` and
/// `set_input_as_handled()` fires at `:132` even with no focus owner and no
/// target found. `<norepeat>` keeps a held Ctrl+J from queueing a ~20/s storm
/// of deferred `grab_focus` calls. `<physical>` is what gives a Dvorak or
/// Cyrillic user the same four chords by position.
///
/// These four are also the reason the `<C-w>` grammar guard has to be a
/// question rather than a denylist: `panel` is `editor.nav`'s parent, so every
/// line here is validated against vim-core's own parser, and all four must
/// come back clean.
const DEFAULTS: &str = "\
panelmap <physical> <void> <norepeat> panel <C-h> godotvim.focus.left
panelmap <physical> <void> <norepeat> panel <C-j> godotvim.focus.down
panelmap <physical> <void> <norepeat> panel <C-k> godotvim.focus.up
panelmap <physical> <void> <norepeat> panel <C-l> godotvim.focus.right
";

pub(crate) const PROVIDER: Provider = Provider {
    tag: "godotvim.panel",
    surfaces: &[&PANEL],
    // Verbs stayed in `specs::SHIPPED` when P2 extracted them; see `Provider`.
    actions: &[],
    defaults: DEFAULTS,
};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::actions::surface::fixtures::*;
    use crate::actions::surface::FocusChain;

    #[test]
    fn the_root_never_claims_anything() {
        // Including the empty chain. If `panel` ever probed, it would have to
        // sit ahead of `unknown` to be reachable, and the two would race.
        for chain in [
            no_focus_owner(),
            FocusChain {
                nodes: vec![tree("Tree", 1)],
                ..Default::default()
            },
            FocusChain {
                nodes: vec![code_edit(1)],
                attached_editor: Some(id(1)),
                ..Default::default()
            },
        ] {
            assert!((PANEL.probe)(&chain).is_none());
        }
    }

    #[test]
    fn the_root_is_a_root() {
        assert_eq!(PANEL.parent, None);
        assert_eq!(PANEL.seal, Seal::Open);
        assert!(!PANEL.yields_to_engine);
    }
}
