//! Floating window tracking: detects Godot editor `WindowWrapper` nodes,
//! connects viewport signals on their floating `Window` children, and tears
//! everything down on shutdown.
//!
//! All methods are `pub(super)` — only [`super::GodotVimPlugin`] signal
//! handlers and lifecycle hooks call them.

use godot::classes::EditorInterface;
use godot::prelude::*;

use super::{
    GodotVimPlugin, SIG_CHILD_ENTERED_TREE, SIG_GUI_FOCUS_CHANGED,
    SIG_TREE_EXITED, SIG_WINDOW_VISIBILITY_CHANGED,
};
use super::signals::{connect_deferred, connect_immediate, safe_disconnect};

/// Tracks a floating WindowWrapper and its associated Window viewport.
pub(super) struct TrackedWindow {
    pub(super) wrapper_id: InstanceId,
    /// `Some` when the wrapper's Window child is visible (floating); `None` when docked.
    pub(super) window_id: Option<InstanceId>,
}

/// Detect whether a node is a Godot editor `WindowWrapper`.
///
/// `WindowWrapper` is an internal C++ editor class that extends
/// `MarginContainer`. It is NOT registered in ClassDB with `GDCLASS`, so
/// `Node::is_class("WindowWrapper")` returns `false` -- it only sees the
/// registered base class `MarginContainer`.
///
/// Instead we use a signal-based heuristic: `WindowWrapper` defines a custom
/// signal `window_visibility_changed` that no other standard node type has.
/// Checking `has_signal()` is reliable, fast, and survives Godot version
/// changes that might rename or re-parent the internal class.
pub(super) fn is_window_wrapper(node: &Gd<Node>) -> bool {
    node.has_signal(SIG_WINDOW_VISIBILITY_CHANGED)
}

impl GodotVimPlugin {
    // ── Viewport signal connection ───────────────────────────────────

    /// Connect focus signals on Window children of visible (floating) wrappers.
    pub(super) fn connect_floating_viewport(&mut self) {
        let callable = self.base().callable("on_focus_changed");
        let focus_callable = self.base().callable("on_floating_window_focused");
        for tw in &mut self.tracked_windows {
            let Ok(wrapper) = Gd::<Node>::try_from_instance_id(tw.wrapper_id) else {
                log::trace!(
                    "connect_floating_viewport: wrapper #{} freed, skipping",
                    tw.wrapper_id.to_i64()
                );
                continue;
            };

            log::trace!(
                "connect_floating_viewport: wrapper #{} has {} children",
                tw.wrapper_id.to_i64(), wrapper.get_child_count()
            );

            let mut found_window = false;
            for child in wrapper.get_children().iter_shared() {
                if child.is_class("Window") {
                    found_window = true;
                    // Only connect to visible (floating) windows. Docked wrappers
                    // keep a hidden Window child — connecting to it would violate
                    // the window_id invariant (Some = floating, None = docked).
                    let Ok(window_check) = child.clone().try_cast::<godot::classes::Window>() else {
                        continue;
                    };
                    if !window_check.is_visible() {
                        log::trace!(
                            "connect_floating_viewport: Window in wrapper #{} is hidden (docked), skipping",
                            tw.wrapper_id.to_i64()
                        );
                        break;
                    }
                    let window_id = child.instance_id();
                    if tw.window_id == Some(window_id) {
                        log::debug!(
                            "connect_floating_viewport: Window #{} already connected",
                            window_id.to_i64()
                        );
                        break;
                    }

                    // Window inherits Viewport -- gui_focus_changed detects
                    // focus changes between controls within the floating window.
                    let mut node = child;
                    let was_disconnected = !node.is_connected(SIG_GUI_FOCUS_CHANGED, &callable);
                    connect_deferred(&mut node, SIG_GUI_FOCUS_CHANGED, &callable);
                    if was_disconnected {
                        log::debug!(
                            "connect_floating_viewport: connected gui_focus_changed on floating Window #{}",
                            window_id.to_i64()
                        );
                    }

                    // gui_focus_changed does NOT fire when the user clicks back
                    // into the floating window from the main window, because the
                    // CodeEdit never lost key_focus within that viewport.
                    // focus_entered fires on every OS-level window focus event.
                    let was_disconnected = !node.is_connected("focus_entered", &focus_callable);
                    connect_immediate(&mut node, "focus_entered", &focus_callable);
                    if was_disconnected {
                        log::debug!(
                            "connect_floating_viewport: connected focus_entered on Window #{}",
                            window_id.to_i64()
                        );
                    }

                    tw.window_id = Some(window_id);
                    break;
                }
            }
            if !found_window {
                log::trace!(
                    "connect_floating_viewport: no Window child found in wrapper #{}",
                    tw.wrapper_id.to_i64()
                );
            }
        }
    }

    /// Selectively disconnect focus signals from unfloated windows only.
    ///
    /// Checks each wrapper's Window visibility rather than blanket-disconnecting
    /// all tracked windows -- multiple editors can be floating simultaneously,
    /// and unfloating one must not break the others.
    pub(super) fn disconnect_floating_viewport(&mut self) {
        let callable = self.base().callable("on_focus_changed");
        let focus_callable = self.base().callable("on_floating_window_focused");
        for tw in &mut self.tracked_windows {
            let Some(window_id) = tw.window_id else { continue };

            // The Window node is a permanent child of the wrapper -- it is
            // hidden (not removed) when unfloated. Must check visibility,
            // not child presence.
            let wrapper_still_visible = Gd::<Node>::try_from_instance_id(window_id)
                .ok()
                .is_some_and(|window| {
                    window.is_class("Window")
                        && window.clone().try_cast::<godot::classes::Window>()
                            .is_ok_and(|w| w.is_visible())
                });

            if wrapper_still_visible {
                log::trace!(
                    "disconnect_floating_viewport: wrapper #{} still has Window #{}, skipping",
                    tw.wrapper_id.to_i64(), window_id.to_i64()
                );
                continue;
            }

            tw.window_id = None;
            log::debug!(
                "disconnect_floating_viewport: disconnecting Window #{} from wrapper #{}",
                window_id.to_i64(), tw.wrapper_id.to_i64()
            );
            if let Ok(mut window) = Gd::<Node>::try_from_instance_id(window_id) {
                safe_disconnect(&mut window, SIG_GUI_FOCUS_CHANGED, &callable);
                safe_disconnect(&mut window, "focus_entered", &focus_callable);
            }
        }
    }

    /// Unconditionally disconnect ALL floating window signals (exit_tree only).
    pub(super) fn disconnect_all_floating_viewports(&mut self) {
        let callable = self.base().callable("on_focus_changed");
        let focus_callable = self.base().callable("on_floating_window_focused");
        for tw in &mut self.tracked_windows {
            if let Some(window_id) = tw.window_id.take() {
                if let Ok(mut window) = Gd::<Node>::try_from_instance_id(window_id) {
                    safe_disconnect(&mut window, SIG_GUI_FOCUS_CHANGED, &callable);
                    safe_disconnect(&mut window, "focus_entered", &focus_callable);
                }
            }
        }
    }

    // ── Scanning ─────────────────────────────────────────────────────

    /// Scan known WindowWrapper parent locations and begin tracking any new ones.
    ///
    /// WindowWrappers live under three separate parents in the editor scene tree:
    ///   1. `gui_base.get_parent()` -- top-level editor windows
    ///   2. `EditorMainScreen`      -- ScriptEditor / Shader / GameView floats
    ///   3. `gui_base` itself       -- dock windows
    pub(super) fn scan_floating_windows(&mut self) {
        let interface = EditorInterface::singleton();

        // Deduplicate by InstanceId -- the three logical parents may overlap.
        let mut scan_roots: Vec<(Gd<Node>, &str)> = Vec::with_capacity(3);

        if let Some(gui_base) = interface.get_base_control() {
            scan_roots.push((gui_base.clone().upcast::<Node>(), "gui_base"));
            if let Some(parent) = gui_base.get_parent() {
                scan_roots.push((parent, "gui_base_parent"));
            }
        }

        if let Some(main_screen) = interface.get_editor_main_screen() {
            let ms_node = main_screen.upcast::<Node>();
            let ms_id = ms_node.instance_id();
            let already_listed = scan_roots.iter().any(|(n, _)| n.instance_id() == ms_id);
            if !already_listed {
                scan_roots.push((ms_node, "editor_main_screen"));
            }
        }

        log::trace!(
            "scan_floating_windows: scanning {} root(s): [{}]",
            scan_roots.len(),
            scan_roots.iter().map(|(n, label)| {
                format!("{}({})", label, n.get_class())
            }).collect::<Vec<_>>().join(", ")
        );

        let callable = self.base().callable("on_window_visibility_changed");
        let mut newly_tracked = 0u32;

        for (parent, label) in &scan_roots {
            let child_count = parent.get_child_count();
            let mut wrappers_in_parent = 0u32;

            for child in parent.get_children().iter_shared() {
                let child_class = child.get_class().to_string();

                if !is_window_wrapper(&child) {
                    continue;
                }
                wrappers_in_parent += 1;

                let wrapper_id = child.instance_id();

                if self.tracked_windows.iter().any(|tw| tw.wrapper_id == wrapper_id) {
                    log::trace!(
                        "scan_floating_windows: [{}] already tracking #{} (class={})",
                        label, wrapper_id.to_i64(), child_class
                    );
                    continue;
                }

                let mut node = child;
                connect_immediate(&mut node, SIG_WINDOW_VISIBILITY_CHANGED, &callable);
                let tree_exit_callable = self.base().callable("on_wrapper_tree_exited");
                connect_immediate(&mut node, SIG_TREE_EXITED, &tree_exit_callable);

                log::debug!(
                    "scan_floating_windows: [{}] tracking new WindowWrapper #{} (class={})",
                    label, wrapper_id.to_i64(), child_class
                );

                self.tracked_windows.push(TrackedWindow {
                    wrapper_id,
                    window_id: None,
                });
                newly_tracked += 1;
            }

            log::trace!(
                "scan_floating_windows: [{}] {} children total, {} WindowWrappers found",
                label, child_count, wrappers_in_parent
            );
        }

        let before = self.tracked_windows.len();
        self.tracked_windows.retain(|tw| {
            Gd::<Node>::try_from_instance_id(tw.wrapper_id).is_ok()
        });
        let evicted = before - self.tracked_windows.len();

        log::trace!(
            "scan_floating_windows: done -- {} newly tracked, {} evicted, {} total",
            newly_tracked, evicted, self.tracked_windows.len()
        );
    }

    // ── Lifecycle ────────────────────────────────────────────────────

    /// Set up reactive WindowWrapper detection via `child_entered_tree`
    /// signals on the known parent nodes, then run an initial scan.
    pub(super) fn init_floating_window_tracking(&mut self) {
        let interface = EditorInterface::singleton();
        let child_callable = self.base().callable("on_child_entered_tree");

        if let Some(gui_base) = interface.get_base_control() {
            if let Some(mut parent) = gui_base.get_parent() {
                let parent_class = parent.get_class().to_string();
                log::trace!(
                    "enter_tree: gui_base parent = {} (#{})",
                    parent_class,
                    parent.instance_id().to_i64()
                );
                let was_disconnected = !parent.is_connected(SIG_CHILD_ENTERED_TREE, &child_callable);
                connect_immediate(&mut parent, SIG_CHILD_ENTERED_TREE, &child_callable);
                if was_disconnected {
                    log::trace!(
                        "enter_tree: connected child_entered_tree on gui_base parent ({})",
                        parent_class
                    );
                }
            }
        }

        if let Some(gui_base_node) = interface.get_base_control() {
            let gb_class = gui_base_node.get_class().to_string();
            log::trace!(
                "enter_tree: gui_base = {} (#{})",
                gb_class,
                gui_base_node.instance_id().to_i64()
            );
            let was_disconnected =
                !gui_base_node.is_connected(SIG_CHILD_ENTERED_TREE, &child_callable);
            connect_immediate(
                &mut gui_base_node.clone().upcast::<Node>(),
                SIG_CHILD_ENTERED_TREE,
                &child_callable,
            );
            if was_disconnected {
                log::trace!(
                    "enter_tree: connected child_entered_tree on gui_base ({})",
                    gb_class
                );
            }
        }

        if let Some(mut main_screen) = interface.get_editor_main_screen() {
            let ms_class = main_screen.get_class().to_string();
            log::trace!(
                "enter_tree: EditorMainScreen = {} (#{})",
                ms_class,
                main_screen.instance_id().to_i64()
            );
            let was_disconnected = !main_screen.is_connected(SIG_CHILD_ENTERED_TREE, &child_callable);
            connect_immediate(&mut main_screen, SIG_CHILD_ENTERED_TREE, &child_callable);
            if was_disconnected {
                log::trace!(
                    "enter_tree: connected child_entered_tree on EditorMainScreen ({})",
                    ms_class
                );
            }
        } else {
            log::warn!(
                "enter_tree: EditorMainScreen unavailable, floating ScriptEditor detection limited"
            );
        }

        self.scan_floating_windows();

        // Connect focus signals on any windows that are already floating at
        // plugin init time. The scan above tracked the wrappers but created
        // them with `window_id: None`. This call finds visible Window children
        // and wires up gui_focus_changed + focus_entered.
        self.connect_floating_viewport();
    }

    /// Symmetric teardown: disconnect all child_entered_tree, focus, and
    /// visibility signals, then clear the tracked windows list.
    pub(super) fn teardown_floating_window_tracking(&mut self) {
        let interface = EditorInterface::singleton();
        let child_callable = self.base().callable("on_child_entered_tree");
        if let Some(gui_base) = interface.get_base_control() {
            if let Some(mut parent) = gui_base.get_parent() {
                safe_disconnect(&mut parent, SIG_CHILD_ENTERED_TREE, &child_callable);
            }
        }
        if let Some(gui_base) = interface.get_base_control() {
            safe_disconnect(
                &mut gui_base.clone().upcast::<Node>(),
                SIG_CHILD_ENTERED_TREE,
                &child_callable,
            );
        }
        if let Some(mut main_screen) = interface.get_editor_main_screen() {
            safe_disconnect(&mut main_screen, SIG_CHILD_ENTERED_TREE, &child_callable);
        }

        self.disconnect_all_floating_viewports();

        let vis_callable = self.base().callable("on_window_visibility_changed");
        let tree_exit_callable = self.base().callable("on_wrapper_tree_exited");
        for tw in &self.tracked_windows {
            if let Ok(mut wrapper) =
                Gd::<godot::classes::Node>::try_from_instance_id(tw.wrapper_id)
            {
                safe_disconnect(&mut wrapper, SIG_WINDOW_VISIBILITY_CHANGED, &vis_callable);
                safe_disconnect(&mut wrapper, SIG_TREE_EXITED, &tree_exit_callable);
            }
        }
        self.tracked_windows.clear();
    }
}
