//! Cycle-focus navigation among all navigable panels in the editor.
//!
//! Maps `WindowNavAction` effects to either spatial cycling (CycleNext/CyclePrev)
//! or directional movement (delegated to `navigation::window`).
//!
//! Cycling order is determined by on-screen position (top-to-bottom,
//! left-to-right), which differs from Vim's native `:bnext`/`:bprev` buffer
//! ordering. This is intentional: Godot's dock layout is spatial, so
//! position-based cycling feels more natural than insertion-order cycling.

use godot::classes::{Control, Node};
use godot::prelude::*;

use crate::effects::WindowNavAction;
use crate::navigation::window::{self, WindowNavDirection};

/// Runs at the controller level where Godot scene tree access is available.
/// The vim-core effects layer only describes *what* to do; this executes it.
pub(crate) fn handle_window_nav_action(current: &Gd<Control>, action: WindowNavAction) {
    match action {
        WindowNavAction::MoveLeft
        | WindowNavAction::MoveRight
        | WindowNavAction::MoveUp
        | WindowNavAction::MoveDown => {
            let direction = match action {
                WindowNavAction::MoveLeft => WindowNavDirection::Left,
                WindowNavAction::MoveRight => WindowNavDirection::Right,
                WindowNavAction::MoveUp => WindowNavDirection::Up,
                WindowNavAction::MoveDown => WindowNavDirection::Down,
                _ => return,
            };
            let result = window::handle_window_nav(current, direction);
            log::debug!("window_nav: {:?} -> {:?}", action, result);
        }
        WindowNavAction::CycleNext | WindowNavAction::CyclePrev => {
            handle_cycle_focus(current, action);
        }
        WindowNavAction::CloseTab => {
            log::debug!(
                "WindowClose: tab closing is handled via :q/:close host request, \
                 not via effect dispatch"
            );
        }
    }
}

/// Cycle focus among all navigable panels in spatial (screen-position) order.
///
/// When the current control isn't found among candidates (e.g., focus is on
/// a transient popup), cycling starts from the first or last candidate
/// rather than silently doing nothing.
fn handle_cycle_focus(current: &Gd<Control>, action: WindowNavAction) {
    use godot::classes::EditorInterface;

    let interface = EditorInterface::singleton();
    let Some(base) = interface.get_base_control() else {
        return;
    };

    let candidates = find_cycle_candidates(&base);
    if candidates.is_empty() {
        return;
    }

    let current_id = current.instance_id();
    let current_idx = candidates
        .iter()
        .position(|c| c.instance_id() == current_id);

    let is_next = matches!(action, WindowNavAction::CycleNext);
    let target_idx = match current_idx {
        Some(idx) => {
            if is_next {
                (idx + 1) % candidates.len()
            } else {
                (idx + candidates.len() - 1) % candidates.len()
            }
        }
        None => {
            if is_next { 0 } else { candidates.len() - 1 }
        }
    };

    let target = &candidates[target_idx];
    log::debug!(
        "cycle_focus: {:?} -> #{}",
        action,
        target.instance_id().to_i64()
    );
    // Deferred because focus changes during input processing can be
    // swallowed by Godot's event dispatch.
    target
        .clone()
        .upcast::<Node>()
        .call_deferred("grab_focus", &[]);
}

fn find_cycle_candidates(root: &Gd<Control>) -> Vec<Gd<Control>> {
    /// Shallower than `scene_tree::MAX_DISCOVERY_DEPTH` (20) because cycle
    /// candidates are top-level editor panels, not deeply nested internals.
    const MAX_DEPTH: u32 = 10;

    let mut candidates = Vec::new();
    crate::scene_tree::collect_descendants(
        &root.clone().upcast::<Node>(),
        MAX_DEPTH,
        &mut candidates,
        &|node| {
            let control = node.clone().try_cast::<Control>().ok()?;
            if !control.is_visible_in_tree() {
                return None;
            }
            is_cycle_candidate(&control).then_some(control)
        },
    );

    candidates.sort_by(|a, b| {
        let ac = a.get_global_rect().center();
        let bc = b.get_global_rect().center();
        ac.y.partial_cmp(&bc.y)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(
                ac.x.partial_cmp(&bc.x)
                    .unwrap_or(std::cmp::Ordering::Equal),
            )
    });

    candidates
}

/// Excludes tiny controls (buttons, checkboxes) that happen to be focusable.
const MIN_CYCLE_CANDIDATE_SIZE: f32 = 50.0;

fn is_cycle_candidate(control: &Gd<Control>) -> bool {
    if !control.is_visible_in_tree() {
        return false;
    }
    if control.get_focus_mode() == godot::classes::control::FocusMode::NONE {
        return false;
    }
    let size = control.get_size();
    if size.x < MIN_CYCLE_CANDIDATE_SIZE || size.y < MIN_CYCLE_CANDIDATE_SIZE {
        return false;
    }
    let node = control.clone().upcast::<Node>();
    crate::scene_tree::is_navigable_control(&node)
}
