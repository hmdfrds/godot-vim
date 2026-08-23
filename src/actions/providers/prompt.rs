//! `prompt` — the FileSystem create/rename `LineEdit` this plugin owns.
//!
//! A plugin-owned control, built by `FileSystemExplorer::ensure_prompt` and
//! parented into the FileSystem dock's own `VBoxContainer`
//! (`src/navigation/filesystem_explorer.rs`). Nothing about its
//! *class* distinguishes it from any other `LineEdit`; the discriminant is
//! instance identity, which `FocusChain::is_plugin_prompt` records at sample
//! time.
//!
//! # Why it is probed third, ahead of `searchbox` and `foreign`
//!
//! Both of the later probes can claim a `LineEdit`, and which one would depend
//! on a fact nobody can determine without running the editor: whether
//! `find_sibling_nav_control` finds the FileSystem tree from the prompt. It
//! does today — the prompt's `HBoxContainer` shares a parent with the dock's
//! `FileSystemTree` — so `searchbox` would win, and `<Esc>` would be routed to
//! "leave the filter box" instead of dismissing the prompt. Re-parent the
//! prompt one level and the answer flips to `foreign`, a `Barrier`, where
//! `<Esc>` reaches nothing at all. Giving `prompt` first refusal makes the
//! question irrelevant instead of load-bearing.
//!
//! `Seal::Sealed` is what keeps the prompt usable: bare keys fall through to
//! the `LineEdit`'s own `gui_input`, so typing a filename types and `<CR>`
//! still reaches `text_submitted`, while Ctrl+hjkl continues up to `panel` and
//! can still escape.

use crate::actions::caps::Caps;
use crate::actions::surface::{Anchor, Seal, SurfaceSpec};

use super::Provider;

pub(crate) static PROMPT: SurfaceSpec = SurfaceSpec {
    id: "prompt",
    parent: Some("panel"),
    seal: Seal::Sealed,
    grants: |_| Caps::empty(),
    // The `focus()` guard is not redundant: `Anchor::Node(0)` promises a node
    // at index 0, and a chain can carry a stale flag with an empty node list.
    probe: |chain| (chain.is_plugin_prompt && chain.focus().is_some()).then_some(Anchor::Node(0)),
    overlay: None,
    on_key: None,
    refuses_positional: false,
    yields_to_engine: false,
};

pub(crate) const PROVIDER: Provider = Provider {
    tag: "godotvim.prompt",
    surfaces: &[&PROMPT],
    // Verbs stayed in `specs::SHIPPED` when P2 extracted them; see `Provider`.
    actions: &[],
    // None yet. The FS prompt's Escape is handled inside
    // `FileSystemExplorer::handle_key` on the `gui_input` transport, which P6
    // routes through the resolver; binding it here now would make the same
    // key fire twice.
    defaults: "",
};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::actions::surface::fixtures::*;
    use crate::actions::surface::FocusChain;

    fn prompt_chain(sibling: Option<i64>) -> FocusChain {
        FocusChain {
            nodes: vec![
                line_edit(30),
                plain("HBoxContainer", 31),
                plain("VBoxContainer", 32),
                plain("FileSystemDock", 33),
            ],
            in_filesystem_dock: true,
            sibling_nav_control: sibling.map(id),
            is_plugin_prompt: true,
            ..Default::default()
        }
    }

    #[test]
    fn the_plugin_prompt_is_claimed_whatever_its_siblings_look_like() {
        // Both answers to "does it have a sibling nav control" must land here,
        // because the answer is not knowable without a running editor.
        assert_eq!(
            (PROMPT.probe)(&prompt_chain(Some(11))),
            Some(Anchor::Node(0))
        );
        assert_eq!((PROMPT.probe)(&prompt_chain(None)), Some(Anchor::Node(0)));
    }

    #[test]
    fn a_dock_filter_box_is_not_the_prompt() {
        let filter = FocusChain {
            nodes: vec![line_edit(40), plain("SceneTreeDock", 41)],
            sibling_nav_control: Some(id(42)),
            ..Default::default()
        };
        assert_eq!((PROMPT.probe)(&filter), None);
    }

    #[test]
    fn a_stale_flag_with_no_focus_owner_claims_nothing() {
        let stale = FocusChain {
            is_plugin_prompt: true,
            ..Default::default()
        };
        assert_eq!((PROMPT.probe)(&stale), None);
    }

    #[test]
    fn the_prompt_is_sealed_not_barred() {
        // Barrier would kill `<CR>` → `text_submitted`; Open would let bare
        // keys be claimed as commands and the user could not type a filename.
        assert_eq!(PROMPT.seal, Seal::Sealed);
        assert_eq!(PROMPT.parent, Some("panel"));
    }
}
