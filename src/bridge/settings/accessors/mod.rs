//! Type-safe accessors for `GodotVim` settings.

use super::types::{LineNumberMode, LogLevel};

mod cursor;
mod general;
mod internal;
mod mapping;

/// Type-safe accessor for `GodotVim` settings.
///
/// All methods read directly from `EditorSettings` (no caching).
/// Settings are realtime - changes take effect on next access.
pub struct VimSettings;
