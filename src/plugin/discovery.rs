//! Discovers the active CodeEdit in Godot's ScriptEditor via scene-tree
//! traversal.

use godot::classes::{CodeEdit, Control, EditorInterface};
use godot::prelude::*;

use crate::scene_tree::{find_descendant, MAX_DISCOVERY_DEPTH};

/// Find the CodeEdit for the currently active script tab.
///
/// Matches any CodeEdit descendant (shader editors, resource editors, custom
/// addon editors) -- Vim keybindings are useful in all text editing contexts.
pub(super) fn find_active_code_edit() -> Option<Gd<CodeEdit>> {
    let interface = EditorInterface::singleton();
    let script_editor = interface.get_script_editor()?;
    let current_editor = script_editor.get_current_editor()?;
    find_descendant::<CodeEdit>(&current_editor.upcast(), MAX_DISCOVERY_DEPTH)
}

/// Find a CodeEdit descendant starting from a focused Control node.
pub(super) fn find_code_edit_from_control(control: &Gd<Control>) -> Option<Gd<CodeEdit>> {
    find_descendant::<CodeEdit>(&control.clone().upcast(), MAX_DISCOVERY_DEPTH)
}
