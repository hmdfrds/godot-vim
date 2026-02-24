//! `GodotVim` settings integration with Godot's `EditorSettings`.
//!
//! Settings appear at: Editor → Editor Settings → Plugins → GodotVim
//!
//! This module provides:
//! - Registration of settings in `EditorSettings` on startup
//! - Type-safe access to settings values
//! - Default values and property hints for the Inspector
//!
//! # Module Structure
//!
//! - [`keys`] - Setting key constants (paths in `EditorSettings`)
//! - [`defaults`] - Default values for all settings
//! - [`types`] - Type definitions (`LogLevel`)
//! - [`presets`] - Curated list of recommended mappings
//! - [`registration`] - Registration logic for `EditorSettings`
//! - [`accessors`] - Type-safe `VimSettings` accessor struct

pub mod accessors;
pub mod defaults;
pub mod keys;
pub mod presets;
pub mod registration;
pub mod types;

// Re-export commonly used items at the module root
pub use accessors::VimSettings;
pub use registration::register_settings;
pub use types::LogLevel;

/// Synchronize all settings from `ProjectSettings` to runtime state.
///
/// This must be called:
/// - Once at startup after `register_settings()`
/// - Whenever settings may have changed (e.g., on editor focus)
///
/// This bridges the gap between `ProjectSettings` (persistent) and global statics (runtime).
pub fn sync_all_settings() {
    // Sync log level to global logger
    let level = VimSettings::log_level();
    let filter = match level {
        LogLevel::Error => log::LevelFilter::Error,
        LogLevel::Warn => log::LevelFilter::Warn,
        LogLevel::Info => log::LevelFilter::Info,
        LogLevel::Debug => log::LevelFilter::Debug,
        LogLevel::Trace => log::LevelFilter::Trace,
        LogLevel::Off => log::LevelFilter::Off,
    };
    log::set_max_level(filter);
}
