//! Intra-dock navigation handler.
//!
//! Handles Vim-like navigation (hjkl) within Godot's Tree and ItemList controls.
//! This is the top-level input handler; navigation logic lives in `focus_nav`,
//! and search/find helpers live in `focus_search`.

use super::super::window::nav::{
    self as window_nav, NavDirection as WindowNavDir, WindowNavResult,
};
use crate::bridge::godot::names::{control, item_list, line_edit, tree};
use godot::classes::{Control, EditorInterface, InputEvent, InputEventKey, Node};
use godot::global::Key;
use godot::prelude::*;

use super::focus_nav::{handle_hierarchy, handle_navigation, HierarchyAction, NavDirection};
use super::focus_search::{find_child_recursive_type, find_sibling_search_box};

// Re-exports for external callers
pub use super::focus_search::{find_child_recursive_type_control, find_sibling_nav_control};

#[derive(Debug)]
pub enum DockInputResult {
    Ignored,
    Handled,
    Focused(Gd<Control>),
}

/// Handles global input for dock navigation.
/// Returns `true` if the event was consumed.
pub fn handle_dock_input(focused_control: Gd<Control>, event: Gd<InputEvent>) -> DockInputResult {
    // Only handle key presses
    let Some(key_event) = event.try_cast::<InputEventKey>().ok() else {
        return DockInputResult::Ignored;
    };

    if !key_event.is_pressed() {
        return DockInputResult::Ignored;
    }

    // Check for Window Navigation (Ctrl + h/j/k/l)
    if key_event.is_ctrl_pressed() && !key_event.is_alt_pressed() && !key_event.is_meta_pressed() {
        let dir = match key_event.get_keycode() {
            Key::J => Some(WindowNavDir::Next),
            Key::K => Some(WindowNavDir::Prev),
            Key::H => Some(WindowNavDir::Left),
            Key::L => Some(WindowNavDir::Right),
            _ => None,
        };
        if let Some(direction) = dir {
            return match window_nav::handle_window_nav(&focused_control, direction) {
                WindowNavResult::Ignored => DockInputResult::Ignored,
                WindowNavResult::Focused(c) => DockInputResult::Focused(c),
            };
        }
        return DockInputResult::Ignored;
    }

    // Default Dock Navigation (No modifiers)
    if key_event.is_ctrl_pressed() || key_event.is_alt_pressed() || key_event.is_meta_pressed() {
        return DockInputResult::Ignored;
    }

    match key_event.get_keycode() {
        Key::J | Key::K | Key::H | Key::L => {
            let direction = match key_event.get_keycode() {
                Key::J => NavDirection::Next,
                Key::K => NavDirection::Prev,
                Key::H => {
                    // Collapse
                    if handle_hierarchy(&focused_control, HierarchyAction::Collapse) {
                        return DockInputResult::Handled;
                    }
                    return DockInputResult::Ignored;
                }
                Key::L => {
                    // Expand
                    if handle_hierarchy(&focused_control, HierarchyAction::Expand) {
                        return DockInputResult::Handled;
                    }
                    return DockInputResult::Ignored;
                }
                _ => unreachable!(),
            };

            if handle_navigation(&focused_control, direction) {
                DockInputResult::Handled
            } else {
                DockInputResult::Ignored
            }
        }
        Key::SLASH => {
            // Focus search bar if present in the same dock
            // Deferred to avoid re-entrant borrow panic.
            if let Some(search_box) = find_sibling_search_box(&focused_control) {
                search_box
                    .clone()
                    .upcast::<Node>()
                    .call_deferred(control::methods::GRAB_FOCUS, &[]);
                search_box
                    .clone()
                    .upcast::<Node>()
                    .call_deferred(line_edit::methods::SELECT_ALL, &[]);
                return DockInputResult::Focused(search_box.upcast());
            }
            DockInputResult::Ignored
        }
        Key::ENTER => {
            if focused_control.is_class("Tree") {
                let mut tree = focused_control.clone();
                tree.emit_signal(tree::signals::ITEM_ACTIVATED, &[]);
                return DockInputResult::Handled;
            } else if focused_control.is_class("ItemList") {
                let mut list = focused_control.clone().cast::<godot::classes::ItemList>();
                let selected = list.get_selected_items();
                if !selected.is_empty() {
                    let idx = selected.get(0).unwrap_or(0);
                    let mut control = focused_control.clone();
                    control.emit_signal(item_list::signals::ITEM_SELECTED, &[Variant::from(idx)]);
                    control.emit_signal(tree::signals::ITEM_ACTIVATED, &[Variant::from(idx)]);
                    return DockInputResult::Handled;
                }
            } else if focused_control.is_class("LineEdit") {
                if let Some(nav_control) = find_sibling_nav_control(&focused_control) {
                    nav_control
                        .clone()
                        .upcast::<Node>()
                        .call_deferred(control::methods::GRAB_FOCUS, &[]);
                    return DockInputResult::Focused(nav_control);
                }
            }
            DockInputResult::Ignored
        }
        Key::ESCAPE => {
            // Skip if focus is already on an editor (CodeEdit, TextEdit) - VimController handles it
            if focused_control.is_class("CodeEdit") || focused_control.is_class("TextEdit") {
                return DockInputResult::Ignored;
            }

            // If focus is on a LineEdit (search box), ESC returns focus to the Tree/List.
            if focused_control.is_class("LineEdit") {
                if let Some(nav_control) = find_sibling_nav_control(&focused_control) {
                    nav_control
                        .clone()
                        .upcast::<Node>()
                        .call_deferred(control::methods::GRAB_FOCUS, &[]);
                    return DockInputResult::Focused(nav_control);
                }
            }

            // No search box focused; return focus to the main editor.
            let interface = EditorInterface::singleton();
            if let Some(script_editor) = interface.get_script_editor() {
                if let Some(current) = script_editor.get_current_editor() {
                    let root = current.clone().upcast::<Node>();
                    if let Some(code_edit) =
                        find_child_recursive_type::<godot::classes::CodeEdit>(&root)
                    {
                        code_edit
                            .clone()
                            .upcast::<Node>()
                            .call_deferred(control::methods::GRAB_FOCUS, &[]);
                        return DockInputResult::Focused(code_edit.upcast());
                    }
                    if let Some(text_edit) =
                        find_child_recursive_type::<godot::classes::TextEdit>(&root)
                    {
                        text_edit
                            .clone()
                            .upcast::<Node>()
                            .call_deferred(control::methods::GRAB_FOCUS, &[]);
                        return DockInputResult::Focused(text_edit.upcast());
                    }

                    let control = current.upcast::<Control>();
                    control
                        .clone()
                        .upcast::<Node>()
                        .call_deferred(control::methods::GRAB_FOCUS, &[]);
                    return DockInputResult::Focused(control);
                }
            }
            DockInputResult::Ignored
        }
        _ => DockInputResult::Ignored,
    }
}
