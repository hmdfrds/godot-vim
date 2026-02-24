//! Dock navigation handler.
//!
//! Handles focusing and navigating Godot editor docks (FileSystem, Scene, Inspector, etc.).

#[derive(Debug, Clone, Copy, PartialEq, strum::Display)]
pub enum DockTarget {
    FileSystem,
    Scene,
    Inspector,
    Script,
    Output,
    Editor2D,
    Editor3D,
}

use crate::bridge::godot::names::control;
use godot::classes::{EditorInterface, Node, TabContainer};
use godot::prelude::*;

/// Handles a request to focus a specific dock.
/// Returns the specific internal control that was focused, if any.
/// Focus is deferred to avoid re-entrant borrow panics when called from signal handlers.
pub fn handle_dock_focus(target: DockTarget) -> Option<Gd<godot::classes::Control>> {
    let mut interface = EditorInterface::singleton();

    match target {
        DockTarget::FileSystem => {
            if let Some(dock) = interface.get_file_system_dock() {
                // Make the dock's tab visible first
                make_dock_tab_visible(&dock.clone().upcast());

                // Find the internal Tree or ItemList to focus
                if let Some(focus_target) = find_first_focusable_control(&dock.clone().upcast()) {
                    focus_target
                        .clone()
                        .upcast::<Node>()
                        .call_deferred(control::methods::GRAB_FOCUS, &[]);
                    return Some(focus_target);
                } else {
                    let dock_control = dock.upcast::<godot::classes::Control>();
                    dock_control
                        .clone()
                        .upcast::<Node>()
                        .call_deferred(control::methods::GRAB_FOCUS, &[]);
                    return Some(dock_control);
                }
            }
        }
        DockTarget::Scene => {
            // Scene tree is hard to find directly via API
        }
        DockTarget::Inspector => {
            if let Some(inspector) = interface.get_inspector() {
                make_dock_tab_visible(&inspector.clone().upcast());
                inspector
                    .clone()
                    .upcast::<Node>()
                    .call_deferred(control::methods::GRAB_FOCUS, &[]);
                return Some(inspector.upcast());
            }
        }
        DockTarget::Script => {
            interface.set_main_screen_editor("Script");
        }
        DockTarget::Output => {
            // Output is a bottom panel - find it via node tree traversal
            if let Some(base) = interface.get_base_control() {
                if let Some(output_panel) =
                    find_bottom_panel_by_name(&base.clone().upcast(), "Output")
                {
                    make_dock_tab_visible(&output_panel.clone().upcast());

                    // Find the internal focusable control (RichTextLabel, ItemList, etc)
                    if let Some(focus_target) =
                        find_first_focusable_control(&output_panel.clone().upcast())
                    {
                        focus_target
                            .clone()
                            .upcast::<Node>()
                            .call_deferred(control::methods::GRAB_FOCUS, &[]);
                        return Some(focus_target);
                    } else {
                        // Fall back to the panel itself (focus may silently fail if not focusable).
                        output_panel
                            .clone()
                            .upcast::<Node>()
                            .call_deferred(control::methods::GRAB_FOCUS, &[]);
                        return Some(output_panel);
                    }
                }
            }
        }
        DockTarget::Editor2D => {
            interface.set_main_screen_editor("2D");
        }
        DockTarget::Editor3D => {
            interface.set_main_screen_editor("3D");
        }
    }
    None
}

/// Makes a dock's tab visible by finding its parent TabContainer and selecting the tab.
fn make_dock_tab_visible(control: &Gd<godot::classes::Node>) {
    // Walk up the tree to find a TabContainer parent
    let mut current = control.get_parent();
    while let Some(parent) = current {
        if parent.is_class("TabContainer") {
            if let Ok(mut tab_container) = parent.clone().try_cast::<TabContainer>() {
                // Find which tab index this control is in
                for i in 0..tab_container.get_tab_count() {
                    if let Some(child) = tab_container.get_tab_control(i) {
                        // Check whether the target is this child or one of its descendants.
                        if is_ancestor_of(&child.clone().upcast(), control) {
                            tab_container.set_current_tab(i);
                            return;
                        }
                    }
                }
            }
            return;
        }
        current = parent.get_parent();
    }
}

/// Checks if `ancestor` is an ancestor of `node` (or is the same node).
fn is_ancestor_of(ancestor: &Gd<godot::classes::Node>, node: &Gd<godot::classes::Node>) -> bool {
    if ancestor.instance_id() == node.instance_id() {
        return true;
    }
    let mut current = node.get_parent();
    while let Some(parent) = current {
        if parent.instance_id() == ancestor.instance_id() {
            return true;
        }
        current = parent.get_parent();
    }
    false
}

/// Recursively finds the first Tree or ItemList or RichTextLabel or editor.
pub fn find_first_focusable_control(
    node: &Gd<godot::classes::Node>,
) -> Option<Gd<godot::classes::Control>> {
    // Check specific navigation controls
    if node.is_class("Tree") || node.is_class("ItemList") || node.is_class("RichTextLabel") {
        if let Ok(control) = node.clone().try_cast::<godot::classes::Control>() {
            if control.is_visible_in_tree()
                && control.get_focus_mode() != godot::classes::control::FocusMode::NONE
            {
                return Some(control);
            }
        }
    }

    // Check editors (CodeEdit, TextEdit) explicitly as they are often nested
    if node.is_class("CodeEdit") || node.is_class("TextEdit") {
        if let Ok(control) = node.clone().try_cast::<godot::classes::Control>() {
            if control.is_visible_in_tree()
                && control.get_focus_mode() != godot::classes::control::FocusMode::NONE
            {
                return Some(control);
            }
        }
    }

    for child in node.get_children().iter_shared() {
        if let Some(found) = find_first_focusable_control(&child) {
            return Some(found);
        }
    }
    None
}

/// Finds a bottom panel by name (e.g., "Output", "Debugger").
/// Searches the node tree for a Control with the given name.
fn find_bottom_panel_by_name(
    root: &Gd<godot::classes::Node>,
    name: &str,
) -> Option<Gd<godot::classes::Control>> {
    // Recursively search for a node with the given name
    fn search_node(
        node: &Gd<godot::classes::Node>,
        name: &str,
    ) -> Option<Gd<godot::classes::Control>> {
        let node_name = node.get_name().to_string();

        // Check if this node's name contains the target (e.g., "Output" or "OutputPanel")
        if node_name.contains(name) {
            if let Ok(control) = node.clone().try_cast::<godot::classes::Control>() {
                // Visibility is not checked; the panel may be hidden when the bottom panel is collapsed.
                return Some(control);
            }
        }

        // Search children
        for child in node.get_children().iter_shared() {
            if let Some(found) = search_node(&child, name) {
                return Some(found);
            }
        }
        None
    }

    search_node(root, name)
}
