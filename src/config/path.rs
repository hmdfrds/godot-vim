//! Config file path resolution.
//!
//! Resolves the active `.godot-vimrc` config file path using a two-tier
//! lookup: project-level (`res://`) overrides user-level (`user://`).

use godot::classes::FileAccess;
use godot::prelude::*;

const PROJECT_PATH: &str = "res://.godot-vimrc";
const USER_PATH: &str = "user://.godot-vimrc";

/// Resolved config path and provenance. `is_project_level` drives the
/// sandbox policy: project-level configs are untrusted by default.
#[derive(Debug)]
pub(crate) struct ResolvedConfig {
    pub(crate) path: String,
    pub(crate) is_project_level: bool,
}

/// Resolve the active config file path.
///
/// Priority: explicit EditorSettings override > `res://.godot-vimrc` > `user://.godot-vimrc`.
pub(crate) fn resolve(setting_override: &str) -> ResolvedConfig {
    if !setting_override.is_empty() {
        // User-configured override is treated as trusted (they chose the path).
        let resolved = ResolvedConfig {
            path: setting_override.to_string(),
            is_project_level: false,
        };
        log::debug!(
            "config::resolve: path='{}' is_project={}",
            resolved.path,
            resolved.is_project_level
        );
        return resolved;
    }

    if FileAccess::file_exists(&GString::from(PROJECT_PATH)) {
        let resolved = ResolvedConfig {
            path: PROJECT_PATH.to_string(),
            is_project_level: true,
        };
        log::debug!(
            "config::resolve: path='{}' is_project={}",
            resolved.path,
            resolved.is_project_level
        );
        return resolved;
    }

    let resolved = ResolvedConfig {
        path: USER_PATH.to_string(),
        is_project_level: false,
    };
    log::debug!(
        "config::resolve: path='{}' is_project={}",
        resolved.path,
        resolved.is_project_level
    );
    resolved
}
