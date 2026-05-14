//! GodotVim — Vim emulation for Godot's CodeEdit via GDExtension.
//!
//! This crate is a GDExtension plugin that wires **vim-core** (a standalone
//! Vim engine in Rust) into Godot's script editor. The major subsystems:
//!
//! - [`bridge`] — CodeEdit <-> vim-core document/input translation.
//! - [`effects`] — Applies engine effects (edits, cursor, scroll) to CodeEdit.
//! - [`host`] — Fulfills engine host requests (file I/O, clipboard, custom commands).
//! - [`controller`] — Per-editor lifecycle: owns the engine instance, pumps the
//!   input -> engine -> effects -> host loop each frame. Contains sub-modules
//!   for completion interception, key passthrough, performance tracking, and
//!   vimdebug state.
//! - [`config`] — `.godot-vimrc` parsing, preset management, and mapping dialog.
//! - [`ui`] — Status bar, cursor overlay (GLSL shader), and line number gutter.
//! - [`settings`] — EditorSettings registration and typed accessors.
//! - [`navigation`] — Cross-panel `Ctrl+hjkl` focus movement and dock keyboard nav.
//! - [`plugin`] — [`GodotVimCore`] node that manages controller lifecycle and input routing.
//!
//! # Keystroke Data Flow
//!
//! ```text
//! Godot InputEvent
//!   │
//!   ▼
//! plugin::input  ──parse──▶  bridge::input::parse_godot_key
//!   │                              │
//!   ▼                              ▼
//! controller::process_single_key   KeyEvent
//!   │
//!   ├─1─▶ bridge::godot_host::refresh_from_editor  (sync text/cursor/viewport)
//!   │
//!   ├─2─▶ VimSession<GodotHost>::process_key(KeyEvent)
//!   │         │
//!   │         ▼
//!   │     GodotHost::apply_effects  →  effects::dispatch (two-pass)
//!   │
//!   ├─3─▶ effects::dispatch  (two-pass: text mutations, then cursor/scroll/mode)
//!   │         │
//!   │         ▼
//!   │     bridge::port::TextEditorPort  →  Gd<CodeEdit> mutations
//!   │
//!   ├─4─▶ host::dispatch  (file I/O, clipboard, shell, custom commands)
//!   │
//!   └─5─▶ ui::coordinator::update(ui_snapshot())  →  status bar, cursor overlay
//! ```
//!
//! # Reading Guide
//!
//! To understand the codebase, follow the keystroke pipeline:
//! 1. [`bridge`] — how Godot types become engine types (start with `input.rs`, then `codec.rs`)
//! 2. [`controller`] — the orchestration loop (`process.rs` is the entry point)
//! 3. [`effects`] — how engine output becomes CodeEdit mutations (`dispatch.rs`)
//! 4. [`host`] — how the engine requests services from the shell (`dispatch.rs`)
//! 5. [`ui`] — how state becomes pixels (`coordinator.rs`, `snapshot.rs`)
//!
//! Cross-cutting modules (`config`, `settings`, `state`, `safety`) can be read
//! on demand as the pipeline modules reference them.

use godot::prelude::*;

// Pipeline modules (ordered by keystroke data flow: input -> engine -> effects -> host -> ui).
mod bridge;
mod controller;
mod effects;
mod host;
mod multi_cursor;
mod ui;

// Cross-cutting concerns (referenced on demand by the pipeline modules above).
mod config;
mod logging;
mod navigation;
mod plugin;
mod safety;
mod scene_tree;
mod settings;
mod state;
mod types;

#[cfg(test)]
mod testing;

struct GodotVimExt;

#[gdextension]
// SAFETY: The gdext `#[gdextension]` macro requires an unsafe impl to register
// this crate as a GDExtension library. The safety contract holds because:
//   1. All Godot callbacks run on the main thread (no Send/Sync issues).
//   2. All `Gd<T>` references are scoped to their callback lifetimes.
//   3. Rust panics are caught by `safety::panic_guard` at every signal handler
//      boundary, preventing unwinding across the FFI.
unsafe impl ExtensionLibrary for GodotVimExt {
    fn on_level_init(level: InitLevel) {
        // Scene is the earliest InitLevel where Godot's print macros work;
        // calling godot_print! at Server or Core level panics.
        if level == InitLevel::Scene {
            logging::init();
        }
    }
}
