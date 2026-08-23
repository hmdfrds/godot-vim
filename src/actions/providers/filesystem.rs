//! `dock.filesystem` — the FileSystem dock's own Tree and file list.
//!
//! The surface that carries the nvim-tree keyset (`a` create, `d` delete,
//! `r` rename, `y` yank path, `R` refresh) and the one place in the design
//! where a *surface* grants a capability.
//!
//! # `FILEOPS` is dock membership, not a widget class
//!
//! `Caps::of_control` can never contribute [`Caps::FILEOPS`], because no
//! widget class implies it: the FileSystem dock's Tree and the Scene tree are
//! both a `Tree`. Granting it here is what stops
//! `panelmap dock a godotvim.fs.create` from creating files at `res://` root
//! from a focused Scene tree — `get_selected_path` returns `None` for a non-FS
//! Tree and `begin_create` falls back to `"res://"`
//! (`src/navigation/filesystem_explorer.rs`).
//!
//! # Why depth replaces the hardcoded FileSystem-first branch
//!
//! `src/plugin/input.rs` hardcodes "ask the FileSystem explorer first,
//! then fall back to generic dock input". Here that is just the forest:
//! `dock.filesystem` names `dock` as its parent, so it is deeper, so its
//! bindings are consulted first and `j` — which it does not bind — falls
//! through to `dock` by the ordinary upward walk. The branch disappears with
//! no replacement.
//!
//! # Why the probe is not `chain.in_filesystem_dock` alone
//!
//! Because a focused `Button` in the FileSystem dock would then anchor here
//! and be granted `FILEOPS`, and `a` would open the create prompt from a
//! toolbar button — behaviour today's `classify_focus` cannot produce, since
//! it answers `Unknown` for a Button and never reaches the FS explorer at all.
//! The probe is the conjunction: in the dock **and** on a widget the dock
//! surfaces know how to drive.

use crate::actions::caps::Caps;
use crate::actions::surface::{Anchor, Seal, SurfaceSpec};

use super::dock::focuses_nav_widget;
use super::Provider;

pub(crate) static DOCK_FILESYSTEM: SurfaceSpec = SurfaceSpec {
    id: "dock.filesystem",
    parent: Some("dock"),
    seal: Seal::Open,
    grants: |_| Caps::FILEOPS,
    probe: |chain| {
        (chain.in_filesystem_dock && focuses_nav_widget(chain)).then_some(Anchor::Node(0))
    },
    overlay: None,
    // The one shipped hook, and it belongs to no binding: the stale-prompt
    // auto-dismiss that used to sit at the top of `FileSystemExplorer::
    // handle_key`, before the modifier filter and before any key was
    // resolved. Extracting the keyset into rules would otherwise have lost
    // it, and it is reachable for MORE keys here than it was there — the old
    // dispatcher's intercept predicate returned before the dock arm ran, so
    // Ctrl+L with the create prompt open moved focus away and left the prompt
    // pointing at a Tree it would later steal focus back to.
    on_key: Some(|cx| {
        if let Some(fs) = cx.fs() {
            fs.on_key_tick();
        }
    }),
    refuses_positional: false,
    yields_to_engine: false,
};

/// The nvim-tree-flavoured file operations of `resolve_fs_action`
/// (`filesystem_explorer.rs`).
///
/// `R` and `r` are two *keys*, not one key with a modifier: `bridge::input`
/// folds Shift into the character itself, so the discriminant is carried by
/// the char. Writing them as `R` and `r` here is therefore both the shipped
/// behaviour and the only spelling that can match.
///
/// `dock.filesystem` is deeper in the forest than `dock`, so these five get
/// first refusal while `j`/`k` still fall through to the parent. That depth is
/// the entire replacement for the hardcoded branch at
/// `src/plugin/input.rs`.
const DEFAULTS: &str = "\
panelmap <physical> dock.filesystem a godotvim.fs.create
panelmap <physical> dock.filesystem d godotvim.fs.delete
panelmap <physical> dock.filesystem r godotvim.fs.rename
panelmap <physical> dock.filesystem y godotvim.fs.yank_path
panelmap <physical> dock.filesystem R godotvim.fs.refresh
";

pub(crate) const PROVIDER: Provider = Provider {
    tag: "godotvim.filesystem",
    surfaces: &[&DOCK_FILESYSTEM],
    // Verbs stayed in `specs::SHIPPED` when P2 extracted them; see `Provider`.
    actions: &[],
    defaults: DEFAULTS,
};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::actions::surface::fixtures::*;
    use crate::actions::surface::{ChainNode, FocusChain};

    fn in_fs_dock(node: ChainNode) -> FocusChain {
        FocusChain {
            nodes: vec![
                node,
                plain("SplitContainer", 60),
                plain("FileSystemDock", 61),
            ],
            in_filesystem_dock: true,
            ..Default::default()
        }
    }

    #[test]
    fn the_filesystem_tree_and_list_both_anchor_here() {
        for node in [tree("FileSystemTree", 1), item_list("FileSystemList", 2)] {
            assert_eq!(
                (DOCK_FILESYSTEM.probe)(&in_fs_dock(node)),
                Some(Anchor::Node(0))
            );
        }
    }

    #[test]
    fn the_same_widget_outside_the_dock_does_not() {
        let elsewhere = FocusChain {
            nodes: vec![tree("SceneTreeEditor", 1), plain("SceneTreeDock", 62)],
            in_filesystem_dock: false,
            ..Default::default()
        };
        assert_eq!((DOCK_FILESYSTEM.probe)(&elsewhere), None);
    }

    #[test]
    fn a_button_inside_the_filesystem_dock_does_not_get_fileops() {
        // `in_filesystem_dock` alone would claim this, and `a` would open the
        // create prompt from a focused toolbar button.
        assert_eq!(
            (DOCK_FILESYSTEM.probe)(&in_fs_dock(plain("Button", 3))),
            None
        );
        assert_eq!((DOCK_FILESYSTEM.probe)(&in_fs_dock(line_edit(4))), None);
    }

    #[test]
    fn the_surface_grants_fileops_and_the_widget_never_does() {
        let chain = in_fs_dock(tree("FileSystemTree", 1));
        assert_eq!((DOCK_FILESYSTEM.grants)(&chain), Caps::FILEOPS);
        assert!(
            !chain.widget_caps().contains(Caps::FILEOPS),
            "FILEOPS is dock membership, not a widget affordance"
        );
    }

    #[test]
    fn the_on_key_hook_dismisses_a_stale_prompt_through_the_ctx() {
        // The one shipped `on_key` hook, and it belongs to no binding — so
        // nothing in the resolver, the trie or the golden fixture table can
        // notice it going missing. Gutting the body to `Some(|_| {})` passed
        // 1416/1416 while silently reproducing the original bug: a create
        // prompt left open after focus moved back to the Tree, which
        // `dismiss_prompt` would later `call_deferred("grab_focus")` away from
        // wherever the user has since gone.
        //
        // Driven through `ActionCtx::with_fs` rather than by calling
        // `on_key_tick` directly, because the WIRING is the thing at risk: the
        // hook has to reach the explorer the transport lends it.
        let mut fs = crate::navigation::FileSystemExplorer::new();
        fs.arm_stale_prompt();
        assert!(
            fs.prompt_is_live(),
            "the fixture must start with a prompt up"
        );

        let hook = DOCK_FILESYSTEM
            .on_key
            .expect("dock.filesystem declares the one shipped hook");
        {
            let mut cx =
                crate::actions::action::ActionCtx::new(None, crate::actions::action::Params::new())
                    .with_fs(&mut fs);
            hook(&mut cx);
        }
        assert!(
            !fs.prompt_is_live(),
            "the hook must auto-dismiss the orphaned prompt"
        );
    }

    #[test]
    fn the_on_key_hook_is_idempotent_and_needs_no_prompt() {
        // The hook contract: it runs for EVERY key including key-repeat
        // echoes, before any lookup and whether or not a binding matches. A
        // second call must be a no-op rather than a second dismiss.
        let mut fs = crate::navigation::FileSystemExplorer::new();
        let hook = DOCK_FILESYSTEM.on_key.expect("shipped hook");
        for _ in 0..3 {
            let mut cx =
                crate::actions::action::ActionCtx::new(None, crate::actions::action::Params::new())
                    .with_fs(&mut fs);
            hook(&mut cx);
        }
        assert!(!fs.prompt_is_live());
    }

    #[test]
    fn the_on_key_hook_declines_a_transport_that_lends_no_explorer() {
        // `:action` and the `gui_input` transport both leave `fs` unset. The
        // hook must not panic there — `cx.fs()` is the whole guard.
        let hook = DOCK_FILESYSTEM.on_key.expect("shipped hook");
        let mut cx =
            crate::actions::action::ActionCtx::new(None, crate::actions::action::Params::new());
        hook(&mut cx);
    }

    #[test]
    fn it_is_declared_deeper_than_the_generic_dock() {
        // The whole FileSystem-first refusal, as one parent link.
        assert_eq!(DOCK_FILESYSTEM.parent, Some("dock"));
        assert_eq!(DOCK_FILESYSTEM.seal, Seal::Open);
    }
}
