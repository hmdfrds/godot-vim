//! Scene tree search helpers for dock navigation.
//!
//! Finds search/filter LineEdits and sibling navigable controls within dock
//! boundaries. All searches climb upward from the focused control until a dock
//! boundary is hit (see `is_dock_boundary`), preventing cross-dock matches.

use godot::classes::{Control, ItemList, LineEdit, Node, RichTextLabel, Tree};
use godot::prelude::*;

use crate::scene_tree::{find_child_of_type, MAX_DISCOVERY_DEPTH};

/// Maximum ancestor levels to climb before giving up. 8 is enough to reach
/// any dock root from its deepest navigable child in the Godot editor.
const MAX_CLIMB_DEPTH: usize = 8;

/// Dock boundary detection: stops upward searches from crossing into
/// adjacent docks.
///
/// Uses `get_class().contains("Dock")` rather than a hardcoded allowlist so
/// that all editor dock types (FileSystemDock, SceneTreeDock, NodeDock,
/// InspectorDock, ImportDock, etc.) are automatically covered without
/// maintenance. `EditorHelp` and `TabContainer` are non-dock boundaries
/// that also separate logical navigation regions.
fn is_dock_boundary(node: &Gd<Node>) -> bool {
    let class_name = node.get_class().to_string();
    class_name.contains("Dock")
        || node.is_class("EditorHelp")
        || node.is_class("TabContainer")
}

/// Climb ancestors to find the containing dock, then search downward for a
/// LineEdit that looks like a search/filter box (scored by placeholder text
/// and node name).
pub(super) fn find_sibling_search_box(node: &Gd<Control>) -> Option<Gd<LineEdit>> {
    let mut current = node.clone().upcast::<Node>();

    for _ in 0..MAX_CLIMB_DEPTH {
        let Some(parent) = current.get_parent() else {
            break;
        };

        if let Some(search) = find_visible_search_box(&parent) {
            return Some(search);
        }

        // Stop at the dock boundary — searching above it would match
        // search boxes in adjacent docks.
        if is_dock_boundary(&parent) {
            return None;
        }

        current = parent;
    }
    None
}

/// Find a sibling navigable control (Tree/ItemList/RichTextLabel) within the
/// same dock boundary. Used to return focus from a search box back to the
/// dock's main content after ESC/Enter.
pub(super) fn find_sibling_nav_control(node: &Gd<Control>) -> Option<Gd<Control>> {
    let mut current = node.clone().upcast::<Node>();

    for _ in 0..MAX_CLIMB_DEPTH {
        let Some(parent) = current.get_parent() else {
            break;
        };

        if let Some(tree) = find_child_of_type::<Tree>(&parent, MAX_DISCOVERY_DEPTH) {
            if tree.is_visible_in_tree() {
                return Some(tree.upcast());
            }
        }
        if let Some(list) = find_child_of_type::<ItemList>(&parent, MAX_DISCOVERY_DEPTH) {
            if list.is_visible_in_tree() {
                return Some(list.upcast());
            }
        }
        if let Some(label) = find_child_of_type::<RichTextLabel>(&parent, MAX_DISCOVERY_DEPTH) {
            if label.is_visible_in_tree() {
                return Some(label.upcast());
            }
        }

        if is_dock_boundary(&parent) {
            return None;
        }

        current = parent;
    }
    None
}

/// Select the best search/filter LineEdit among all visible candidates.
///
/// Scoring heuristic (lower = better): placeholder text containing
/// "filter"/"search" beats node names containing those strings, which beats
/// any other visible LineEdit. This handles all known Godot editor dock
/// layouts without hardcoding specific node paths.
fn find_visible_search_box(root: &Gd<Node>) -> Option<Gd<LineEdit>> {
    let mut candidates = Vec::new();
    find_search_candidates_recursive(root, &mut candidates, 0);

    candidates.sort_by_key(|search| {
        let name = search.get_name().to_string();
        let placeholder = search.get_placeholder().to_string().to_lowercase();

        if placeholder.contains("filter") || placeholder.contains("search") {
            return 0;
        }
        if name.contains("codeContext") || name.contains("Filter") || name.contains("Search") {
            return 1;
        }
        2
    });

    candidates.into_iter().next()
}

fn find_search_candidates_recursive(
    node: &Gd<Node>,
    candidates: &mut Vec<Gd<LineEdit>>,
    depth: usize,
) {
    if depth >= MAX_DISCOVERY_DEPTH as usize {
        return;
    }
    for child in node.get_children().iter_shared() {
        if let Ok(search) = child.clone().try_cast::<LineEdit>() {
            if search.is_visible_in_tree() {
                candidates.push(search);
            }
        }
        find_search_candidates_recursive(&child, candidates, depth + 1);
    }
}
