//! Depth-limited scene-tree traversal primitives.
//!
//! Godot's scene tree is the only way to discover editor UI nodes at runtime
//! (there is no global registry of CodeEdits, docks, or panels). This module
//! provides DFS helpers that centralize traversal logic previously duplicated
//! across discovery, buffer navigation, dock search, and window cycling.
//!
//! All helpers take a `max_depth` parameter to cap recursion. Without a depth
//! limit, a full DFS of the editor tree (~3000+ nodes in Godot 4.3) would be
//! measurably slow on every keystroke.
//!
//! # Function summary
//!
//! | Function              | Tests root? | Returns       | Use case                           |
//! |-----------------------|-------------|---------------|------------------------------------|
//! | `find_descendant`     | yes         | first by type | typed node discovery               |
//! | `find_child_of_type`  | no          | first by type | child-only typed search            |
//! | `find_descendant_by`  | yes         | first match   | predicate-based search (class name, visibility, etc.) |
//! | `collect_descendants` | yes         | all matches   | gathering multiple candidates      |

use godot::classes::Node;
use godot::prelude::*;

/// Whether `node` is a Godot control type that accepts keyboard focus in the
/// editor UI (Tree, ItemList, CodeEdit, GraphEdit, RichTextLabel).
///
/// Uses string-based `is_class()` because Godot's class hierarchy is only
/// known at runtime; a typo here silently returns false, so this single
/// source of truth replaces 6+ scattered copies of the same check.
pub(crate) fn is_navigable_control(node: &Gd<Node>) -> bool {
    node.is_class("Tree")
        || node.is_class("ItemList")
        || node.is_class("CodeEdit")
        || node.is_class("GraphEdit")
        || node.is_class("RichTextLabel")
}

/// Maximum DFS depth for editor scene-tree searches.
/// Empirically measured: Godot 4.3 editor max observed depth is 14.
/// The margin to 20 covers custom themes and deeply-nested addon UIs.
pub(crate) const MAX_DISCOVERY_DEPTH: u32 = 20;

/// Depth-limited DFS for the first descendant castable to `T`.
///
/// Tests the root node itself before recursing. `max_depth` counts edges,
/// not nodes: depth 0 tests only the root, depth 1 tests root + its children.
pub(crate) fn find_descendant<T>(node: &Gd<Node>, max_depth: u32) -> Option<Gd<T>>
where
    T: GodotClass + Inherits<Node>,
{
    if let Ok(found) = node.clone().try_cast::<T>() {
        return Some(found);
    }
    if max_depth == 0 {
        return None;
    }
    for child in node.get_children().iter_shared() {
        if let Some(found) = find_descendant::<T>(&child, max_depth - 1) {
            return Some(found);
        }
    }
    None
}

/// Like [`find_descendant`] but skips the root node.
///
/// Use this when the root is known to be a container (e.g. a VBoxContainer)
/// and matching it would be a false positive.
pub(crate) fn find_child_of_type<T>(root: &Gd<Node>, max_depth: u32) -> Option<Gd<T>>
where
    T: GodotClass + Inherits<Node>,
{
    if max_depth == 0 {
        return None;
    }
    for child in root.get_children().iter_shared() {
        if let Ok(typed) = child.clone().try_cast::<T>() {
            return Some(typed);
        }
        if let Some(found) = find_child_of_type::<T>(&child, max_depth - 1) {
            return Some(found);
        }
    }
    None
}

/// Generic depth-limited DFS with a predicate that doubles as an extractor.
///
/// The predicate receives each `Gd<Node>` and returns `Some(R)` to accept
/// (short-circuiting) or `None` to reject and continue. This is the
/// building block for compound conditions that the type-only helpers
/// cannot express (e.g. "visible Control whose class name is X").
///
/// # Examples (conceptual)
///
/// ```ignore
/// // Find a node by Godot class name:
/// find_descendant_by(&root, 15, |node| {
///     node.is_class("SceneTreeDock").then(|| node.clone())
/// });
///
/// // Find a visible Control of a specific class:
/// find_descendant_by(&root, 15, |node| {
///     let control = node.clone().try_cast::<Control>().ok()?;
///     (control.is_visible_in_tree() && node.is_class("Tree")).then_some(control)
/// });
/// ```
pub(crate) fn find_descendant_by<R>(
    node: &Gd<Node>,
    max_depth: u32,
    predicate: &impl Fn(&Gd<Node>) -> Option<R>,
) -> Option<R> {
    if let Some(result) = predicate(node) {
        return Some(result);
    }
    if max_depth == 0 {
        return None;
    }
    for child in node.get_children().iter_shared() {
        if let Some(result) = find_descendant_by(&child, max_depth - 1, predicate) {
            return Some(result);
        }
    }
    None
}

/// Like [`find_descendant_by`] but collects **all** matches instead of
/// short-circuiting on the first.
///
/// Results are appended to `out` in DFS pre-order. The caller provides
/// the `Vec` to allow reuse across repeated scans (e.g. per-frame editor
/// discovery).
///
/// # Examples (conceptual)
///
/// ```ignore
/// // Collect all visible CodeEdit instances:
/// let mut editors = Vec::new();
/// collect_descendants(&root, 20, &mut editors, |node| {
///     let control = node.clone().try_cast::<CodeEdit>().ok()?;
///     control.is_visible_in_tree().then_some(control)
/// });
/// ```
pub(crate) fn collect_descendants<R>(
    node: &Gd<Node>,
    max_depth: u32,
    out: &mut Vec<R>,
    predicate: &impl Fn(&Gd<Node>) -> Option<R>,
) {
    if let Some(result) = predicate(node) {
        out.push(result);
    }
    if max_depth == 0 {
        return;
    }
    for child in node.get_children().iter_shared() {
        collect_descendants(&child, max_depth - 1, out, predicate);
    }
}
