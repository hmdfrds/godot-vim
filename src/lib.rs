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
//! - [`ftplugin`]: filetype detection and the per-language `:setlocal` lines.
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
mod actions;
mod bridge;
mod controller;
mod effects;
mod ftplugin;
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

#[cfg(test)]
mod tests {
    /// gdext refuses to start on a Godot older than the API it was compiled
    /// against (`godot-ffi/src/interface_init.rs`, whole version triple), so
    /// that API must be exactly the oldest Godot the manifest admits.
    #[test]
    fn godot_api_matches_compatibility_minimum() {
        let manifest = include_str!("../addons/godot_vim/godot_vim.gdextension");
        let minimum = manifest
            .lines()
            .find_map(|line| {
                let (key, value) = line.split_once('=')?;
                (key.trim() == "compatibility_minimum").then(|| value.trim().trim_matches('"'))
            })
            .expect("compatibility_minimum missing from godot_vim.gdextension");
        // Godot reads up to three dot-separated parts; a missing part is 0.
        let mut parts = minimum.split('.').map(|p| {
            p.parse::<u8>()
                .expect("compatibility_minimum is not a version")
        });
        let mut next = || parts.next().unwrap_or(0);
        let minimum = (next(), next(), next());

        assert_eq!(
            godot::init::GdextBuild::godot_static_version_triple(),
            minimum,
            "godot is built against a different API than compatibility_minimum; \
             set the matching api-4-N feature on the godot dependency"
        );
    }

    fn rust_sources() -> Vec<(std::path::PathBuf, String)> {
        fn visit(dir: &std::path::Path, out: &mut Vec<(std::path::PathBuf, String)>) {
            for entry in std::fs::read_dir(dir).expect("read src") {
                let path = entry.expect("dir entry").path();
                if path.is_dir() {
                    visit(&path, out);
                } else if path.extension().is_some_and(|e| e == "rs") {
                    let src = std::fs::read_to_string(&path).expect("read file");
                    out.push((path, src));
                }
            }
        }
        let mut out = Vec::new();
        visit(
            &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src"),
            &mut out,
        );
        out
    }

    /// gdext 0.6 makes a non-tool class an inert placeholder in the editor,
    /// where every class of this plugin runs.
    #[test]
    fn every_godot_class_is_a_tool_class() {
        let mut missing = Vec::new();
        for (path, src) in rust_sources() {
            let mut lines = src.lines().enumerate();
            while let Some((i, line)) = lines.next() {
                if line.trim() == "#[derive(GodotClass)]" {
                    let class = lines.next().map_or("", |(_, l)| l.trim());
                    if !class.starts_with("#[class(tool") {
                        missing.push(format!("{}:{}", path.display(), i + 1));
                    }
                }
            }
        }
        assert!(
            missing.is_empty(),
            "add `tool` to these #[class(...)] attributes: {missing:?}"
        );
    }
}
