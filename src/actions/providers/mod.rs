//! The shipped surface providers, in probe order.
//!
//! A provider is one file that declares one subsystem's surfaces (and, from
//! P5, its actions and default bindings). Adding a panel to the plugin is one
//! new file here plus one line in [`PROVIDERS`]; `src/plugin/input.rs`,
//! `src/actions/caps.rs` and every other provider stay untouched.
//!
//! # Why a `const` array and not `inventory`/`linkme`
//!
//! Link-time registration was rejected outright. Life-before-main constructors
//! in a `cdylib` that the Godot editor `dlopen`s, under `lto = "fat"` with
//! linker section GC and GDExtension hot-reload, is a cross-platform footgun
//! for zero semantic gain. A `const` array is compile-time checked, reviewable
//! in a diff, and deterministically ordered — which the ordering audits below
//! and the introspector's golden snapshots both depend on.
//!
//! # The order is the classification, and it is fixed in both directions
//!
//! Probes run in array order and the **first `Some` wins**. That is what
//! preserves the mutual exclusivity today's `classify_focus` gets structurally
//! from being one `if`/`else` chain, and which N independent predicates would
//! otherwise silently lose. Four constraints pin the order:
//!
//! 1. **`unknown` probes unconditionally**, so it must be the last *probing*
//!    entry. Anything behind it is unreachable.
//! 2. **`foreign` must therefore precede `unknown`.** Behind it, a Project
//!    Settings `LineEdit` would resolve to `unknown` → `panel` instead of to a
//!    `Barrier`, and Ctrl+hjkl would be consumed mid-word — a direct violation
//!    of the old dispatcher's refusal to touch a foreign text input.
//! 3. **`foreign` must not be first.** Its predicate claims "a `LineEdit` with
//!    no sibling nav control", and whether the plugin's own FileSystem prompt
//!    has one is an editor-runtime fact nobody can settle from source. Ahead
//!    of `prompt` it could take the prompt, and `<Esc>` would meet a `Barrier`
//!    instead of dismissing it. `prompt`, `searchbox`, `filesystem`, `dock`
//!    and `editor` all get first refusal for the same reason.
//! 4. **A descendant precedes its ancestor** — `filesystem` and `debugger`
//!    both before `dock` — or the child never matches and its bindings are
//!    silently dead.
//!
//! Two surfaces never probe at all, and their positions are therefore free.
//! `panel` is the forest root, reached only by following parent links, so no
//! total probe can shadow it. `editor.completion` is an OVERLAY: it declares
//! an `OverlaySpec` instead of a probe, and `Forest::overlay` materialises it
//! per keystroke for the `gui_input` transport, because whether the
//! autocomplete popup is visible is a per-keystroke fact and the sampled
//! `FocusChain` is cached per focus change. Both are excluded from the
//! golden-fixture coverage audit for the same reason: a fixture for a surface
//! that cannot be classified would prove nothing.
//!
//! Every clause above is a test in this module's `ordering` block, each
//! written so that reordering the array fails the suite rather than a user's
//! keyboard.

pub(crate) mod completion;
pub(crate) mod debugger;
pub(crate) mod dock;
pub(crate) mod editor;
pub(crate) mod filesystem;
pub(crate) mod foreign;
pub(crate) mod panel;
pub(crate) mod prompt;
pub(crate) mod searchbox;
pub(crate) mod unknown;

use super::action::ActionSpec;
use super::surface::{Forest, SurfaceSpec};

/// One subsystem's contribution to the plane.
///
/// The `tag` is what every rule this provider installs is owned by, which
/// makes "a third party cannot silently displace a builtin binding" checkable
/// rather than aspirational.
pub(crate) struct Provider {
    pub(crate) tag: &'static str,
    /// Surfaces this provider declares, in probe order.
    pub(crate) surfaces: &'static [&'static SurfaceSpec],
    /// Verbs this provider contributes to the [`ActionRegistry`].
    ///
    /// This field is what makes §7.1's "one file plus one manifest line"
    /// literally true rather than nearly true. Without it a new subsystem's
    /// `ActionSpec`s would have to be appended to `actions::specs::SHIPPED`,
    /// which is a second registration point in a file the new provider has no
    /// business editing — and `builtin_index` would reject its defaults with
    /// `UnknownAction` until someone did.
    ///
    /// Empty for the eight original providers: their verbs were extracted into
    /// `specs::SHIPPED` in P2, before this field existed, and moving them now
    /// would be churn with no behavioural change. `specs::registry()` unions
    /// both sources, and `every_provider_verb_is_registered` below pins that
    /// the union is what actually reaches the registry.
    ///
    /// [`ActionRegistry`]: crate::actions::action::ActionRegistry
    pub(crate) actions: &'static [&'static ActionSpec],
    /// Default bindings, written in **exactly the text a user types** and
    /// parsed by exactly the same parser (`crate::config::panelmap`).
    ///
    /// That is the anti-drift device of the whole design. Defaults built by
    /// calling constructors directly could be expressed in a dialect the
    /// documented grammar does not describe, the parser and the sandbox
    /// whitelist could not be held to one property, and a user rebinding
    /// Ctrl+hjkl could not reproduce the shipped semantics.
    ///
    /// A line here that fails to load is a `debug_assert!`, not a warning:
    /// warn-and-skip is the policy for user text, and a shipped default that
    /// silently vanishes is a keyset regression with a green test suite.
    pub(crate) defaults: &'static str,
}

/// Probe order. See the module docs — every position here is load-bearing.
pub(crate) const PROVIDERS: &[Provider] = &[
    // editor.nav / editor.insert — the attached CodeEdit, split by mode. First
    // refusal on our own editor, before anything can claim it as a text input.
    editor::PROVIDER,
    // editor.completion — the autocomplete popup's keys, as an OVERLAY.
    // Position here is arbitrary and that is a property, not a hole: its
    // probe is `|_| None`, so it is unreachable by classification from ANY
    // position; the `gui_input` transport asks `Forest::overlay` for it per
    // keystroke instead. It sits beside the other editor surfaces so a
    // reader finds it where they expect.
    completion::PROVIDER,
    // prompt — our own LineEdit, by instance identity. Ahead of `searchbox`
    // and `foreign`, both of which could otherwise claim it.
    prompt::PROVIDER,
    // searchbox — a dock filter LineEdit, discriminated by a sibling nav
    // control.
    searchbox::PROVIDER,
    // dock.filesystem — before `dock`, its declared parent, or the FileSystem
    // keyset is unreachable.
    filesystem::PROVIDER,
    // dock.debugger — THE ENTIRE REGISTRATION of a new subsystem (§7.2). Same
    // constraint as its sibling above: before `dock`, its declared parent, or
    // the child never matches and its four bindings are silently dead. `Forest
    // ::audit` checks that rather than trusting this comment.
    debugger::PROVIDER,
    // dock — the generic Tree/ItemList/RichTextLabel surface.
    dock::PROVIDER,
    // foreign — a Barrier, but only AFTER every surface that needs first
    // refusal and BEFORE `unknown`, or it is unreachable.
    foreign::PROVIDER,
    // unknown — the TOTAL probe. Must be the last probing entry.
    unknown::PROVIDER,
    // panel — the forest root. Never probes; reached only through parent links.
    panel::PROVIDER,
];

/// Every declared surface, flattened in probe order.
pub(crate) fn surfaces() -> Vec<&'static SurfaceSpec> {
    PROVIDERS
        .iter()
        .flat_map(|p| p.surfaces.iter().copied())
        .collect()
}

/// Every verb the providers contribute, flattened in `PROVIDERS` order.
///
/// Unioned with `specs::SHIPPED` by `specs::registry()`, which is the single
/// place the registry is built. A provider that ships a default naming a verb
/// it did not declare here is rejected at load with `UnknownAction` — under
/// `Provenance::Builtin` that is a `debug_assert!`, so it fails the suite
/// rather than shipping a dead key.
pub(crate) fn actions() -> Vec<&'static ActionSpec> {
    PROVIDERS
        .iter()
        .flat_map(|p| p.actions.iter().copied())
        .collect()
}

/// The shipped forest, ready to classify — and audited on the way out.
///
/// The audit is what makes three separate doc comments true rather than
/// aspirational. Before this call existed, [`Forest::audit`] implemented the
/// design's V1/V3/V4 validations and every caller of it was inside
/// `#[cfg(test)]`, so a provider added by a third party got exactly the
/// enforcement its author assumed was already there: none. A duplicate id, an
/// undeclared parent, a parent cycle or an ancestor probed before its
/// descendant each produce a surface whose bindings are silently dead.
///
/// Two channels on purpose. `debug_assert!` fails the suite for an in-repo
/// mistake, which is where the shipped forest's errors belong. `log::error!`
/// is for a release build, where refusing to start would be a worse answer
/// than running with one broken surface — the rest of the keyset still works.
///
/// The cost is a walk over ~10 specs with a bounded `path_from` per spec, paid
/// once per `builtin_index` call — i.e. at load and at each config reload,
/// never on the input path, which reads `self.bindings.forest()`.
pub(crate) fn forest() -> Forest {
    audited(Forest::new(&surfaces()))
}

/// Run [`Forest::audit`] over a freshly built forest and pass it through.
///
/// A separate function purely so the failure path is reachable from a test:
/// the shipped forest is clean, so calling this on it proves nothing, and
/// `a_malformed_forest_is_rejected_at_construction` needs a forest that is
/// not.
fn audited(forest: Forest) -> Forest {
    let errors = forest.audit();
    debug_assert!(
        errors.is_empty(),
        "the surface forest is malformed: {errors:?}"
    );
    for error in &errors {
        log::error!("surface forest: {error}");
    }
    forest
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::actions::caps::Caps;
    use crate::actions::surface::fixtures::*;
    use crate::actions::surface::{Anchor, ChainNode, FocusChain, Seal, SurfaceId};
    use vim_core::primitives::{Mode, Operator, VisualType};

    // ── The golden table ─────────────────────────────────────────────
    //
    // Forty-two literal focus chains, each one a shape the editor really
    // produces, and each asserting the whole resolved path: surface ids,
    // capabilities, seal and anchor. This table is the plane's specification.
    // It is also the input to the partition audit and to every ordering test
    // below, which is why a new surface must arrive with a fixture — a surface
    // with no fixture is a surface nobody proved disjoint.

    const NAV: &[SurfaceId] = &["editor.nav", "panel"];
    const INSERT: &[SurfaceId] = &["editor.insert"];
    const FOREIGN: &[SurfaceId] = &["foreign"];
    const PROMPT: &[SurfaceId] = &["prompt", "panel"];
    const SEARCH: &[SurfaceId] = &["searchbox", "panel"];
    const FS: &[SurfaceId] = &["dock.filesystem", "dock", "panel"];
    const DEBUGGER: &[SurfaceId] = &["dock.debugger", "dock", "panel"];
    const DOCK: &[SurfaceId] = &["dock", "panel"];
    const UNKNOWN: &[SurfaceId] = &["unknown", "panel"];

    const ATTACHED: i64 = 7;

    struct Case {
        what: &'static str,
        chain: FocusChain,
        ids: &'static [SurfaceId],
        caps: Caps,
        seal: Seal,
        anchor: Anchor,
    }

    fn editor_chain(mode: Option<Mode>) -> FocusChain {
        FocusChain {
            nodes: vec![
                code_edit(ATTACHED),
                plain("CodeTextEditor", 101),
                plain("ScriptTextEditor", 102),
            ],
            attached_editor: Some(id(ATTACHED)),
            editor_mode: mode,
            ..Default::default()
        }
    }

    /// A chain inside Godot's FileSystem dock.
    fn fs_chain(focus: ChainNode) -> FocusChain {
        FocusChain {
            nodes: vec![
                focus,
                plain("SplitContainer", 110),
                plain("VBoxContainer", 111),
                plain("FileSystemDock", 112),
            ],
            in_filesystem_dock: true,
            ..Default::default()
        }
    }

    /// A chain inside some other dock.
    fn dock_chain(focus: ChainNode, dock_class: &'static str) -> FocusChain {
        FocusChain {
            nodes: vec![focus, plain("VBoxContainer", 120), plain(dock_class, 121)],
            ..Default::default()
        }
    }

    /// A chain inside the debugger dock, shaped like the real one:
    /// `EditorDebuggerNode` → `TabContainer` → `ScriptEditorDebugger` → the
    /// Stack Trace tab's containers → the focused control.
    fn debugger_chain(focus: ChainNode) -> FocusChain {
        FocusChain {
            nodes: vec![
                focus,
                plain("VBoxContainer", 190),
                plain("HSplitContainer", 191),
                plain("ScriptEditorDebugger", 192),
                plain("TabContainer", 193),
                plain("EditorDebuggerNode", 194),
            ],
            ..Default::default()
        }
    }

    #[allow(clippy::too_many_lines, reason = "a golden table is a table")]
    fn golden() -> Vec<Case> {
        let tree_caps = Caps::VNAV | Caps::HIERARCHY | Caps::ACTIVATE;
        let list_caps = Caps::VNAV | Caps::ACTIVATE;
        vec![
            // ── The attached CodeEdit, one case per mode ──────────────
            Case {
                what: "attached CodeEdit, no controller yet",
                chain: editor_chain(None),
                ids: NAV,
                caps: Caps::TEXTENTRY,
                seal: Seal::Open,
                anchor: Anchor::Node(0),
            },
            Case {
                what: "attached CodeEdit in Normal",
                chain: editor_chain(Some(Mode::Normal)),
                ids: NAV,
                caps: Caps::TEXTENTRY,
                seal: Seal::Open,
                anchor: Anchor::Node(0),
            },
            Case {
                what: "attached CodeEdit in Visual char",
                chain: editor_chain(Some(Mode::Visual(VisualType::Char))),
                ids: NAV,
                caps: Caps::TEXTENTRY,
                seal: Seal::Open,
                anchor: Anchor::Node(0),
            },
            Case {
                what: "attached CodeEdit in Visual line",
                chain: editor_chain(Some(Mode::Visual(VisualType::Line))),
                ids: NAV,
                caps: Caps::TEXTENTRY,
                seal: Seal::Open,
                anchor: Anchor::Node(0),
            },
            Case {
                what: "attached CodeEdit in Visual block",
                chain: editor_chain(Some(Mode::Visual(VisualType::Block))),
                ids: NAV,
                caps: Caps::TEXTENTRY,
                seal: Seal::Open,
                anchor: Anchor::Node(0),
            },
            Case {
                what: "attached CodeEdit in Operator-pending",
                chain: editor_chain(Some(Mode::OperatorPending(Operator::Delete))),
                ids: NAV,
                caps: Caps::TEXTENTRY,
                seal: Seal::Open,
                anchor: Anchor::Node(0),
            },
            Case {
                what: "attached CodeEdit in Insert — Ctrl+H is backspace",
                chain: editor_chain(Some(Mode::Insert)),
                ids: INSERT,
                caps: Caps::TEXTENTRY,
                seal: Seal::Barrier,
                anchor: Anchor::Node(0),
            },
            Case {
                what: "attached CodeEdit in Replace",
                chain: editor_chain(Some(Mode::Replace)),
                ids: INSERT,
                caps: Caps::TEXTENTRY,
                seal: Seal::Barrier,
                anchor: Anchor::Node(0),
            },
            Case {
                what: "attached CodeEdit in Virtual replace",
                chain: editor_chain(Some(Mode::VirtualReplace)),
                ids: INSERT,
                caps: Caps::TEXTENTRY,
                seal: Seal::Barrier,
                anchor: Anchor::Node(0),
            },
            Case {
                what: "attached CodeEdit on the command line",
                chain: editor_chain(Some(Mode::CommandLine)),
                ids: INSERT,
                caps: Caps::TEXTENTRY,
                seal: Seal::Barrier,
                anchor: Anchor::Node(0),
            },
            Case {
                what: "attached CodeEdit in Select — insert-like on purpose",
                chain: editor_chain(Some(Mode::Select(VisualType::Char))),
                ids: INSERT,
                caps: Caps::TEXTENTRY,
                seal: Seal::Barrier,
                anchor: Anchor::Node(0),
            },
            // ── Text inputs that are not ours ─────────────────────────
            Case {
                what: "a CodeEdit in an addon panel while ours is attached",
                chain: FocusChain {
                    nodes: vec![code_edit(9), plain("AddonPanel", 130)],
                    attached_editor: Some(id(ATTACHED)),
                    editor_mode: Some(Mode::Normal),
                    ..Default::default()
                },
                ids: FOREIGN,
                caps: Caps::TEXTENTRY,
                seal: Seal::Barrier,
                anchor: Anchor::Node(0),
            },
            Case {
                what: "a CodeEdit with nothing attached at all",
                chain: FocusChain {
                    nodes: vec![code_edit(9)],
                    ..Default::default()
                },
                ids: FOREIGN,
                caps: Caps::TEXTENTRY,
                seal: Seal::Barrier,
                anchor: Anchor::Node(0),
            },
            Case {
                what: "a plain TextEdit",
                chain: FocusChain {
                    nodes: vec![text_edit(131), plain("EditorPropertyText", 132)],
                    ..Default::default()
                },
                ids: FOREIGN,
                caps: Caps::TEXTENTRY,
                seal: Seal::Barrier,
                anchor: Anchor::Node(0),
            },
            Case {
                what: "a Project Settings LineEdit — the Ctrl+hjkl hard stop",
                chain: FocusChain {
                    nodes: vec![line_edit(133), plain("EditorSettingsDialog", 134)],
                    ..Default::default()
                },
                ids: FOREIGN,
                caps: Caps::TEXTENTRY,
                seal: Seal::Barrier,
                anchor: Anchor::Node(0),
            },
            Case {
                what: "a TextEdit inside the FileSystem dock is still foreign",
                chain: fs_chain(text_edit(135)),
                ids: FOREIGN,
                caps: Caps::TEXTENTRY,
                seal: Seal::Barrier,
                anchor: Anchor::Node(0),
            },
            Case {
                what: "a TextEdit beside a nav control is foreign, not a filter box",
                chain: FocusChain {
                    nodes: vec![text_edit(136)],
                    sibling_nav_control: Some(id(137)),
                    ..Default::default()
                },
                ids: FOREIGN,
                caps: Caps::TEXTENTRY,
                seal: Seal::Barrier,
                anchor: Anchor::Node(0),
            },
            // ── The plugin's own prompt ───────────────────────────────
            Case {
                what: "FS create prompt, with a sibling nav control (today's shape)",
                chain: FocusChain {
                    sibling_nav_control: Some(id(141)),
                    is_plugin_prompt: true,
                    ..fs_chain(line_edit(140))
                },
                ids: PROMPT,
                caps: Caps::TEXTENTRY,
                seal: Seal::Sealed,
                anchor: Anchor::Node(0),
            },
            Case {
                what: "FS create prompt, with no sibling nav control",
                chain: FocusChain {
                    is_plugin_prompt: true,
                    ..fs_chain(line_edit(140))
                },
                ids: PROMPT,
                caps: Caps::TEXTENTRY,
                seal: Seal::Sealed,
                anchor: Anchor::Node(0),
            },
            Case {
                what: "FS prompt in a floated dock, outside the dock ancestry",
                chain: FocusChain {
                    nodes: vec![line_edit(140), plain("Window", 142)],
                    is_plugin_prompt: true,
                    ..Default::default()
                },
                ids: PROMPT,
                caps: Caps::TEXTENTRY,
                seal: Seal::Sealed,
                anchor: Anchor::Node(0),
            },
            // ── Dock filter boxes ─────────────────────────────────────
            Case {
                what: "the FileSystem dock's own filter box",
                chain: FocusChain {
                    sibling_nav_control: Some(id(151)),
                    ..fs_chain(line_edit(150))
                },
                ids: SEARCH,
                caps: Caps::TEXTENTRY,
                seal: Seal::Sealed,
                anchor: Anchor::Node(0),
            },
            Case {
                what: "the Scene tree dock's filter box",
                chain: FocusChain {
                    sibling_nav_control: Some(id(153)),
                    ..dock_chain(line_edit(152), "SceneTreeDock")
                },
                ids: SEARCH,
                caps: Caps::TEXTENTRY,
                seal: Seal::Sealed,
                anchor: Anchor::Node(0),
            },
            Case {
                what: "the Inspector's property filter box",
                chain: FocusChain {
                    sibling_nav_control: Some(id(155)),
                    ..dock_chain(line_edit(154), "InspectorDock")
                },
                ids: SEARCH,
                caps: Caps::TEXTENTRY,
                seal: Seal::Sealed,
                anchor: Anchor::Node(0),
            },
            // ── The FileSystem dock ───────────────────────────────────
            Case {
                what: "the FileSystem tree",
                chain: fs_chain(tree("FileSystemTree", 160)),
                ids: FS,
                caps: tree_caps | Caps::FILEOPS,
                seal: Seal::Open,
                anchor: Anchor::Node(0),
            },
            Case {
                what: "the FileSystem file list",
                chain: fs_chain(item_list("FileSystemList", 161)),
                ids: FS,
                caps: list_caps | Caps::FILEOPS,
                seal: Seal::Open,
                anchor: Anchor::Node(0),
            },
            Case {
                what: "a RichTextLabel inside the FileSystem dock",
                chain: fs_chain(rich_text("RichTextLabel", 162)),
                ids: FS,
                caps: Caps::VNAV | Caps::FILEOPS,
                seal: Seal::Open,
                anchor: Anchor::Node(0),
            },
            // ── The debugger dock ─────────────────────────────────────
            //
            // §7.3 makes a fixture per new surface mandatory rather than
            // courteous: the partition audit runs over this table, so a
            // surface with no row here is a surface nobody proved disjoint.
            Case {
                what: "the debugger's Stack Frames tree",
                chain: debugger_chain(tree("Tree", 195)),
                ids: DEBUGGER,
                caps: tree_caps,
                seal: Seal::Open,
                anchor: Anchor::Node(0),
            },
            Case {
                what: "the debugger's Breakpoints tree — same class, same surface",
                chain: FocusChain {
                    nodes: vec![
                        tree("Tree", 196),
                        plain("HSplitContainer", 197),
                        plain("ScriptEditorDebugger", 198),
                        plain("TabContainer", 199),
                        plain("EditorDebuggerNode", 200),
                    ],
                    ..Default::default()
                },
                ids: DEBUGGER,
                caps: tree_caps,
                seal: Seal::Open,
                anchor: Anchor::Node(0),
            },
            Case {
                what: "the debugger's thread OptionButton — inside the dock, but not a Tree",
                chain: debugger_chain(plain("OptionButton", 201)),
                ids: UNKNOWN,
                caps: Caps::empty(),
                seal: Seal::Open,
                anchor: Anchor::Node(0),
            },
            Case {
                what: "the debugger's 'Filter Stack Variables' box is a filter box, not a debugger",
                chain: FocusChain {
                    sibling_nav_control: Some(id(203)),
                    ..debugger_chain(line_edit(202))
                },
                ids: SEARCH,
                caps: Caps::TEXTENTRY,
                seal: Seal::Sealed,
                anchor: Anchor::Node(0),
            },
            Case {
                what: "the debugger's Errors tree is a debugger tree too",
                chain: debugger_chain(tree("Tree", 204)),
                ids: DEBUGGER,
                caps: tree_caps,
                seal: Seal::Open,
                anchor: Anchor::Node(0),
            },
            // ── Every other dock ──────────────────────────────────────
            Case {
                what: "the Scene tree",
                chain: dock_chain(tree("SceneTreeEditor", 170), "SceneTreeDock"),
                ids: DOCK,
                caps: tree_caps,
                seal: Seal::Open,
                anchor: Anchor::Node(0),
            },
            Case {
                what: "the Import dock's tree",
                chain: dock_chain(tree("Tree", 171), "ImportDock"),
                ids: DOCK,
                caps: tree_caps,
                seal: Seal::Open,
                anchor: Anchor::Node(0),
            },
            Case {
                what: "the open-scripts list",
                chain: dock_chain(item_list("ItemList", 172), "ScriptEditor"),
                ids: DOCK,
                caps: list_caps,
                seal: Seal::Open,
                anchor: Anchor::Node(0),
            },
            Case {
                what: "the built-in docs panel (EditorHelp class_desc)",
                chain: dock_chain(rich_text("RichTextLabel", 173), "EditorHelp"),
                ids: DOCK,
                caps: Caps::VNAV,
                seal: Seal::Open,
                anchor: Anchor::Node(0),
            },
            Case {
                what: "the Output log (EditorLog log)",
                chain: dock_chain(rich_text("RichTextLabel", 174), "EditorLog"),
                ids: DOCK,
                caps: Caps::VNAV,
                seal: Seal::Open,
                anchor: Anchor::Node(0),
            },
            Case {
                what:
                    "a FileSystemTree that is NOT inside the dock — the fact decides, not the class",
                chain: dock_chain(tree("FileSystemTree", 175), "SomeAddonDock"),
                ids: DOCK,
                caps: tree_caps,
                seal: Seal::Open,
                anchor: Anchor::Node(0),
            },
            Case {
                what: "a dock tree focused while an editor is attached elsewhere",
                chain: FocusChain {
                    attached_editor: Some(id(ATTACHED)),
                    editor_mode: Some(Mode::Normal),
                    ..dock_chain(tree("SceneTreeEditor", 176), "SceneTreeDock")
                },
                ids: DOCK,
                caps: tree_caps,
                seal: Seal::Open,
                anchor: Anchor::Node(0),
            },
            Case {
                what: "a favorites ItemList in the FileSystem dock, dock ancestry only",
                chain: dock_chain(item_list("FileSystemList", 177), "SomeOtherDock"),
                ids: DOCK,
                caps: list_caps,
                seal: Seal::Open,
                anchor: Anchor::Node(0),
            },
            // ── Everything else ───────────────────────────────────────
            Case {
                what: "a focused GraphEdit — no `graph` surface, and none needed",
                chain: dock_chain(plain("GraphEdit", 180), "VisualShaderEditor"),
                ids: UNKNOWN,
                caps: Caps::empty(),
                seal: Seal::Open,
                anchor: Anchor::Node(0),
            },
            Case {
                what: "a focused Button inside a dock",
                chain: dock_chain(plain("Button", 181), "SceneTreeDock"),
                ids: UNKNOWN,
                caps: Caps::empty(),
                seal: Seal::Open,
                anchor: Anchor::Node(0),
            },
            Case {
                what: "a focused Button inside the FileSystem dock — must NOT get FILEOPS",
                chain: fs_chain(plain("Button", 182)),
                ids: UNKNOWN,
                caps: Caps::empty(),
                seal: Seal::Open,
                anchor: Anchor::Node(0),
            },
            Case {
                what: "a focused OptionButton in the FileSystem dock toolbar",
                chain: fs_chain(plain("OptionButton", 183)),
                ids: UNKNOWN,
                caps: Caps::empty(),
                seal: Seal::Open,
                anchor: Anchor::Node(0),
            },
            Case {
                what: "a focused HSlider in an inspector row",
                chain: dock_chain(plain("HSlider", 184), "InspectorDock"),
                ids: UNKNOWN,
                caps: Caps::empty(),
                seal: Seal::Open,
                anchor: Anchor::Node(0),
            },
            Case {
                what: "a focused TabBar",
                chain: dock_chain(plain("TabBar", 185), "TabContainer"),
                ids: UNKNOWN,
                caps: Caps::empty(),
                seal: Seal::Open,
                anchor: Anchor::Node(0),
            },
            Case {
                what: "no focus owner at all",
                chain: no_focus_owner(),
                ids: UNKNOWN,
                caps: Caps::empty(),
                seal: Seal::Open,
                anchor: Anchor::Rootless,
            },
            Case {
                what: "no focus owner while an editor is attached and in Insert",
                chain: FocusChain {
                    attached_editor: Some(id(ATTACHED)),
                    editor_mode: Some(Mode::Insert),
                    ..Default::default()
                },
                ids: UNKNOWN,
                caps: Caps::empty(),
                seal: Seal::Open,
                anchor: Anchor::Rootless,
            },
            Case {
                what: "no focus owner with a stale in-dock flag",
                chain: FocusChain {
                    in_filesystem_dock: true,
                    ..Default::default()
                },
                ids: UNKNOWN,
                caps: Caps::empty(),
                seal: Seal::Open,
                anchor: Anchor::Rootless,
            },
        ]
    }

    #[test]
    fn at_most_one_overlay_is_active_for_any_facts() {
        // V-O4. `Forest::overlay` is first-active-wins in PROVIDERS order,
        // exactly as `classify` is; with one shipped overlay this can only
        // fail when a second one arrives with an overlapping predicate, which
        // is exactly when someone must decide the order on purpose.
        use crate::actions::surface::OverlayFacts;
        use vim_core::primitives::{Mode, Operator, VisualType};
        let all = surfaces();
        let modes = [
            None,
            Some(Mode::Normal),
            Some(Mode::Insert),
            Some(Mode::Replace),
            Some(Mode::VirtualReplace),
            Some(Mode::CommandLine),
            Some(Mode::Visual(VisualType::Char)),
            Some(Mode::Select(VisualType::Char)),
            Some(Mode::OperatorPending(Operator::Delete)),
        ];
        for at_attached_editor in [false, true] {
            for mode in modes {
                for selected_index in [-1, 0] {
                    let facts = OverlayFacts {
                        at_attached_editor,
                        mode,
                        selected_index,
                    };
                    let active = all
                        .iter()
                        .filter(|s| s.overlay.as_ref().is_some_and(|o| (o.active)(&facts)))
                        .count();
                    assert!(active <= 1, "{facts:?} activates {active} overlays");
                }
            }
        }
    }

    #[test]
    fn the_shipped_forest_declares_exactly_one_overlay() {
        // Named, so a second overlay is a deliberate edit here rather than a
        // silent join.
        let overlays: Vec<_> = surfaces()
            .iter()
            .filter(|s| s.overlay.is_some())
            .map(|s| s.id)
            .collect();
        assert_eq!(overlays, vec!["editor.completion"]);
    }

    #[test]
    fn no_overlay_surface_can_reserve_a_prefix() {
        // This is what keeps `Pending.caps` (`sequence.rs`) from ever
        // snapshotting `Caps::POPUP`, and it is now asserted rather than
        // coincidental: `editor_reachable` keeps multi-key LHSs refused at
        // registration, so no reservation can exist for the pending layer to
        // hold across keystrokes.
        let registry = crate::actions::specs::registry();
        let index = crate::actions::bind::builtin_index(&registry);
        assert!(!index.reserves_any("editor.completion"));
        assert!(index.editor_reachable("editor.completion"));
    }

    #[test]
    fn every_provider_verb_is_registered_and_namespaced_to_its_owner() {
        // The other half of the "one file plus one line" claim. A provider
        // that declares an `ActionSpec` nobody registers ships defaults that
        // fail to load — a `debug_assert!` under `Provenance::Builtin`, so it
        // is loud, but the message names the *binding* rather than the missing
        // registration. This names the registration.
        let registry = crate::actions::specs::registry();
        for provider in PROVIDERS {
            for spec in provider.actions {
                assert!(
                    registry.id_of(spec.id).is_some(),
                    "'{}' is declared by '{}' but never registered",
                    spec.id,
                    provider.tag
                );
                // A provider may not squat on another's namespace: V2 rejects a
                // duplicate id outright, and this catches the near miss that
                // would otherwise read as a builtin verb.
                assert!(
                    spec.id.starts_with(provider.tag),
                    "'{}' escapes its provider's namespace '{}'",
                    spec.id,
                    provider.tag
                );
            }
        }
    }

    #[test]
    fn no_two_providers_declare_the_same_verb() {
        // V2 in its structural form. `NameRegistry::register` is idempotent, so
        // a duplicate id would silently ALIAS an existing verb and the second
        // provider's `run` would never execute — a third-party action that
        // does nothing, with no diagnostic anywhere.
        let mut ids: Vec<&str> = actions().iter().map(|s| s.id).collect();
        ids.extend(crate::actions::specs::SHIPPED.iter().map(|s| s.id));
        let total = ids.len();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), total, "duplicate action id across providers");
    }

    #[test]
    fn the_golden_table_covers_every_declared_surface() {
        // A surface with no fixture is a surface nobody proved disjoint, so
        // this is what makes the partition audit meaningful rather than
        // decorative. `panel` is excluded: it never classifies directly.
        let forest = forest();
        let table = golden();
        assert!(table.len() >= 40, "table has only {} rows", table.len());
        // `panel` and every overlay are excluded, and for the same reason:
        // neither probes. `panel` is reached by following parent links, an
        // overlay is lent by `Forest::overlay` to the `gui_input` transport,
        // so neither can have a fixture and a fixture for either would prove
        // nothing. Deriving the exclusion from the declaration is what turns
        // a silent exclusion into a declared one, and keeps a second overlay
        // from silently joining an audit it cannot satisfy. Every surface
        // that DOES probe must still bring one — that is what makes the
        // partition audit below meaningful rather than decorative.
        for id in forest
            .ids()
            .filter(|id| *id != "panel" && forest.get(id).is_none_or(|s| s.overlay.is_none()))
        {
            assert!(
                table.iter().any(|c| c.ids.first() == Some(&id)),
                "no golden fixture resolves to '{id}'"
            );
        }
    }

    #[test]
    fn every_fixture_resolves_to_its_declared_path() {
        let forest = forest();
        for case in golden() {
            let path = forest
                .classify(&case.chain)
                .unwrap_or_else(|| panic!("'{}' classified to nothing", case.what));
            assert_eq!(path.ids, case.ids, "ids for '{}'", case.what);
            assert_eq!(path.caps, case.caps, "caps for '{}'", case.what);
            assert_eq!(path.seal, case.seal, "seal for '{}'", case.what);
            assert_eq!(path.anchor, case.anchor, "anchor for '{}'", case.what);
        }
    }

    #[test]
    fn every_path_ends_at_a_root_and_only_barriers_skip_panel() {
        // The focus-trap invariant in its structural form: a non-Barrier path
        // must reach `panel`, or the user is stuck somewhere Ctrl+hjkl cannot
        // leave. The two Barriers are the deliberate exceptions, and both are
        // escapable — Esc leaves insert mode, clicking leaves a foreign input.
        let forest = forest();
        for case in golden() {
            let path = forest.classify(&case.chain).expect("total");
            let ends_at_panel = path.ids.last() == Some(&"panel");
            match path.seal {
                Seal::Barrier => assert!(
                    !ends_at_panel,
                    "'{}' is a Barrier yet still reaches panel",
                    case.what
                ),
                Seal::Open | Seal::Sealed => assert!(
                    ends_at_panel,
                    "'{}' cannot reach panel — Ctrl+hjkl would be lost",
                    case.what
                ),
            }
        }
    }

    #[test]
    fn only_the_editor_surface_yields_to_the_engine() {
        let forest = forest();
        for case in golden() {
            let path = forest.classify(&case.chain).expect("total");
            assert_eq!(
                path.anchor_yields_to_engine,
                case.ids.first() == Some(&"editor.nav"),
                "'{}' — arbitration is a property of the surface",
                case.what
            );
        }
    }

    #[test]
    fn fileops_is_granted_by_exactly_one_surface() {
        // The loose end `Caps::of_control` structurally cannot close: no
        // widget class implies FILEOPS, so if the surface did not grant it,
        // nothing would, and every `godotvim.fs.*` binding would be dead.
        let forest = forest();
        for case in golden() {
            let path = forest.classify(&case.chain).expect("total");
            assert_eq!(
                path.caps.contains(Caps::FILEOPS),
                path.ids.contains(&"dock.filesystem"),
                "'{}' — FILEOPS must track dock membership exactly",
                case.what
            );
        }
    }

    #[test]
    fn only_the_catch_all_ever_anchors_rootless() {
        // `Anchor::Rootless` means "matched with no focus owner", and
        // `ActionCtx::target` is `None` for it. Any other surface returning it
        // would hand a target-less context to an action that assumes one.
        let forest = forest();
        for case in golden() {
            for (id, anchor) in forest.claimants(&case.chain) {
                if anchor == Anchor::Rootless {
                    assert_eq!(id, "unknown", "'{}' anchored rootless at '{id}'", case.what);
                }
            }
        }
    }

    // ── Agreement with the classifier this plane replaced ────────────

    /// The five answers the pre-surface-plane `classify_focus` gave. That
    /// function and the file it lived in are gone; this enum and the
    /// transcription below are all that remain of it.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum Legacy {
        Editor,
        Dock,
        SearchBox,
        Foreign,
        Unknown,
    }

    /// The old `classify_focus`, transcribed against the sampled chain
    /// instead of a live `Gd<Control>` — same order, same predicates, same
    /// fallthrough.
    ///
    /// It has no production twin any more: the classifier it copies was
    /// deleted with its last caller, and the forest is the only classifier
    /// left. What this still buys is a second, independently written
    /// statement of the five-way split, so a probe reordering that quietly
    /// changes who owns a control fails here as well as in `ordering`.
    fn classify_focus_equivalent(chain: &FocusChain) -> Legacy {
        let Some(node) = chain.focus() else {
            return Legacy::Unknown;
        };
        if node.is("CodeEdit") {
            if chain.attached_editor_focused() {
                return Legacy::Editor;
            }
            return Legacy::Foreign;
        }
        if node.is("Tree") || node.is("ItemList") || node.is("RichTextLabel") {
            return Legacy::Dock;
        }
        if node.is("LineEdit") {
            if chain.sibling_nav_control.is_some() {
                return Legacy::SearchBox;
            }
            return Legacy::Foreign;
        }
        if node.is("TextEdit") {
            return Legacy::Foreign;
        }
        Legacy::Unknown
    }

    fn legacy_of(surface: SurfaceId) -> Legacy {
        match surface {
            "editor.nav" | "editor.insert" => Legacy::Editor,
            // `dock.debugger` is a Tree like any other to today's classifier,
            // which is the whole point: the new surface refines where a key
            // lands without changing what the legacy code would have said.
            "dock" | "dock.filesystem" | "dock.debugger" => Legacy::Dock,
            // The prompt is a `SearchBox` to today's classifier, which is why
            // `input.rs` has to ask `is_prompt_active` and decline.
            "searchbox" | "prompt" => Legacy::SearchBox,
            "foreign" => Legacy::Foreign,
            "unknown" => Legacy::Unknown,
            other => unreachable!("no legacy answer for surface '{other}'"),
        }
    }

    #[test]
    fn the_new_classifier_agrees_with_classify_focus() {
        // The divergence count the design requires is zero, with exactly one
        // structural exception, asserted rather than waived: a plugin prompt
        // that has no sibling nav control. Today's classifier answers
        // `Foreign` for it — a Barrier — because it can only see a `LineEdit`
        // with no sibling. That is the very failure `prompt` exists to make
        // impossible, so disagreeing there is the point of the surface, not a
        // transcription slip.
        let forest = forest();
        for case in golden() {
            let path = forest.classify(&case.chain).expect("total");
            let new = legacy_of(path.ids[0]);
            let old = classify_focus_equivalent(&case.chain);
            if new == old {
                continue;
            }
            assert!(
                case.chain.is_plugin_prompt && new == Legacy::SearchBox && old == Legacy::Foreign,
                "'{}' diverges: surface plane says {new:?}, classify_focus says {old:?}",
                case.what
            );
        }
    }

    // ── The partition audit (V5) ─────────────────────────────────────

    /// The overlaps in the shipped forest that are resolved by ARRAY ORDER
    /// rather than by disjoint predicates, earlier surface first.
    ///
    /// Both entries are the same fact: the FileSystem prompt is a `LineEdit`
    /// this plugin builds and parents into Godot's own dock
    /// (`filesystem_explorer.rs`), so the two generic `LineEdit`
    /// predicates cannot be made to miss it. Whether
    /// `find_sibling_nav_control` reaches the dock's tree from the prompt is
    /// an editor-runtime fact — today it does, so `searchbox` would claim it;
    /// re-parent the prompt one level and `foreign` would. Teaching either
    /// predicate about `is_plugin_prompt` would put the decision in three
    /// places that must agree; giving `prompt` first refusal keeps it in one,
    /// where the `ordering` tests below can falsify it.
    ///
    /// A new entry here is a design decision that needs the same paragraph.
    /// An entry no fixture exercises is stale and fails the suite.
    const ORDERED_OVERRIDES: &[(SurfaceId, SurfaceId)] =
        &[("prompt", "searchbox"), ("prompt", "foreign")];

    /// Unrelated (earlier, later) claimant pairs over the whole golden table.
    fn unrelated_overlaps() -> Vec<(SurfaceId, SurfaceId, &'static str)> {
        let forest = forest();
        let mut found = Vec::new();
        for case in golden() {
            // `unknown` is excluded by construction: it is the designated
            // total probe, so it claims every fixture on purpose, and V4 —
            // asserted separately — is what keeps that harmless.
            let claimants: Vec<_> = forest
                .claimants(&case.chain)
                .into_iter()
                .map(|(id, _)| id)
                .filter(|id| *id != "unknown")
                .collect();
            for (i, a) in claimants.iter().enumerate() {
                for b in &claimants[i + 1..] {
                    if !forest.is_ancestor_or_self(a, b) && !forest.is_ancestor_or_self(b, a) {
                        found.push((*a, *b, case.what));
                    }
                }
            }
        }
        found
    }

    #[test]
    fn no_two_unrelated_surfaces_claim_the_same_fixture() {
        // Replacing an exclusive if/else chain with N independent probes
        // structurally loses mutual exclusivity. This is what buys it back:
        // two claimants are legal when one is the other's declared ancestor
        // (dock.filesystem ⊂ dock), because then array order is *derived* from
        // the forest — or when the pair is a documented ordered override.
        // Anything else is two providers disagreeing about who owns a control,
        // settled by array position: nondeterministic to the authors and
        // invisible to the user.
        for (a, b, what) in unrelated_overlaps() {
            assert!(
                ORDERED_OVERRIDES.contains(&(a, b)),
                "'{what}' is claimed by unrelated surfaces '{a}' and '{b}', \
                 and that pair is not a documented ordered override"
            );
        }
    }

    #[test]
    fn every_ordered_override_is_exercised_by_a_fixture() {
        // The other half: an override nobody triggers is either a predicate
        // that has since been made disjoint — in which case delete the entry
        // and let the audit be strict again — or a fixture that went missing.
        let overlaps = unrelated_overlaps();
        for pair in ORDERED_OVERRIDES {
            assert!(
                overlaps.iter().any(|(a, b, _)| (*a, *b) == *pair),
                "ordered override {pair:?} is stale — no golden fixture produces it"
            );
        }
    }

    #[test]
    fn the_first_claimant_is_always_the_winner() {
        // Classification is a *fold* with first-match-wins, not a search for
        // the most specific claimant. Anything else would make array order
        // decorative and the ordering audits meaningless.
        let forest = forest();
        for case in golden() {
            let first = forest.claimants(&case.chain).first().copied();
            let path = forest.classify(&case.chain).expect("total");
            assert_eq!(first, Some((path.ids[0], path.anchor)), "'{}'", case.what);
        }
    }

    // ── Ordering (V4) ────────────────────────────────────────────────

    mod ordering {
        use super::*;

        /// The shipped surfaces in a caller-chosen probe order.
        ///
        /// Every reorder test below builds one of these and asserts the WRONG
        /// answer it produces. That is the point: it makes each position in
        /// `PROVIDERS` falsifiable instead of merely commented.
        fn reordered(ids: &[SurfaceId]) -> crate::actions::surface::Forest {
            let all = surfaces();
            assert_eq!(
                ids.len(),
                all.len(),
                "a reorder must keep every surface, or it proves nothing"
            );
            let specs: Vec<_> = ids
                .iter()
                .map(|id| {
                    *all.iter()
                        .find(|s| s.id == *id)
                        .unwrap_or_else(|| panic!("no surface '{id}'"))
                })
                .collect();
            crate::actions::surface::Forest::new(&specs)
        }

        /// The shipped order, spelled out. Reordering `PROVIDERS` without
        /// updating this list fails here first, with the diff in the message.
        const SHIPPED: &[SurfaceId] = &[
            "editor.nav",
            "editor.insert",
            "editor.completion",
            "prompt",
            "searchbox",
            "dock.filesystem",
            "dock.debugger",
            "dock",
            "foreign",
            "unknown",
            "panel",
        ];

        #[test]
        fn the_array_order_is_the_documented_order() {
            assert_eq!(
                PROVIDERS.iter().map(|p| p.tag).collect::<Vec<_>>(),
                vec![
                    "godotvim.editor",
                    "godotvim.completion",
                    "godotvim.prompt",
                    "godotvim.searchbox",
                    "godotvim.filesystem",
                    "godotvim.debugger",
                    "godotvim.dock",
                    "godotvim.foreign",
                    "godotvim.unknown",
                    "godotvim.panel",
                ]
            );
            assert_eq!(surfaces().iter().map(|s| s.id).collect::<Vec<_>>(), SHIPPED);
        }

        #[test]
        fn the_forest_audit_passes_on_the_shipped_order() {
            // V1, V3 and the descendant-before-ancestor half of V4.
            assert_eq!(forest().audit(), Vec::<String>::new());
        }

        /// Two surfaces naming each other as parent — V3's cycle case.
        static CYCLE_A: crate::actions::surface::SurfaceSpec =
            crate::actions::surface::SurfaceSpec {
                id: "t.cycle_a",
                parent: Some("t.cycle_b"),
                seal: Seal::Open,
                grants: |_| Caps::empty(),
                probe: |_| None,
                overlay: None,
                on_key: None,
                yields_to_engine: false,
                refuses_positional: false,
            };
        static CYCLE_B: crate::actions::surface::SurfaceSpec =
            crate::actions::surface::SurfaceSpec {
                id: "t.cycle_b",
                parent: Some("t.cycle_a"),
                seal: Seal::Open,
                grants: |_| Caps::empty(),
                probe: |_| None,
                overlay: None,
                on_key: None,
                yields_to_engine: false,
                refuses_positional: false,
            };

        #[test]
        #[should_panic(expected = "the surface forest is malformed")]
        fn a_malformed_forest_is_rejected_at_construction() {
            // The half that was missing. `Forest::audit` implements V1/V3/V4
            // and, until `audited` existed, every caller of it was inside
            // `#[cfg(test)]` — while three doc comments told the next provider
            // author that a malformed forest is "rejected at registration".
            // It was not: a parent cycle produced a surface whose bindings are
            // silently dead, at runtime, with a green suite.
            let _ = audited(crate::actions::surface::Forest::new(&[&CYCLE_A, &CYCLE_B]));
        }

        #[test]
        fn a_clean_forest_passes_through_the_construction_audit_unchanged() {
            // The other direction: `audited` is a pass-through, not a filter.
            // A version that dropped surfaces on the way would take the whole
            // keyset out.
            let audited_ids: Vec<_> = audited(Forest::new(&surfaces())).ids().collect();
            assert_eq!(audited_ids, SHIPPED);
        }

        #[test]
        fn unknown_is_the_only_total_probe() {
            // "Total" is not decidable from a fn pointer, so it is decided
            // over the golden table: every other surface misses at least one
            // fixture, and `unknown` misses none.
            let forest = forest();
            let table = golden();
            for id in forest.ids() {
                let spec = forest.get(id).expect("declared");
                let claims = table
                    .iter()
                    .filter(|c| (spec.probe)(&c.chain).is_some())
                    .count();
                if id == "unknown" {
                    assert_eq!(claims, table.len(), "unknown must claim everything");
                } else {
                    assert!(claims < table.len(), "'{id}' has a total probe too");
                }
            }
        }

        #[test]
        fn nothing_probes_after_unknown() {
            // The rule that makes `foreign`'s position mandatory rather than
            // stylistic: behind a total probe, a surface is unreachable.
            let order = surfaces();
            let unknown_at = order
                .iter()
                .position(|s| s.id == "unknown")
                .expect("declared");
            for spec in &order[unknown_at + 1..] {
                for case in golden() {
                    assert_eq!(
                        (spec.probe)(&case.chain),
                        None,
                        "'{}' sits behind `unknown` and would never match '{}'",
                        spec.id,
                        case.what
                    );
                }
            }
        }

        fn project_settings_line_edit() -> FocusChain {
            FocusChain {
                nodes: vec![line_edit(200), plain("EditorSettingsDialog", 201)],
                ..Default::default()
            }
        }

        #[test]
        fn foreign_after_unknown_would_consume_ctrl_hjkl_in_project_settings() {
            let chain = project_settings_line_edit();

            let shipped = forest().classify(&chain).expect("total");
            assert_eq!(shipped.ids, vec!["foreign"]);
            assert_eq!(shipped.seal, Seal::Barrier, "dispatch stops before lookup");

            // Move `foreign` one place later and the barrier evaporates: the
            // path becomes unknown → panel, whose Ctrl+hjkl rules are `Void`,
            // and the key is consumed while the user is typing a setting name.
            let broken = reordered(&[
                "editor.completion",
                "editor.nav",
                "editor.insert",
                "prompt",
                "searchbox",
                "dock.filesystem",
                "dock.debugger",
                "dock",
                "unknown",
                "foreign",
                "panel",
            ]);
            let wrong = broken.classify(&chain).expect("total");
            assert_eq!(wrong.ids, vec!["unknown", "panel"]);
            assert_eq!(wrong.seal, Seal::Open, "this is the regression");
        }

        #[test]
        fn foreign_before_prompt_would_barrier_the_filesystem_prompt() {
            // `foreign` claims "a LineEdit with no sibling nav control", and
            // whether the prompt has one cannot be settled from source. With
            // `foreign` first, the prompt becomes a Barrier: Esc never
            // dismisses it and the stale prompt keeps stealing focus back.
            let chain = FocusChain {
                nodes: vec![line_edit(202), plain("HBoxContainer", 203)],
                is_plugin_prompt: true,
                ..Default::default()
            };

            let shipped = forest().classify(&chain).expect("total");
            assert_eq!(shipped.ids, vec!["prompt", "panel"]);
            assert_eq!(shipped.seal, Seal::Sealed);

            let broken = reordered(&[
                "editor.completion",
                "foreign",
                "editor.nav",
                "editor.insert",
                "prompt",
                "searchbox",
                "dock.filesystem",
                "dock.debugger",
                "dock",
                "unknown",
                "panel",
            ]);
            let wrong = broken.classify(&chain).expect("total");
            assert_eq!(wrong.ids, vec!["foreign"]);
            assert_eq!(wrong.seal, Seal::Barrier, "this is the regression");
        }

        #[test]
        fn searchbox_before_prompt_would_route_escape_to_the_wrong_verb() {
            // In today's editor the prompt DOES have a sibling nav control —
            // its HBox shares a parent with the FileSystemTree — so this is
            // the reordering that actually bites, not a hypothetical one.
            let chain = FocusChain {
                nodes: vec![line_edit(204), plain("HBoxContainer", 205)],
                sibling_nav_control: Some(id(206)),
                is_plugin_prompt: true,
                ..Default::default()
            };

            assert_eq!(
                forest().classify(&chain).expect("total").ids,
                vec!["prompt", "panel"]
            );

            let broken = reordered(&[
                "editor.completion",
                "editor.nav",
                "editor.insert",
                "searchbox",
                "prompt",
                "dock.filesystem",
                "dock.debugger",
                "dock",
                "foreign",
                "unknown",
                "panel",
            ]);
            assert_eq!(
                broken.classify(&chain).expect("total").ids,
                vec!["searchbox", "panel"],
                "this is the regression"
            );
        }

        #[test]
        fn dock_before_dock_filesystem_would_kill_the_filesystem_keyset() {
            let chain = fs_chain(tree("FileSystemTree", 207));

            let shipped = forest().classify(&chain).expect("total");
            assert!(shipped.caps.contains(Caps::FILEOPS));

            let broken = reordered(&[
                "editor.completion",
                "editor.nav",
                "editor.insert",
                "prompt",
                "searchbox",
                "dock.debugger",
                "dock",
                "dock.filesystem",
                "foreign",
                "unknown",
                "panel",
            ]);
            let wrong = broken.classify(&chain).expect("total");
            assert_eq!(wrong.ids, vec!["dock", "panel"]);
            assert!(
                !wrong.caps.contains(Caps::FILEOPS),
                "every godotvim.fs.* binding would be capability-gated out"
            );
            // And the audit catches it without needing a fixture at all.
            assert!(broken.audit().iter().any(|e| e.contains("dock.filesystem")));
        }

        #[test]
        fn dock_before_dock_debugger_would_kill_the_debugger_keyset() {
            // The sibling of the FileSystem case, and the reason the P9 diff
            // had to insert its line ABOVE `dock::PROVIDER` rather than
            // appending it. Appended, `dock` would claim the stack tree first,
            // the walk would never reach `dock.debugger`, and all four of its
            // bindings would be dead with a green suite.
            let chain = debugger_chain(tree("Tree", 208));

            let shipped = forest().classify(&chain).expect("total");
            assert_eq!(shipped.ids, vec!["dock.debugger", "dock", "panel"]);

            let broken = reordered(&[
                "editor.completion",
                "editor.nav",
                "editor.insert",
                "prompt",
                "searchbox",
                "dock.filesystem",
                "dock",
                "dock.debugger",
                "foreign",
                "unknown",
                "panel",
            ]);
            let wrong = broken.classify(&chain).expect("total");
            assert_eq!(wrong.ids, vec!["dock", "panel"], "this is the regression");
            // And the audit catches it with no fixture at all, which is what
            // makes the rule enforced rather than merely tested.
            assert!(broken.audit().iter().any(|e| e.contains("dock.debugger")));
        }

        #[test]
        fn editor_after_unknown_would_leak_ctrl_hjkl_into_insert_mode() {
            let chain = editor_chain(Some(Mode::Insert));

            assert_eq!(
                forest().classify(&chain).expect("total").seal,
                Seal::Barrier
            );

            let broken = reordered(&[
                "editor.completion",
                "prompt",
                "searchbox",
                "dock.filesystem",
                "dock.debugger",
                "dock",
                "foreign",
                "unknown",
                "editor.nav",
                "editor.insert",
                "panel",
            ]);
            let wrong = broken.classify(&chain).expect("total");
            assert_eq!(wrong.ids, vec!["unknown", "panel"]);
            assert_eq!(
                wrong.seal,
                Seal::Open,
                "Ctrl+H would stop being backspace mid-word"
            );
        }
    }
}
