//! Borrowed context that splits [`super::VimController`] into fine-grained fields.
//!
//! Rust's borrow checker cannot split a `&mut self` into independent field
//! borrows across function boundaries. `ProcessContext` solves this by
//! destructuring the controller into individually-borrowed fields, allowing
//! helpers to accept only the subset they need (e.g., `engine` + `state`)
//! without blocking access to unrelated fields (`perf`, `vimdebug`).
//!
//! Created once per processing pipeline via [`super::VimController::as_process_context()`]
//! and threaded through the call chain by `&mut`. Field names mirror
//! `VimController`'s fields exactly for mechanical migration.

use std::collections::HashSet;

use godot::prelude::*;
use vim_core::effects::Effect;
use vim_core::execution::VimEngine;
use vim_core::keymap::KeyEvent;

use crate::effects::UndoDepth;
use crate::host::SecurityPolicy;
use crate::state::ShellState;

use super::perf::PerfTracker;
use super::vimdebug::VimdebugState;
use super::PendingUiAction;

/// Borrowed view of every [`super::VimController`] field needed by the
/// keystroke-to-effects processing pipeline.
///
/// All fields are `pub(super)` — only sibling modules inside `controller/`
/// may access them.
pub(super) struct ProcessContext<'a> {
    pub(super) engine: &'a mut VimEngine,
    pub(super) state: &'a mut ShellState,
    /// Tracks nesting depth of Godot's `begin/end_complex_operation` for
    /// undo group balance enforcement.
    pub(super) undo_depth: &'a mut UndoDepth,
    /// Cached document text tagged with `InstanceId`; self-invalidates on
    /// buffer switch or text mutation to avoid stale reads.
    pub(super) persistent_text: &'a mut Option<(InstanceId, String)>,
    pub(super) vimdebug: &'a mut VimdebugState,
    /// Pass-2 effects deferred by vimdebug step-mode for interactive inspection.
    pub(super) pending_step_effects: &'a mut Option<Vec<Effect>>,
    /// Spans all drain paths within a single `process_cycle` to catch runaway
    /// recursion that per-drain counters miss (e.g., `:norm` calling drain).
    pub(super) operations_this_cycle: &'a mut u32,
    pub(super) perf: &'a mut PerfTracker,
    /// Set by intercepted commands; drained by the plugin after `process_cycle`.
    pub(super) pending_ui_action: &'a mut Option<PendingUiAction>,
    /// Read-only — never mutated during processing.
    pub(super) security_policy: &'a SecurityPolicy,
    pub(super) highlight_yank_duration_ms: u32,
    /// Keys that bypass Vim entirely (read-only).
    pub(super) passthrough_keys: &'a HashSet<KeyEvent>,
    /// Whether Godot's native code completion should auto-trigger on typing.
    pub(super) code_complete_enabled: bool,
    /// Clipboard abstraction for register sync and clipboard read operations.
    pub(super) clipboard: &'a mut dyn crate::bridge::clipboard::ClipboardPort,
}
