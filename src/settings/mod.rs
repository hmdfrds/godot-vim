//! GodotVim settings infrastructure — registration, reading, and typed snapshots.
//!
//! This module bridges Godot's `EditorSettings` key-value store with the typed
//! Rust configuration the engine and UI layers consume:
//!
//! - [`keys`]: setting key path constants (`plugins/GodotVim/...`).
//! - [`defaults`]: default value constants and constructors.
//! - [`registration`]: `register_all()` — ensures all keys exist with hints.
//! - [`reader`]: `read_all()` — reads all settings into a [`SettingsSnapshot`].
//! - [`snapshot`]: [`SettingsSnapshot`] struct and [`LineNumberMode`] enum.
//!
//! The plugin calls `register_all` once in `enter_tree`, then `read_all` to
//! create a snapshot that is pushed to the engine and UI.

pub(crate) mod defaults;
mod keys;
pub(crate) mod reader;
pub(crate) mod registration;
mod snapshot;

pub(crate) use snapshot::CursorSettings;
pub(crate) use snapshot::FileAccessScope;
pub(crate) use snapshot::InccommandMode;
pub(crate) use snapshot::LineNumberMode;
pub(crate) use snapshot::ProjectVimrc;
pub(crate) use snapshot::SettingsSnapshot;
pub(crate) use snapshot::ShellExecution;
pub(crate) use snapshot::StatusBarColors;
