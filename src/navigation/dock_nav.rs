//! Vim-style navigation within individual dock controls.
//!
//! Provides j/k movement and h/l hierarchy expand/collapse for Tree, ItemList,
//! and RichTextLabel. The `NavigableItem` trait abstracts over Godot's
//! per-control navigation APIs so the core loop is widget-agnostic.

use godot::classes::{Control, ItemList, RichTextLabel, Tree};
use godot::prelude::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum NavDirection {
    Next,
    Prev,
}

#[derive(Debug, Clone, Copy)]
pub(super) enum HierarchyAction {
    Expand,
    Collapse,
}

/// Abstracts over Godot's per-widget navigation APIs so `find_navigable_target`
/// is a single generic loop regardless of whether we're in a Tree or ItemList.
trait NavigableItem: Clone {
    fn nav_next_visible(&mut self) -> Option<Self>;
    fn nav_prev_visible(&mut self) -> Option<Self>;
    fn nav_is_selectable(&self) -> bool;
}

/// Walk from `start` in the given direction, skipping non-selectable items
/// (e.g., Tree separator rows). Bounded to prevent infinite loops if the
/// widget's visibility chain is cyclic or broken.
fn find_navigable_target<T: NavigableItem>(start: T, direction: NavDirection) -> Option<T> {
    let mut current = start;
    const MAX_ATTEMPTS: usize = 1000;

    for _ in 0..MAX_ATTEMPTS {
        let next = match direction {
            NavDirection::Next => current.nav_next_visible(),
            NavDirection::Prev => current.nav_prev_visible(),
        };

        match next {
            Some(item) if item.nav_is_selectable() => return Some(item),
            Some(item) => current = item,
            None => return None,
        }
    }
    None
}

impl NavigableItem for Gd<godot::classes::TreeItem> {
    fn nav_next_visible(&mut self) -> Option<Self> {
        self.get_next_visible()
    }

    fn nav_prev_visible(&mut self) -> Option<Self> {
        self.get_prev_visible()
    }

    fn nav_is_selectable(&self) -> bool {
        self.is_selectable(0)
    }
}

/// Wrappers for Tree methods that require `call()` or have confusing
/// signatures in the gdext bindings.
trait TreeExt {
    fn safe_scroll_to_item(&mut self, item: &Gd<godot::classes::TreeItem>);
    fn safe_select(&mut self, item: &Gd<godot::classes::TreeItem>);
    fn safe_is_root_hidden(&self) -> bool;
}

impl TreeExt for Gd<Tree> {
    fn safe_scroll_to_item(&mut self, item: &Gd<godot::classes::TreeItem>) {
        // `scroll_to_item` is not exposed in gdext's typed API; use `call()`.
        self.call("scroll_to_item", &[item.to_variant()]);
    }

    fn safe_select(&mut self, item: &Gd<godot::classes::TreeItem>) {
        // Column 0 — all editor dock Trees are single-column.
        self.set_selected(item, 0);
    }

    fn safe_is_root_hidden(&self) -> bool {
        self.is_root_hidden()
    }
}

/// Bounded recursion: if the focused control isn't directly navigable (e.g.,
/// a VBoxContainer wrapping a Tree), we search its children. This limit
/// prevents stack overflow on unexpectedly deep nesting.
const MAX_NAV_DEPTH: u32 = 3;

pub(super) fn handle_navigation(control: &Gd<Control>, direction: NavDirection, depth: u32) -> bool {
    if depth >= MAX_NAV_DEPTH {
        return false;
    }

    if control.is_class("Tree") {
        let Ok(tree) = control.clone().try_cast::<Tree>() else { return false; };
        handle_tree_nav(tree, direction)
    } else if control.is_class("ItemList") {
        let Ok(list) = control.clone().try_cast::<ItemList>() else { return false; };
        handle_item_list_nav(list, direction)
    } else if control.is_class("RichTextLabel") {
        let Ok(label) = control.clone().try_cast::<RichTextLabel>() else { return false; };
        handle_richtextlabel_nav(label, direction)
    } else if let Some(target) = find_best_nav_target(control, 0) {
        handle_navigation(&target, direction, depth + 1)
    } else {
        false
    }
}

pub(super) fn handle_hierarchy(control: &Gd<Control>, action: HierarchyAction) -> bool {
    if control.is_class("Tree") {
        let Ok(tree) = control.clone().try_cast::<Tree>() else { return false; };
        handle_tree_hierarchy(tree, action)
    } else {
        false
    }
}

/// Recursively find a navigable child (Tree/ItemList/RichTextLabel) within
/// a non-navigable container. Godot's editor docks often wrap the actual
/// navigable widget inside several layers of layout containers.
fn find_best_nav_target(root: &Gd<Control>, depth: u32) -> Option<Gd<Control>> {
    const MAX_NAV_DEPTH: u32 = 10;
    if depth >= MAX_NAV_DEPTH {
        return None;
    }

    for child in root.get_children().iter_shared() {
        if let Ok(control) = child.clone().try_cast::<Control>() {
            if (control.is_class("Tree")
                || control.is_class("ItemList")
                || control.is_class("RichTextLabel"))
                && control.is_visible_in_tree()
            {
                return Some(control);
            }
            if let Some(found) = find_best_nav_target(&control, depth + 1) {
                return Some(found);
            }
        }
    }
    None
}

fn handle_tree_nav(mut tree: Gd<Tree>, direction: NavDirection) -> bool {
    let Some(selected) = tree.get_selected() else {
        // Bootstrap selection: when nothing is selected (e.g., dock just became
        // visible), pick the first visible item. If the root is hidden (common
        // in SceneTree dock), skip it — selecting a hidden root is a no-op that
        // makes the Tree appear unresponsive.
        if let Some(mut root) = tree.get_root() {
            let start = if tree.safe_is_root_hidden() {
                root.get_next_visible()
            } else {
                Some(root)
            };
            if let Some(item) = start {
                tree.safe_select(&item);
                tree.safe_scroll_to_item(&item);
                tree.queue_redraw();
                return true;
            }
        }
        return false;
    };

    if let Some(target) = find_navigable_target(selected, direction) {
        tree.safe_select(&target);
        tree.safe_scroll_to_item(&target);
        tree.queue_redraw();
        true
    } else {
        false
    }
}

/// Mirrors NERDTree/netrw behavior: `l` expands or descends into first child,
/// `h` collapses or ascends to parent.
fn handle_tree_hierarchy(mut tree: Gd<Tree>, action: HierarchyAction) -> bool {
    let Some(mut selected) = tree.get_selected() else {
        return false;
    };

    match action {
        HierarchyAction::Expand => {
            if selected.is_collapsed() {
                selected.set_collapsed(false);
                true
            } else if selected.get_child_count() > 0 {
                if let Some(child) = selected.get_first_child() {
                    tree.safe_select(&child);
                    true
                } else {
                    false
                }
            } else {
                false
            }
        }
        HierarchyAction::Collapse => {
            if !selected.is_collapsed() && selected.get_child_count() > 0 {
                selected.set_collapsed(true);
                true
            } else if let Some(parent) = selected.get_parent() {
                // Don't navigate to a hidden root — it's invisible and
                // unselectable, so the user would appear stuck.
                if parent.get_parent().is_some() || !tree.safe_is_root_hidden() {
                    tree.safe_select(&parent);
                    true
                } else {
                    false
                }
            } else {
                false
            }
        }
    }
}

fn handle_item_list_nav(mut list: Gd<ItemList>, direction: NavDirection) -> bool {
    let selected_items = list.get_selected_items();
    if selected_items.is_empty() {
        if list.get_item_count() > 0 {
            // IMPORTANT: only call `select()`, NOT `emit_signal("item_selected")`.
            // Emitting would trigger auto-open in the Scripts List dock, causing
            // the editor to switch scripts just from arrow-key browsing.
            list.select(0);
            list.ensure_current_is_visible();
            return true;
        }
        return false;
    }

    let current = selected_items.get(0).unwrap_or(0);
    let current_idx = current as usize;
    let count = list.get_item_count() as usize;

    let target_idx = match direction {
        NavDirection::Next => {
            if current_idx + 1 < count {
                current_idx + 1
            } else {
                return false;
            }
        }
        NavDirection::Prev => {
            if current_idx > 0 {
                current_idx - 1
            } else {
                return false;
            }
        }
    };

    list.deselect(current_idx as i32);
    list.select(target_idx as i32);
    list.ensure_current_is_visible();
    true
}

/// Scroll distance in pixels per j/k press. Tuned for readability in the
/// Godot editor's help/docs panels.
const RICHTEXTLABEL_SCROLL_STEP: f64 = 50.0;

fn handle_richtextlabel_nav(mut label: Gd<RichTextLabel>, direction: NavDirection) -> bool {
    let Some(mut scroll) = label.get_v_scroll_bar() else {
        return false;
    };
    let step = RICHTEXTLABEL_SCROLL_STEP;

    let current = scroll.get_value();
    let target = match direction {
        NavDirection::Next => current + step,
        NavDirection::Prev => (current - step).max(0.0),
    };

    scroll.set_value(target);
    true
}
