//! Search helpers for dock navigation.
//!
//! Functions for finding search boxes, navigation controls, and
//! recursive child lookups within Godot's scene tree.

use godot::classes::{Control, ItemList, Node, RichTextLabel, Tree};
use godot::prelude::*;

const MAX_RECURSIVE_DEPTH: usize = 20;

/// Start searching up the tree to find the Dock root (VBox/HBox handling the dock),
/// then search down for a LineEdit which acts as the search box.
pub(super) fn find_sibling_search_box(node: &Gd<Control>) -> Option<Gd<godot::classes::LineEdit>> {
    let mut current = node.clone().upcast::<Node>();

    // Go up a few levels to find the common parent (usually VBox within Dock)
    for _ in 0..8 {
        // Limit hierarchy climb depth
        if let Some(parent) = current.get_parent() {
            // Check for Dock Boundaries to prevent leaking into other docks
            let is_fs_dock = parent.is_class("FileSystemDock");
            let is_scene_dock = parent.is_class("SceneTreeDock");
            let is_help_dock = parent.is_class("EditorHelp");
            let is_boundary = is_fs_dock || is_scene_dock || is_help_dock;

            // Search children of this parent for a *VISIBLE* LineEdit using heuristic
            if let Some(search) = find_visible_search_box(&parent) {
                return Some(search);
            }

            // Stop climbing at a known dock boundary (e.g. FileSystemDock) to avoid
            // matching a sibling dock's search box via the parent TabContainer.
            if is_boundary {
                log::debug!(
                    "Hit boundary class={}, stopping search climb",
                    parent.get_class()
                );
                return None;
            }

            current = parent;
        } else {
            break;
        }
    }
    None
}

/// Recursively find the best visible LineEdit that acts as a search/filter box.
fn find_visible_search_box(root: &Gd<Node>) -> Option<Gd<godot::classes::LineEdit>> {
    let mut candidates = Vec::new();
    find_search_candidates_recursive(root, &mut candidates);

    // Score and pick the best candidate
    // Priority:
    // 1. Placeholder contains "filter" or "search" (strongest)
    // 2. Name contains "Filter" or "Search" or "codeContext"
    // 3. Last resort: empty placeholder in a VBox/HBox (weakest, likely path bar)

    candidates.sort_by_key(|search| {
        let name = search.get_name().to_string();
        let placeholder = search.get_placeholder().to_string().to_lowercase();

        if placeholder.contains("filter") || placeholder.contains("search") {
            return 0; // Top Priority
        }
        if name.contains("codeContext") || name.contains("Filter") || name.contains("Search") {
            return 1;
        }
        2 // Likely a path bar or generic input
    });

    candidates.into_iter().next()
}

fn find_search_candidates_recursive(
    node: &Gd<Node>,
    candidates: &mut Vec<Gd<godot::classes::LineEdit>>,
) {
    find_search_candidates_recursive_depth(node, candidates, 0);
}

fn find_search_candidates_recursive_depth(
    node: &Gd<Node>,
    candidates: &mut Vec<Gd<godot::classes::LineEdit>>,
    depth: usize,
) {
    if depth >= MAX_RECURSIVE_DEPTH {
        return;
    }
    for child in node.get_children().iter_shared() {
        if let Ok(search) = child.clone().try_cast::<godot::classes::LineEdit>() {
            if search.is_visible_in_tree() {
                candidates.push(search);
            }
        }

        find_search_candidates_recursive_depth(&child, candidates, depth + 1);
    }
}

/// Find a sibling Tree or ItemList to return focus to.
pub fn find_sibling_nav_control(node: &Gd<Control>) -> Option<Gd<Control>> {
    let mut current = node.clone().upcast::<Node>();

    // Go up to common parent
    for _ in 0..8 {
        if let Some(parent) = current.get_parent() {
            if let Some(tree) = find_child_recursive_type::<Tree>(&parent) {
                if tree.is_visible_in_tree() {
                    return Some(tree.upcast());
                }
            }
            if let Some(list) = find_child_recursive_type::<ItemList>(&parent) {
                if list.is_visible_in_tree() {
                    return Some(list.upcast());
                }
            }
            if let Some(label) = find_child_recursive_type::<RichTextLabel>(&parent) {
                if label.is_visible_in_tree() {
                    return Some(label.upcast());
                }
            }
            current = parent;
        } else {
            break;
        }
    }
    None
}

pub fn find_child_recursive_type_control<T: GodotClass + Inherits<Control> + Inherits<Node>>(
    root: &Gd<Node>,
) -> Option<Gd<T>> {
    find_child_recursive_type::<T>(root)
}

pub(crate) fn find_child_recursive_type<T: GodotClass + Inherits<Node>>(
    root: &Gd<Node>,
) -> Option<Gd<T>> {
    for child in root.get_children().iter_shared() {
        if let Ok(typed) = child.clone().try_cast::<T>() {
            return Some(typed);
        }
        if let Some(found) = find_child_recursive_type::<T>(&child) {
            return Some(found);
        }
    }
    None
}
