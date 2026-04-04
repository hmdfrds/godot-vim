//! Cross-panel navigation (`Ctrl+hjkl`).
//!
//! Maps Vim's `Ctrl-W h/j/k/l` window-movement commands to Godot's flat
//! dock/editor layout. Unlike Vim's window grid, Godot panels are
//! arbitrarily positioned, so we use a spatial cone + distance scoring
//! algorithm (~63-degree half-angle) to pick the nearest candidate in the
//! desired direction.

use godot::classes::{Control, EditorInterface, Node};
use godot::global::Key;
use godot::prelude::*;

use crate::bridge::godot_calls;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WindowNavDirection {
    Down,
    Up,
    Left,
    Right,
}

/// Try logical keycode first (respects key remapping), fall back to physical
/// keycode (layout-independent, US-QWERTY positions). This ensures Ctrl+hjkl
/// panel navigation works on non-Latin layouts (Russian, Greek, etc.) where
/// `get_keycode()` may not return the Latin H/J/K/L equivalents.
pub(crate) fn direction_from_hjkl(logical: Key, physical: Key) -> Option<WindowNavDirection> {
    hjkl_direction(logical).or_else(|| hjkl_direction(physical))
}

fn hjkl_direction(key: Key) -> Option<WindowNavDirection> {
    match key {
        Key::J => Some(WindowNavDirection::Down),
        Key::K => Some(WindowNavDirection::Up),
        Key::H => Some(WindowNavDirection::Left),
        Key::L => Some(WindowNavDirection::Right),
        _ => None,
    }
}

#[derive(Debug)]
pub(crate) enum WindowNavResult {
    Ignored,
    /// A target was found; `grab_focus()` has been deferred to it.
    Focused,
}

pub(crate) fn handle_window_nav(
    current: &Gd<Control>,
    direction: WindowNavDirection,
) -> WindowNavResult {
    let interface = EditorInterface::singleton();
    let Some(base) = interface.get_base_control() else {
        return WindowNavResult::Ignored;
    };

    let current_rect = current.get_global_rect();
    let current_center = current_rect.center();

    let candidates = find_window_candidates(&base);

    let mut best_candidate: Option<Gd<Control>> = None;
    let mut min_score = f32::MAX;

    log::debug!(
        "window_nav: direction={:?} current_center=({:.0},{:.0}) candidates={}",
        direction, current_center.x, current_center.y, candidates.len()
    );

    for candidate in candidates {
        if candidate.instance_id() == current.instance_id() {
            continue;
        }

        let cand_rect = candidate.get_global_rect();
        let cand_center = cand_rect.center();
        let diff = cand_center - current_center;

        // Cone check: secondary axis must not exceed 2x the primary axis
        // (half-angle ~63°). Deliberately wider than 45° so that panels
        // slightly off-axis (e.g., a dock whose center is diagonal from
        // the editor) are still reachable.
        let in_cone = match direction {
            WindowNavDirection::Down => diff.y > 0.0 && diff.y.abs() > diff.x.abs() * 0.5,
            WindowNavDirection::Up => diff.y < 0.0 && diff.y.abs() > diff.x.abs() * 0.5,
            WindowNavDirection::Left => diff.x < 0.0 && diff.x.abs() > diff.y.abs() * 0.5,
            WindowNavDirection::Right => diff.x > 0.0 && diff.x.abs() > diff.y.abs() * 0.5,
        };

        let class = candidate.clone().upcast::<Node>().get_class().to_string();
        let dist = current_center.distance_squared_to(cand_center);
        log::trace!(
            "  candidate: {} center=({:.0},{:.0}) diff=({:.0},{:.0}) in_cone={} dist={:.0}",
            class, cand_center.x, cand_center.y, diff.x, diff.y, in_cone, dist
        );

        if in_cone && dist < min_score {
            min_score = dist;
            best_candidate = Some(candidate);
        }
    }

    if let Some(target) = best_candidate {
        log::debug!("window_nav: {:?} -> focused #{}", direction, target.instance_id().to_i64());
        target
            .clone()
            .upcast::<Node>()
            .call_deferred("grab_focus", &[]);
        WindowNavResult::Focused
    } else {
        log::debug!("window_nav: {:?} -> no target", direction);
        WindowNavResult::Ignored
    }
}

const MAX_DISCOVERY_DEPTH: u32 = crate::scene_tree::MAX_DISCOVERY_DEPTH;

/// Excludes tiny focusable controls (buttons, checkboxes, toolbars) that
/// would be confusing cross-panel navigation targets.
const MIN_WINDOW_CANDIDATE_SIZE: f32 = 50.0;

fn find_window_candidates(root: &Gd<Control>) -> Vec<Gd<Control>> {
    let mut candidates = Vec::new();
    crate::scene_tree::collect_descendants(
        &root.clone().upcast::<Node>(),
        MAX_DISCOVERY_DEPTH,
        &mut candidates,
        &|node| {
            let control = node.clone().try_cast::<Control>().ok()?;
            if !control.is_visible_in_tree() { return None; }
            is_window_candidate(&control).then_some(control)
        },
    );
    candidates
}

fn is_window_candidate(control: &Gd<Control>) -> bool {
    if !control.is_visible_in_tree() {
        return false;
    }

    if control.get_focus_mode() == godot::classes::control::FocusMode::NONE {
        return false;
    }

    let size = control.get_size();
    if size.x < MIN_WINDOW_CANDIDATE_SIZE || size.y < MIN_WINDOW_CANDIDATE_SIZE {
        return false;
    }

    // Uses is_class() to walk the inheritance chain (catches FileSystemTree,
    // FileSystemList, etc.). TextEdit is intentionally excluded because
    // classify_focus() treats non-CodeEdit TextEdits as Foreign, which would
    // block Ctrl+hjkl FROM that control — creating a one-way navigation trap.
    let node = control.clone().upcast::<Node>();
    let is_known_type = crate::scene_tree::is_navigable_control(&node);

    if !is_known_type {
        return false;
    }

    // Walk ancestors (up to 6 levels) looking for a known editor container.
    // Godot wraps dock contents in variable-depth layout containers
    // (MarginContainer/VBoxContainer/SplitContainer), so parent-only checks
    // miss deeply nested controls like FileSystemDock's Tree.
    let mut ancestor = control.get_parent();
    for _ in 0..6 {
        let Some(node) = ancestor else { break };
        let class_name = node.get_class().to_string();

        // COMPAT: Internal editor classes, not part of public Godot API.
        if node.is_class(godot_calls::CLASS_CODE_TEXT_EDITOR)
            || node.is_class(godot_calls::CLASS_SHADER_TEXT_EDITOR)
            || node.is_class(godot_calls::CLASS_SCENE_TREE_EDITOR)
            || node.is_class(godot_calls::CLASS_EDITOR_HELP)
        {
            return true;
        }

        // COMPAT: Heuristic — substring match on dynamic class name to catch
        // all editor docks without a hardcoded allowlist.
        if class_name.contains("Dock") {
            return true;
        }

        ancestor = node.get_parent();
    }

    // No recognized ancestor within 6 levels — accept anyway. The type
    // allowlist + size/focus/visibility filters already exclude most false
    // positives. Rejecting would miss legitimate panels like the Scripts List
    // (nested under ScriptEditor → HSplitContainer → VBoxContainer, none of
    // which contain "Dock" or match known editor container classes).
    true
}
