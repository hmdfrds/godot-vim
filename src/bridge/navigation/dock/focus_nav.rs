//! Navigation logic for dock controls.
//!
//! Contains the NavigableItem trait, TreeExt extensions, and concrete
//! navigation/hierarchy handlers for Tree, ItemList, and RichTextLabel.

use crate::bridge::godot::names::item_list;
use crate::bridge::vim_adapter::core::cast::{i32_to_usize, usize_to_i32};
use godot::classes::{Control, ItemList, RichTextLabel, Tree};
use godot::prelude::*;

// ─────────────────────────────────────────────────────────────────────────────
// Types
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, strum::Display)]
pub enum NavDirection {
    Next, // Down
    Prev, // Up
}

#[derive(strum::Display)]
pub(super) enum HierarchyAction {
    Expand,
    Collapse,
}

// ─────────────────────────────────────────────────────────────────────────────
// Navigation Logic
// ─────────────────────────────────────────────────────────────────────────────

/// Trait defining a traversable item in a tree structure.
/// This allows the navigation logic to be purely functional and testable.
trait NavigableItem: Clone {
    fn nav_next_visible(&mut self) -> Option<Self>;
    fn nav_prev_visible(&mut self) -> Option<Self>;
    fn nav_is_selectable(&self) -> bool;
}

/// Pure function to find the next valid target in a direction.
/// Does not mutate state or call Godot APIs directly.
fn find_navigable_target<T: NavigableItem>(start: T, direction: &NavDirection) -> Option<T> {
    let mut current = start.clone();

    // Guard against infinite loops in pathological tree structures.
    let mut attempts: usize = 0;
    const MAX_ATTEMPTS: usize = 1000;

    loop {
        if attempts > MAX_ATTEMPTS {
            break None;
        }
        attempts += 1;

        let next = match direction {
            NavDirection::Next => current.nav_next_visible(),
            NavDirection::Prev => current.nav_prev_visible(),
        };

        if let Some(item) = next {
            if item.nav_is_selectable() {
                return Some(item);
            }
            current = item;
        } else {
            return None; // End of tree
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Godot Implementation
// ─────────────────────────────────────────────────────────────────────────────

impl NavigableItem for Gd<godot::classes::TreeItem> {
    fn nav_next_visible(&mut self) -> Option<Self> {
        self.get_next_visible()
    }

    fn nav_prev_visible(&mut self) -> Option<Self> {
        self.get_prev_visible()
    }

    fn nav_is_selectable(&self) -> bool {
        // Column 0 is the standard for main selection
        self.is_selectable(0)
    }
}

/// Extension trait to encapsulate Unsafe/Dynamic Godot calls.
/// This acts as an Anti-Corruption Layer for fragile APIs.
trait TreeExt {
    fn safe_scroll_to_item(&mut self, item: &Gd<godot::classes::TreeItem>);
    fn safe_select(&mut self, item: &Gd<godot::classes::TreeItem>);
    fn safe_is_root_hidden(&self) -> bool;
}

impl TreeExt for Gd<Tree> {
    fn safe_scroll_to_item(&mut self, item: &Gd<godot::classes::TreeItem>) {
        // Bypassing Godot-Rust binding issue (E0271) via dynamic call.
        // This string literal is centralised here; use this method site-wide.
        self.call(item_list::methods::SCROLL_TO_ITEM, &[item.to_variant()]);
    }

    fn safe_select(&mut self, item: &Gd<godot::classes::TreeItem>) {
        // Moves the cursor in multi-select mode without clearing existing selection.
        self.set_selected(item, 0);
    }

    fn safe_is_root_hidden(&self) -> bool {
        // Native method name differs from property name
        self.is_root_hidden()
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Dispatch
// ─────────────────────────────────────────────────────────────────────────────

pub(super) fn handle_navigation(control: &Gd<Control>, direction: NavDirection) -> bool {
    // Use is_class to catch Tree subclasses (like in FileSystemDock)
    if control.is_class("Tree") {
        let tree = control.clone().cast::<Tree>();
        handle_tree_nav(tree, direction)
    } else if control.is_class("ItemList") {
        // Use is_class + explicit cast to catch internal subclasses like FileSystemList
        let list = control.clone().cast::<ItemList>();
        handle_item_list_nav(list, direction)
    } else if control.is_class("RichTextLabel") {
        let label = control.clone().cast::<RichTextLabel>();
        handle_richtextlabel_nav(label, direction)
    } else {
        // Recursively look for a navigable child (e.g. inside EditorHelp VBox).
        if let Some(target) = find_best_nav_target(control) {
            return handle_navigation(&target, direction);
        }
        false
    }
}

fn find_best_nav_target(root: &Gd<Control>) -> Option<Gd<Control>> {
    for child in root.get_children().iter_shared() {
        if let Ok(control) = child.clone().try_cast::<Control>() {
            if (control.is_class("Tree")
                || control.is_class("ItemList")
                || control.is_class("RichTextLabel"))
                && control.is_visible_in_tree()
            {
                return Some(control);
            }
            if let Some(found) = find_best_nav_target(&control) {
                return Some(found);
            }
        }
    }
    None
}

pub(super) fn handle_hierarchy(control: &Gd<Control>, action: HierarchyAction) -> bool {
    // Use is_class to catch Tree subclasses
    if control.is_class("Tree") {
        let tree = control.clone().cast::<Tree>();
        handle_tree_hierarchy(tree, action)
    } else {
        // ItemList is flat; h/l hierarchy actions are not applicable
        false
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Handlers
// ─────────────────────────────────────────────────────────────────────────────

fn handle_tree_nav(mut tree: Gd<Tree>, direction: NavDirection) -> bool {
    let Some(selected) = tree.get_selected() else {
        // No selection; fall back to selecting the root item.
        if let Some(root) = tree.get_root() {
            tree.safe_select(&root);
            tree.safe_scroll_to_item(&root);
            tree.queue_redraw();
            return true;
        }
        return false;
    };

    let target = find_navigable_target(selected, &direction);

    if let Some(target) = target {
        tree.safe_select(&target);
        tree.safe_scroll_to_item(&target);
        tree.queue_redraw();
        return true;
    }

    false
}

fn handle_tree_hierarchy(mut tree: Gd<Tree>, action: HierarchyAction) -> bool {
    let Some(mut selected) = tree.get_selected() else {
        return false;
    };

    match action {
        HierarchyAction::Expand => {
            if selected.is_collapsed() {
                selected.set_collapsed(false);
                return true;
            } else if selected.get_child_count() > 0 {
                // Descend into the first child.
                if let Some(child) = selected.get_first_child() {
                    tree.safe_select(&child);
                    return true;
                }
            }
        }
        HierarchyAction::Collapse => {
            if !selected.is_collapsed() && selected.get_child_count() > 0 {
                selected.set_collapsed(true);
                return true;
            } else if let Some(parent) = selected.get_parent() {
                // Use safe wrapper for root visibility check
                if parent.get_parent().is_some() || !tree.safe_is_root_hidden() {
                    tree.safe_select(&parent);
                    return true;
                }
            }
        }
    }

    false
}

// ─────────────────────────────────────────────────────────────────────────────
// ItemList Implementation
// ─────────────────────────────────────────────────────────────────────────────

fn handle_item_list_nav(mut list: Gd<ItemList>, direction: NavDirection) -> bool {
    let selected_items = list.get_selected_items();
    if selected_items.is_empty() {
        if list.get_item_count() > 0 {
            list.select(0);
            // Do not emit item_selected; that triggers auto-open in the Script list.
            // Visual selection only — the user must press Enter to activate.
            list.ensure_current_is_visible();
            return true;
        }
        return false;
    }

    // The is_empty() check above guarantees index 0 exists; unwrap_or(0) is a defensive fallback.
    let current = selected_items.get(0).unwrap_or(0);
    let current_idx = i32_to_usize(current);
    let count = i32_to_usize(list.get_item_count());

    let target_idx = match direction {
        NavDirection::Next => {
            if current_idx + 1 < count {
                current_idx + 1
            } else {
                return false; // End of list
            }
        }
        NavDirection::Prev => {
            if current_idx > 0 {
                current_idx - 1
            } else {
                return false; // Top of list
            }
        }
    };

    // Deselect the previous item to emulate single-selection navigation.
    // ItemList supports multi-select, but hjkl navigation moves a single cursor.
    list.deselect(usize_to_i32(current_idx));
    list.select(usize_to_i32(target_idx));
    // Do not emit item_selected; that triggers auto-open in the Script list.
    // Visual selection only — the user must press Enter to activate.
    list.ensure_current_is_visible();

    true
}

fn handle_richtextlabel_nav(mut label: Gd<RichTextLabel>, direction: NavDirection) -> bool {
    // RichTextLabel has no selection-based navigation; scroll the viewport instead.
    let Some(mut scroll) = label.get_v_scroll_bar() else {
        return false;
    };
    let step = 50.0; // Pixels per jump

    let current = scroll.get_value();
    let target = match direction {
        NavDirection::Next => current + step,
        NavDirection::Prev => (current - step).max(0.0),
    };

    scroll.set_value(target);
    true
}
