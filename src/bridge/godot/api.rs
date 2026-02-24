//! Centralized Godot API abstraction layer.
//!
//! This module provides a single point of access for all Godot Editor APIs,
//! making it easier to maintain compatibility when Godot changes API paths.
//!
//! # Design Rationale
//! Instead of scattering Godot API calls throughout the codebase, this module:
//! - Centralizes all `EditorSettings`, `EditorInterface`, and similar API access  
//! - Documents the exact API paths used (for easier updates when Godot changes)
//! - Provides fallback defaults when running in headless/test mode
//! - Abstracts away the Godot singleton access patterns
//!
//! # Compatibility
//! This module is designed for version 4.x of Godot. When API paths change:
//! 1. Update the `SETTING_*` constants in this file
//! 2. Add version-detection guards for conditional API paths
//! 3. All consuming code uses these abstractions and remains unchanged
//!
//! # Related Modules
//! - [`super::names`] - Godot class/method name constants
//! - [`super::code_edit_ext`] - CodeEdit extension trait for fold-aware ops
//! - [`crate::bridge::vim_adapter::core::cast`] - Safe i32↔usize conversions

use godot::classes::{EditorInterface, EditorSettings, Engine};
use godot::obj::Singleton;

// ═══════════════════════════════════════════════════════════════════════════════
// Godot Version Detection
// ═══════════════════════════════════════════════════════════════════════════════

/// Godot version information for API compatibility checks.
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(dead_code)]
pub struct GodotVersion {
    pub major: u32,
    pub minor: u32,
    pub patch: u32,
}

#[allow(dead_code)]
impl GodotVersion {
    /// Gets the current Godot version from the Engine singleton.
    #[must_use]
    pub fn current() -> Self {
        let engine = Engine::singleton();
        let info = engine.get_version_info();

        Self {
            major: info
                .get("major")
                .and_then(|v| v.try_to::<u32>().ok())
                .unwrap_or(4),
            minor: info
                .get("minor")
                .and_then(|v| v.try_to::<u32>().ok())
                .unwrap_or(0),
            patch: info
                .get("patch")
                .and_then(|v| v.try_to::<u32>().ok())
                .unwrap_or(0),
        }
    }

    /// Returns true if this version is at least the given version.
    #[must_use]
    pub fn at_least(&self, major: u32, minor: u32, patch: u32) -> bool {
        (self.major, self.minor, self.patch) >= (major, minor, patch)
    }

    /// Returns true if running Godot 4.3 or later.
    #[must_use]
    pub fn is_4_3_plus(&self) -> bool {
        self.at_least(4, 3, 0)
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// EditorSettings Paths (Godot 4.x)
// Update these when Godot changes its settings structure
// ═══════════════════════════════════════════════════════════════════════════════

/// Path to indent size setting in `EditorSettings`
const SETTING_INDENT_SIZE: &str = "text_editor/behavior/indent/size";
/// Path to indent type setting (0 = tabs, 1 = spaces)
const SETTING_INDENT_TYPE: &str = "text_editor/behavior/indent/type";

// ═══════════════════════════════════════════════════════════════════════════════
// Editor Configuration Types
// ═══════════════════════════════════════════════════════════════════════════════

/// Editor configuration extracted from Godot's `EditorSettings`.
///
/// This struct is passed to pure core functions, providing configuration
/// without coupling the core to Godot APIs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, smart_default::SmartDefault)]
pub struct EditorConfig {
    /// Number of spaces per indent level (typically 4)
    #[default = 4]
    pub indent_size: usize,
    /// Whether to use tabs instead of spaces
    #[default = false]
    pub use_tabs: bool,
}

// ═══════════════════════════════════════════════════════════════════════════════
// Godot API Access Functions
// ═══════════════════════════════════════════════════════════════════════════════

/// Reads the current editor configuration from Godot's `EditorSettings`.
///
/// Returns default configuration if:
/// - Running in headless mode (no editor)
/// - `EditorInterface` or `EditorSettings` unavailable
/// - Settings have unexpected types
///
/// This function is safe to call at any time and will never panic.
#[must_use]
pub fn get_editor_config() -> EditorConfig {
    // Try to get EditorInterface singleton
    let interface = EditorInterface::singleton();

    // Try to get EditorSettings
    let Some(settings) = interface.get_editor_settings() else {
        return EditorConfig::default();
    };

    // Read indent size (default: 4)
    let indent_size = read_setting_int(&settings, SETTING_INDENT_SIZE)
        .and_then(|v| usize::try_from(v).ok())
        .unwrap_or(4);

    // Read indent type (0 = tabs, 1 = spaces)
    let use_tabs = read_setting_int(&settings, SETTING_INDENT_TYPE).is_some_and(|v| v == 0);

    EditorConfig {
        indent_size,
        use_tabs,
    }
}

/// Reads an integer setting from `EditorSettings`.
/// Returns None if the setting doesn't exist or isn't an integer.
fn read_setting_int(settings: &EditorSettings, path: &str) -> Option<i64> {
    if !settings.has_setting(path) {
        return None;
    }

    let value = settings.get_setting(path);

    // Godot Variant to i64
    value.try_to::<i64>().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = EditorConfig::default();
        assert_eq!(config.indent_size, 4);
        assert!(!config.use_tabs);
    }

    #[test]
    fn test_version_at_least() {
        let v = GodotVersion {
            major: 4,
            minor: 3,
            patch: 0,
        };
        assert!(v.at_least(4, 0, 0));
        assert!(v.at_least(4, 3, 0));
        assert!(!v.at_least(4, 4, 0));
        assert!(!v.at_least(5, 0, 0));
    }

    #[test]
    fn test_version_is_4_3_plus() {
        assert!(GodotVersion {
            major: 4,
            minor: 3,
            patch: 0
        }
        .is_4_3_plus());
        assert!(GodotVersion {
            major: 4,
            minor: 4,
            patch: 0
        }
        .is_4_3_plus());
        assert!(GodotVersion {
            major: 5,
            minor: 0,
            patch: 0
        }
        .is_4_3_plus());
        assert!(!GodotVersion {
            major: 4,
            minor: 2,
            patch: 5
        }
        .is_4_3_plus());
    }
}
