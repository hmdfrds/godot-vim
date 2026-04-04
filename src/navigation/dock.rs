//! Top-level dock input dispatcher.
//!
//! Routes plain (unmodified) keystrokes to dock navigation handlers based on
//! the focused control's `DockKind`. This gives Vim-style j/k/h/l navigation
//! within Godot's Tree, ItemList, and RichTextLabel dock controls, plus `/`
//! to focus the dock's search box and `ESC` to return to the code editor.
//!
//! Modified keys (Ctrl/Alt/Meta/Shift) always pass through: Ctrl+hjkl is
//! intercepted at a higher priority in `input.rs` for cross-panel navigation.

use godot::classes::{CodeEdit, Control, EditorInterface, InputEventKey, Node};
use godot::global::Key;
use godot::prelude::*;

use super::dock_nav::{
    handle_hierarchy, handle_navigation, HierarchyAction, NavDirection,
};
use super::dock_search::{find_sibling_nav_control, find_sibling_search_box};
use crate::scene_tree::{find_child_of_type, MAX_DISCOVERY_DEPTH};
use super::focus::DockKind;

/// Tri-state result so callers can distinguish "consumed in place" from
/// "consumed and moved focus" — the latter may need additional bookkeeping
/// (e.g., updating the last-focused-editor tracking).
#[derive(Debug)]
pub(crate) enum DockInputResult {
    /// Event consumed — call `set_input_as_handled()`.
    Handled,
    /// Event consumed and focus moved to a different control.
    FocusChanged,
    /// Event not consumed — let Godot's native handling proceed.
    Ignored,
}

impl DockInputResult {
    pub(crate) fn is_consumed(&self) -> bool {
        !matches!(self, Self::Ignored)
    }
}

/// Direction for hjkl dock navigation.
enum DockHjkl {
    Down,
    Up,
    Left,
    Right,
}

/// Check logical keycode first, fall back to physical for non-Latin layouts.
fn dock_hjkl(key_event: &Gd<InputEventKey>) -> Option<DockHjkl> {
    let logical = key_event.get_keycode();
    let physical = key_event.get_physical_keycode();
    hjkl_to_dock(logical).or_else(|| hjkl_to_dock(physical))
}

fn hjkl_to_dock(key: Key) -> Option<DockHjkl> {
    match key {
        Key::J => Some(DockHjkl::Down),
        Key::K => Some(DockHjkl::Up),
        Key::H => Some(DockHjkl::Left),
        Key::L => Some(DockHjkl::Right),
        _ => None,
    }
}

pub(crate) fn handle_dock_input(
    focused: Gd<Control>,
    key_event: &Gd<InputEventKey>,
    dock_kind: DockKind,
) -> DockInputResult {
    log::trace!("dock_input: key={:?} kind={:?}", key_event.get_keycode(), dock_kind);
    // All modified keys pass through. Ctrl+hjkl is already intercepted at
    // Priority 1 in input.rs before this code is reached.
    if key_event.is_ctrl_pressed() || key_event.is_alt_pressed()
        || key_event.is_meta_pressed() || key_event.is_shift_pressed()
    {
        return DockInputResult::Ignored;
    }

    // hjkl and / use logical-then-physical fallback for non-Latin layout support.
    // Enter and Esc use logical keycode only — they are special keys with
    // layout-independent keycodes.
    if let Some(direction) = dock_hjkl(key_event) {
        return match direction {
            DockHjkl::Down => {
                if handle_navigation(&focused, NavDirection::Next, 0) {
                    DockInputResult::Handled
                } else {
                    DockInputResult::Ignored
                }
            }
            DockHjkl::Up => {
                if handle_navigation(&focused, NavDirection::Prev, 0) {
                    DockInputResult::Handled
                } else {
                    DockInputResult::Ignored
                }
            }
            DockHjkl::Left => {
                if matches!(dock_kind, DockKind::Tree)
                    && handle_hierarchy(&focused, HierarchyAction::Collapse)
                {
                    DockInputResult::Handled
                } else {
                    DockInputResult::Ignored
                }
            }
            DockHjkl::Right => {
                if matches!(dock_kind, DockKind::Tree)
                    && handle_hierarchy(&focused, HierarchyAction::Expand)
                {
                    DockInputResult::Handled
                } else {
                    DockInputResult::Ignored
                }
            }
        };
    }

    let keycode = key_event.get_keycode();
    let physical = key_event.get_physical_keycode();
    match keycode {
        Key::SLASH => handle_slash(&focused),
        Key::ENTER => handle_enter(&focused, dock_kind),
        Key::ESCAPE => handle_escape_from_dock(),
        _ if physical == Key::SLASH => handle_slash(&focused),
        _ => DockInputResult::Ignored,
    }
}

/// Only intercepts ESC and Enter from dock search boxes — all other keys
/// pass through for normal typing. Both keys return focus to the sibling
/// nav control (Tree/ItemList), preserving the search filter text.
pub(crate) fn handle_search_input(
    line_edit: &Gd<godot::classes::LineEdit>,
    key_event: &Gd<InputEventKey>,
) -> DockInputResult {
    if key_event.is_ctrl_pressed() || key_event.is_alt_pressed() || key_event.is_meta_pressed() {
        return DockInputResult::Ignored;
    }

    match key_event.get_keycode() {
        Key::ESCAPE | Key::ENTER => {
            let control: Gd<Control> = line_edit.clone().upcast();
            if let Some(nav) = find_sibling_nav_control(&control) {
                defer_grab_focus(&nav);
                DockInputResult::FocusChanged
            } else {
                // No sibling nav control — fall back to the script editor.
                handle_escape_from_dock()
            }
        }
        _ => DockInputResult::Ignored,
    }
}

/// `/` — Vim-style "search": focus the dock's filter/search LineEdit.
fn handle_slash(focused: &Gd<Control>) -> DockInputResult {
    if let Some(search_box) = find_sibling_search_box(focused) {
        defer_grab_focus(&search_box);
        let mut node: Gd<Node> = search_box.clone().upcast();
        node.call_deferred("select_all", &[]);
        DockInputResult::FocusChanged
    } else {
        DockInputResult::Ignored
    }
}

/// `Enter` — emit activation signals to open the selected item.
///
/// For ItemList, both `item_selected` and `item_activated` are emitted because
/// some Godot editor docks listen to one, some to the other (e.g., the script
/// list dock uses `item_activated` to open scripts).
fn handle_enter(focused: &Gd<Control>, dock_kind: DockKind) -> DockInputResult {
    match dock_kind {
        DockKind::Tree => {
            let mut control = focused.clone();
            control.emit_signal("item_activated", &[]);
            DockInputResult::Handled
        }
        DockKind::ItemList => {
            let Ok(mut list) = focused.clone().try_cast::<godot::classes::ItemList>() else {
                return DockInputResult::Ignored;
            };
            let selected = list.get_selected_items();
            if !selected.is_empty() {
                let idx = selected.get(0).unwrap_or(0);
                let mut control = focused.clone();
                control.emit_signal("item_selected", &[Variant::from(idx)]);
                control.emit_signal("item_activated", &[Variant::from(idx)]);
                DockInputResult::Handled
            } else {
                DockInputResult::Ignored
            }
        }
        DockKind::RichTextLabel => DockInputResult::Ignored,
    }
}

/// Deferred because immediate `grab_focus()` during input processing can be
/// swallowed by Godot's event dispatch loop.
fn defer_grab_focus(target: &Gd<impl Inherits<Node>>) {
    target.clone().upcast::<Node>().call_deferred("grab_focus", &[]);
}

/// `ESC` — return focus to the script editor's CodeEdit.
///
/// Tries CodeEdit first (the primary editing surface), then TextEdit (shader
/// editors), then the editor container itself as a last resort.
fn handle_escape_from_dock() -> DockInputResult {
    let interface = EditorInterface::singleton();
    let Some(script_editor) = interface.get_script_editor() else {
        return DockInputResult::Ignored;
    };
    let Some(current) = script_editor.get_current_editor() else {
        log::debug!("dock_escape: no current editor found");
        return DockInputResult::Ignored;
    };

    let root = current.clone().upcast::<Node>();

    if let Some(code_edit) = find_child_of_type::<CodeEdit>(&root, MAX_DISCOVERY_DEPTH) {
        defer_grab_focus(&code_edit);
        return DockInputResult::FocusChanged;
    }
    if let Some(text_edit) = find_child_of_type::<godot::classes::TextEdit>(&root, MAX_DISCOVERY_DEPTH) {
        defer_grab_focus(&text_edit);
        return DockInputResult::FocusChanged;
    }

    let control = current.upcast::<Control>();
    defer_grab_focus(&control);
    DockInputResult::FocusChanged
}
