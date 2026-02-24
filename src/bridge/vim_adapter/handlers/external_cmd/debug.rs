//! Debug operations: ToggleBreakpoint, DebugContinue, DebugNext, etc.

use crate::bridge::godot::names::{debugger, node};
use godot::classes::CodeEdit;
use godot::prelude::*;

pub fn handle_toggle_breakpoint(editor: &mut Gd<CodeEdit>) {
    let line = editor.get_caret_line();
    let is_breakpointed = editor.is_line_breakpointed(line);
    editor.set_line_as_breakpoint(line, !is_breakpointed);
}

/// Continue execution.
pub fn handle_debug_continue() {
    if let Some(mut debugger) = get_debugger_node() {
        debugger.call(debugger::methods::DEBUG_CONTINUE, &[]);
        log::debug!("Debug: continue executed");
    }
}

/// Step over.
pub fn handle_debug_next() {
    if let Some(mut debugger) = get_debugger_node() {
        debugger.call(debugger::methods::DEBUG_NEXT, &[]);
        log::debug!("Debug: next executed");
    }
}

/// Step into.
pub fn handle_debug_step_in() {
    if let Some(mut debugger) = get_debugger_node() {
        debugger.call(debugger::methods::DEBUG_STEP, &[]);
        log::debug!("Debug: step in executed");
    }
}

/// Step out. Godot has no native step-out; falls back to continue.
pub fn handle_debug_step_out() {
    if let Some(mut debugger) = get_debugger_node() {
        debugger.call(debugger::methods::DEBUG_CONTINUE, &[]);
        log::debug!("Debug: step out (no native support, continuing)");
    }
}

/// Pause execution.
pub fn handle_debug_pause() {
    if let Some(mut debugger) = get_debugger_node() {
        debugger.call(debugger::methods::DEBUG_BREAK, &[]);
        log::debug!("Debug: pause executed");
    }
}

/// Returns the `EditorDebuggerNode` singleton, or `None` if unavailable.
fn get_debugger_node() -> Option<Gd<godot::classes::Object>> {
    let mut base = godot::classes::EditorInterface::singleton().get_base_control()?;
    base.call(
        node::methods::FIND_CHILD,
        &[
            debugger::CLASS_NAME.to_variant(),
            true.to_variant(),
            false.to_variant(),
        ],
    )
    .try_to::<Gd<godot::classes::Object>>()
    .ok()
}
