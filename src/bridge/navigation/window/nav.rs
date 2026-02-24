//! Window Navigation (Hyprland-style Ctrl+hjkl).
//!
//! Handles navigation between major editor regions (docks, code editors, etc.)
//! using directional movement similar to Hyprland's focus navigation.
//!
//! ## Separation of Concerns
//! This module was extracted from `dock_navigation.rs` to separate:
//! - **Window Navigation** (Ctrl+hjkl): Moving between major editor panels
//! - **Intra-Dock Navigation** (hjkl): Moving within a Tree/ItemList

use crate::bridge::godot::names::control;
use godot::classes::{Control, EditorInterface, Node};
use godot::prelude::*;
use strum::Display;

/// Direction for navigation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Display)]
pub enum NavDirection {
    Next,  // Down (j)
    Prev,  // Up (k)
    Left,  // Left (h)
    Right, // Right (l)
}

/// Result of window navigation.
#[derive(Debug)]
pub enum WindowNavResult {
    /// Navigation was ignored (no valid target found).
    Ignored,
    /// Navigation succeeded, focus will be moved to the target.
    Focused(Gd<Control>),
}

/// Handle window navigation (Ctrl+hjkl).
///
/// Finds the nearest focusable editor window in the specified direction
/// and defers focus change to avoid re-entrancy issues.
pub fn handle_window_nav(current: &Gd<Control>, direction: NavDirection) -> WindowNavResult {
    let interface = EditorInterface::singleton();
    let Some(base) = interface.get_base_control() else {
        log::warn!("Window navigation: no base control found");
        return WindowNavResult::Ignored;
    };

    let current_rect = current.get_global_rect();
    let current_center = current_rect.center();

    let candidates = find_window_candidates_recursive(&base);
    log::debug!("Window navigation found {} candidates", candidates.len());

    let mut best_candidate: Option<Gd<Control>> = None;
    let mut min_score = f32::MAX;

    for candidate in candidates {
        // Skip self
        if candidate.instance_id() == current.instance_id() {
            continue;
        }

        let cand_rect = candidate.get_global_rect();
        let cand_center = cand_rect.center();

        let diff = cand_center - current_center;

        // Direction check with 45-degree cone tolerance
        let is_direction = match direction {
            // Y is down in screen coords
            NavDirection::Next => diff.y > 0.0 && diff.y.abs() > diff.x.abs() * 0.5,
            NavDirection::Prev => diff.y < 0.0 && diff.y.abs() > diff.x.abs() * 0.5,
            NavDirection::Left => diff.x < 0.0 && diff.x.abs() > diff.y.abs() * 0.5,
            NavDirection::Right => diff.x > 0.0 && diff.x.abs() > diff.y.abs() * 0.5,
        };

        if is_direction {
            let dist = current_center.distance_squared_to(cand_center);
            if dist < min_score {
                min_score = dist;
                best_candidate = Some(candidate);
            }
        }
    }

    if let Some(target) = best_candidate {
        log::debug!(
            "Window navigation jumping to name={} class={}",
            target.get_name(),
            target.get_class()
        );
        // Deferred to avoid re-entrant borrow panic.
        target
            .clone()
            .upcast::<Node>()
            .call_deferred(control::methods::GRAB_FOCUS, &[]);
        return WindowNavResult::Focused(target);
    }

    WindowNavResult::Ignored
}

/// Recursively find all focusable window candidates.
fn find_window_candidates_recursive(root: &Gd<Control>) -> Vec<Gd<Control>> {
    let mut candidates = Vec::new();

    // Skip hidden subtrees entirely.
    if !root.is_visible() {
        return candidates;
    }

    // Check if current node is a candidate
    if is_window_candidate(root) {
        candidates.push(root.clone());
    }

    // Recurse
    for child in root.get_children().iter_shared() {
        if let Ok(control) = child.try_cast::<Control>() {
            candidates.extend(find_window_candidates_recursive(&control));
        }
    }

    candidates
}

/// Check if a control is a valid window navigation target.
fn is_window_candidate(control: &Gd<Control>) -> bool {
    // Strict visibility check (in tree)
    if !control.is_visible_in_tree() {
        return false;
    }

    // Must accept focus
    if control.get_focus_mode() == godot::classes::control::FocusMode::NONE {
        return false;
    }

    // Size check: Ignore tiny controls (buttons, checkboxes)
    // 50x50 pixels is the minimum acceptable size for a Dock or Editor control.
    let size = control.get_size();
    if size.x < 50.0 || size.y < 50.0 {
        return false;
    }

    // Type allowlist
    let class = control.get_class().to_string();
    let is_base_type = matches!(
        class.as_str(),
        "Tree"
            | "ItemList"
            | "CodeEdit"
            | "TextEdit"
            | "GraphEdit"
            | "FileSystemList"
            | "RichTextLabel"
    );

    if !is_base_type {
        return false;
    }

    // Check the parent class to exclude internal Godot widgets.
    if let Some(parent) = control.get_parent() {
        let p_class = parent.get_class().to_string();

        // Known editor container classes — a matching parent confirms this is a valid target.
        let is_valid_parent = matches!(
            p_class.as_str(),
            "CodeTextEditor"
                | "ShaderTextEditor"
                | "SceneTreeEditor"
                | "FileSystemDock"
                | "HSplitContainer"
                | "VSplitContainer"
                | "SplitContainer"
                | "VBoxContainer"
        );

        if is_valid_parent {
            return true;
        }

        // Second-level check for Docks
        if let Some(grandparent) = parent.get_parent() {
            let g_class = grandparent.get_class().to_string();
            if g_class.contains("Dock") || g_class == "EditorHelp" {
                return true;
            }
        }
    }

    // The control is a known class with acceptable size; accept it.
    true
}
