//! Status bar integration for Vim mode display.
//!
//! Handles injection of `VimCmdLine` as a floating overlay directly on the CodeEdit.
//! This maintains the "Vim Aesthetic" while using rigid anchoring to prevent
//! Godot from mangling the position during layout passes.

use godot::classes::{CodeEdit, Control, Node};
use godot::prelude::*;

use crate::bridge::components::cmdline::VimCmdLine;

/// Result of status bar injection attempt.
#[derive(PartialEq, Debug, Clone, Copy, strum::Display)]
pub enum InjectionResult {
    /// Injected as a floating overlay on the editor.
    FloatingOverlay,
}

/// Injects `VimCmdLine` as a floating overlay directly on the CodeEdit.
pub fn inject_cmdline(editor: &Gd<CodeEdit>, cmd_line: &Gd<VimCmdLine>) -> InjectionResult {
    let cmd_line_node = cmd_line.clone().upcast::<Node>();
    let editor_node = editor.clone().upcast::<Node>();

    // Remove any orphaned VimStatusBar nodes in the editor or its direct parent.
    let mut search_areas = vec![editor_node.clone()];
    if let Some(parent) = editor.get_parent() {
        search_areas.push(parent);
    }

    for area in search_areas {
        for child in area.get_children().iter_shared() {
            let node_name = child.get_name().to_string();
            if node_name.starts_with("VimStatusBar")
                && child.instance_id() != cmd_line_node.instance_id()
            {
                let mut orphan = child;
                if let Some(mut p) = orphan.get_parent() {
                    p.remove_child(&orphan);
                }
                orphan.queue_free();
            }
        }
    }

    // Reparent to the CodeEdit if currently attached elsewhere.
    if let Some(mut old_parent) = cmd_line_node.get_parent() {
        if old_parent.instance_id() != editor_node.instance_id() {
            old_parent.remove_child(&cmd_line_node);
            editor.clone().add_child(&cmd_line_node);
        }
    } else {
        editor.clone().add_child(&cmd_line_node);
    }

    InjectionResult::FloatingOverlay
}

/// Configures `VimCmdLine` with rigid anchors to pin it to bottom-right.
///
/// `set_anchors_preset` is avoided because Godot's internal layout passes
/// occasionally reset preset data. Explicitly setting anchors to 1.0
/// and using negative offsets is the most stable strategy for floating overlays.
pub fn configure_cmdline_sizing(cmd_line: &Gd<VimCmdLine>) {
    let mut control = cmd_line.clone().upcast::<Control>();

    // Rigidly anchor all 4 corners to the bottom-right (1.0, 1.0)
    for i in 0..4 {
        let side = match i {
            0 => godot::builtin::Side::LEFT,
            1 => godot::builtin::Side::TOP,
            2 => godot::builtin::Side::RIGHT,
            _ => godot::builtin::Side::BOTTOM,
        };
        control.set_anchor(side, 1.0);

        // Fixed 10px padding from the corner for RIGHT and BOTTOM.
        // LEFT and TOP offsets being 0.0 allows the control to be sized
        // by its internal content (minimum size) from the 1.0 anchor point.
        let offset = if side == Side::RIGHT || side == Side::BOTTOM {
            -10.0
        } else {
            0.0
        };
        control.set_offset(side, offset);
    }

    // Grow toward the top-left from the (1.0, 1.0) anchor point.
    control.set_h_grow_direction(godot::classes::control::GrowDirection::BEGIN);
    control.set_v_grow_direction(godot::classes::control::GrowDirection::BEGIN);

    // Pass clicks to the editor underneath, unless clicking a specific UI element.
    control.set_mouse_filter(godot::classes::control::MouseFilter::PASS);
}

/// No-op retained for interface compatibility.
#[allow(dead_code)]
pub fn cleanup_native_status_bar(_editor: &Gd<CodeEdit>) {}

/// No-op retained for interface compatibility.
pub fn restore_status_bar(_cmd_line: &Gd<VimCmdLine>) {}
