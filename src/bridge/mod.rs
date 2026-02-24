//! Godot-Vim bridge layer.
//!
//! This module integrates the pure `vim-core` engine with Godot editor APIs.
//! All `vim_core` interaction is isolated to `vim_adapter`.
//!
//! Canonical runtime flow:
//!
//! ```text
//! InputEvent -> VimKey -> controller/runtime_gateway
//!            -> vim_adapter::engine::process_key_with_policy
//!            -> effect_converter::effects_to_output
//!            -> controller::dispatch
//! ```
//!
//! Ex command runtime flow:
//!
//! ```text
//! Command source -> controller/runtime_gateway::execute_ex_command_with_visuals
//!                -> vim_adapter::engine::process_ex_command_with_context
//!                -> effect_converter::effects_to_output
//!                -> controller::dispatch
//! ```

// ═══════════════════════════════════════════════════════════════════════════════
// Clean infrastructure (no vim-core imports)
// ═══════════════════════════════════════════════════════════════════════════════

pub mod godot;
pub mod safety;
pub mod types;

// ═══════════════════════════════════════════════════════════════════════════════
// Vim adapter — all vim-core interaction
// ═══════════════════════════════════════════════════════════════════════════════

pub mod vim_adapter;

// ═══════════════════════════════════════════════════════════════════════════════
// Plugin layer (no direct vim-core execution)
// ═══════════════════════════════════════════════════════════════════════════════

pub mod entry;
pub mod global_input;
pub mod navigation;
pub mod vim_wrapper;
mod vim_wrapper_cmdline;
mod vim_wrapper_dock;
mod vim_wrapper_signals;
mod vim_wrapper_util;

// ═══════════════════════════════════════════════════════════════════════════════
// Configuration and UI
// ═══════════════════════════════════════════════════════════════════════════════

pub mod components;
pub mod settings;

// ═══════════════════════════════════════════════════════════════════════════════
// Test infrastructure
// ═══════════════════════════════════════════════════════════════════════════════
