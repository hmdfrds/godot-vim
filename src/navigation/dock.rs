//! The dock executors.
//!
//! Vim-style j/k/h/l navigation within Godot's Tree, ItemList and
//! RichTextLabel dock controls, plus `/` to focus the dock's search box and
//! `ESC` to return to the code editor. Every one of these is reached by NAME
//! through an `ActionSpec`, resolved from the `dock` surface's binding trie —
//! the dispatcher does not know this file exists.
//!
//! # Where the keyset is now asserted
//!
//! This file used to carry P0's characterization suite: two hand-written
//! decision tables (`dock_action_for`, `search_action_for`) and their probe
//! walks, with no production caller, kept alive as an oracle for the
//! dispatcher cutover. The cutover shipped and was verified against the
//! pre-P0 dispatcher, so the oracles are gone. The behaviours they pinned are
//! stated against the live table instead — including the two asymmetries a
//! unified dispatcher is most tempted to erase, which are now
//! `resolve::a_dock_binds_no_modified_key_of_its_own` and
//! `resolve::a_filter_box_swallows_typing_and_breaks_its_seal_only_for_a_chord`.

use godot::classes::{CodeEdit, Control, EditorInterface, Node};
use godot::prelude::*;

use super::dock_search::{find_sibling_nav_control, find_sibling_search_box};
use crate::scene_tree::{find_child_of_type, MAX_DISCOVERY_DEPTH};

/// Retained so every existing call site compiles unchanged.
///
/// A `type` alias DOES resolve variants (RFC 2338, Rust 1.37+), which is why
/// `DockInputResult::Declined` still works here — but it cannot *rename* one,
/// which is why the `Ignored` → `Declined` rename had to land first.
pub(crate) use crate::actions::outcome::Outcome as DockInputResult;

/// The dock widgets whose *signal contracts* differ.
///
/// Moved here when `focus.rs` was deleted rather than dying with
/// `FocusContext`, because the distinction is real at the Godot API level and
/// not merely a dispatch convenience: `Tree::item_activated` takes **no**
/// parameters while `ItemList::item_activated` takes an index
/// (godot `scene/gui/tree.cpp:7534` vs `scene/gui/item_list.cpp:2486`).
/// Emitting the wrong arity is not a no-op — it is
/// `CALL_ERROR_TOO_MANY_ARGUMENTS`, and the editor's handler does not run.
///
/// It is NOT a classification: which surface a control belongs to is the
/// binding plane's question, answered by `Caps` and the surface forest.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DockKind {
    Tree,
    ItemList,
    RichTextLabel,
}

/// Classify a control into the dock kind whose signal contract it follows.
///
/// The pure half, taking the class probe rather than the control. Split out so
/// that this table and [`crate::actions::caps::Caps::of_control`] — two
/// hand-maintained lists over the same three widget classes — can be held to
/// one agreement property without a Godot runtime. A class in one and not the
/// other manufactures a silent dead key: `Caps` grants `VNAV`, so `:panelmap`
/// reports `godotvim.item.next` as eligible, and then `dock_kind_of` answers
/// `None` and the body declines.
pub(crate) fn dock_kind_for(is_class: impl Fn(&str) -> bool) -> Option<DockKind> {
    if is_class("Tree") {
        Some(DockKind::Tree)
    } else if is_class("ItemList") {
        Some(DockKind::ItemList)
    } else if is_class("RichTextLabel") {
        Some(DockKind::RichTextLabel)
    } else {
        None
    }
}

/// Classify a focused control into the dock kind whose signal contract it
/// follows. `Node::is_class` walks the inheritance chain, which is what makes
/// `FileSystemTree` answer `Tree`.
pub(crate) fn dock_kind_of(control: &Gd<Control>) -> Option<DockKind> {
    let node = control.clone().upcast::<Node>();
    dock_kind_for(|class| node.is_class(class))
}

/// Leave a dock's filter box, keeping the filter text.
///
/// The executor behind `godotvim.search.accept`, reached from the `searchbox`
/// surface by `<CR>` and `<Esc>` — both of which tolerate Shift, which is why
/// those two rules and only those two carry the `<shift>` flag.
///
/// Escape does **not** clear the filter; it returns focus to the sibling nav
/// control with the text intact, and falls back to the script editor when the
/// dock has no nav control to return to.
pub(crate) fn leave_search(focused: &Gd<Control>) -> DockInputResult {
    if let Some(nav) = find_sibling_nav_control(focused) {
        defer_grab_focus(&nav);
        DockInputResult::FocusChanged
    } else {
        // No sibling nav control — fall back to the script editor.
        handle_escape_from_dock()
    }
}

/// `/` — Vim-style "search": focus the dock's filter/search LineEdit.
pub(crate) fn handle_slash(focused: &Gd<Control>) -> DockInputResult {
    if let Some(search_box) = find_sibling_search_box(focused) {
        defer_grab_focus(&search_box);
        let mut node: Gd<Node> = search_box.clone().upcast();
        node.call_deferred("select_all", &[]);
        DockInputResult::FocusChanged
    } else {
        DockInputResult::Declined
    }
}

/// `Enter` — emit activation signals to open the selected item.
///
/// For ItemList, both `item_selected` and `item_activated` are emitted because
/// some Godot editor docks listen to one, some to the other (e.g., the script
/// list dock uses `item_activated` to open scripts).
pub(crate) fn handle_enter(focused: &Gd<Control>, dock_kind: DockKind) -> DockInputResult {
    match dock_kind {
        DockKind::Tree => {
            let mut control = focused.clone();
            control.emit_signal("item_activated", &[]);
            DockInputResult::Handled
        }
        DockKind::ItemList => {
            let Ok(list) = focused.clone().try_cast::<godot::classes::ItemList>() else {
                return DockInputResult::Declined;
            };
            let selected = list.get_selected_items();
            if !selected.is_empty() {
                let idx = selected.get(0).unwrap_or(0);
                let mut control = focused.clone();
                control.emit_signal("item_selected", &[Variant::from(idx)]);
                control.emit_signal("item_activated", &[Variant::from(idx)]);
                DockInputResult::Handled
            } else {
                DockInputResult::Declined
            }
        }
        DockKind::RichTextLabel => DockInputResult::Declined,
    }
}

/// Deferred because immediate `grab_focus()` during input processing can be
/// swallowed by Godot's event dispatch loop.
fn defer_grab_focus(target: &Gd<impl Inherits<Node>>) {
    target
        .clone()
        .upcast::<Node>()
        .call_deferred("grab_focus", &[]);
}

/// `ESC` — return focus to the script editor's CodeEdit.
///
/// Tries CodeEdit first (the primary editing surface), then TextEdit (shader
/// editors), then the editor container itself as a last resort.
pub(crate) fn handle_escape_from_dock() -> DockInputResult {
    let interface = EditorInterface::singleton();
    let Some(script_editor) = interface.get_script_editor() else {
        return DockInputResult::Declined;
    };
    let Some(current) = script_editor.get_current_editor() else {
        log::debug!("dock_escape: no current editor found");
        return DockInputResult::Declined;
    };

    let root = current.clone().upcast::<Node>();

    if let Some(code_edit) = find_child_of_type::<CodeEdit>(&root, MAX_DISCOVERY_DEPTH) {
        defer_grab_focus(&code_edit);
        return DockInputResult::FocusChanged;
    }
    if let Some(text_edit) =
        find_child_of_type::<godot::classes::TextEdit>(&root, MAX_DISCOVERY_DEPTH)
    {
        defer_grab_focus(&text_edit);
        return DockInputResult::FocusChanged;
    }

    let control = current.upcast::<Control>();
    defer_grab_focus(&control);
    DockInputResult::FocusChanged
}
