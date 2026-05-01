//! Centralized wrappers for Godot dynamic calls (`Object::call`).
//!
//! Several Godot methods are not exposed in gdext's typed API, so they must be
//! invoked via `Object::call("method_name", &[args...])`. Scattering these
//! string literals across the codebase is fragile — a Godot rename silently
//! breaks every call site. This module quarantines each string literal behind a
//! typed Rust function so that:
//!
//! 1. Each method name appears **exactly once** in the codebase.
//! 2. Call sites get compile-time type checking on arguments and return values.
//! 3. When gdext gains typed bindings for a method, only this file changes.
//!
//! Dynamic calls that use a *runtime-variable* method name (e.g. the debugger
//! dispatch in `custom_commands.rs`) intentionally stay at their call site.

use godot::classes::{CodeEdit, EditorSettings, Tree, TreeItem};
use godot::prelude::*;

// ── Section 1: Constants ────────────────────────────────────────────────

// Internal editor class names — not part of gdext's typed hierarchy, so we
// identify them by string via `Node::is_class()`.

/// Godot's internal `CodeTextEditor` wrapper (contains a CodeEdit + minimap).
pub(crate) const CLASS_CODE_TEXT_EDITOR: &str = "CodeTextEditor";

/// Godot's internal `ShaderTextEditor` (shader variant of the script editor).
pub(crate) const CLASS_SHADER_TEXT_EDITOR: &str = "ShaderTextEditor";

/// Godot's internal `SceneTreeEditor` (the tree widget inside SceneTreeDock).
pub(crate) const CLASS_SCENE_TREE_EDITOR: &str = "SceneTreeEditor";

/// Godot's internal `EditorHelp` (the in-editor documentation viewer).
pub(crate) const CLASS_EDITOR_HELP: &str = "EditorHelp";

/// Godot's internal `SceneTreeDock` (the dock that hosts the scene tree).
pub(crate) const CLASS_SCENE_TREE_DOCK: &str = "SceneTreeDock";

/// `CodeEdit::SearchFlags::SEARCH_WHOLE_WORDS` — hardcoded because the typed
/// constant is not exposed in all gdext versions.
pub(crate) const SEARCH_WHOLE_WORDS: u32 = 2;

/// Shortcut path for the "close file" action in the script editor.
/// Registered by Godot via `ED_SHORTCUT("script_editor/close_file", ...)`.
pub(crate) const SHORTCUT_CLOSE_FILE: &str = "script_editor/close_file";

/// Shortcut path for the "show documentation" tooltip action.
/// Registered by Godot via `ED_SHORTCUT("script_text_editor/show_tooltip", ...)`.
pub(crate) const SHORTCUT_SHOW_TOOLTIP: &str = "script_text_editor/show_tooltip";

/// Shortcut path for deleting files in the FileSystem dock.
/// Registered by Godot via `ED_SHORTCUT("filesystem_dock/delete", ..., Key::DELETE)`.
pub(crate) const SHORTCUT_FS_DELETE: &str = "filesystem_dock/delete";

/// Shortcut path for renaming files in the FileSystem dock.
/// Registered by Godot via `ED_SHORTCUT("filesystem_dock/rename", ..., Key::F2)`.
pub(crate) const SHORTCUT_FS_RENAME: &str = "filesystem_dock/rename";

// ── Section 2: Typed wrapper functions ──────────────────────────────────

/// Set the search text on a `CodeEdit` for built-in search highlighting.
///
/// Wraps `CodeEdit::set_search_text` — an internal method on Godot's
/// `CodeEdit` that is not exposed in gdext's typed API.
///
/// # COMPAT: `editor.call("set_search_text", &[pattern.to_variant()])`
pub(crate) fn set_search_text(editor: &mut Gd<CodeEdit>, pattern: &str) {
    editor.call("set_search_text", &[pattern.to_variant()]);
}

/// Set the search flags on a `CodeEdit` (e.g. `SEARCH_WHOLE_WORDS`).
///
/// Wraps `CodeEdit::set_search_flags` — an internal method on Godot's
/// `CodeEdit` that is not exposed in gdext's typed API.
///
/// # COMPAT: `editor.call("set_search_flags", &[flags.to_variant()])`
pub(crate) fn set_search_flags(editor: &mut Gd<CodeEdit>, flags: u32) {
    editor.call("set_search_flags", &[flags.to_variant()]);
}

/// Dismiss the code completion hint tooltip on a `CodeEdit`.
///
/// Sends an empty string to `CodeEdit::set_code_hint`, which is Godot's
/// internal method for showing inline documentation hints. Not exposed in
/// gdext's typed API.
///
/// # COMPAT: `editor.call("set_code_hint", &["".to_variant()])`
pub(crate) fn dismiss_code_hint(editor: &mut Gd<CodeEdit>) {
    editor.call("set_code_hint", &["".to_variant()]);
}

/// Look up an editor shortcut by its registered path.
///
/// Wraps `EditorSettings::get_shortcut` — an internal method that retrieves
/// a `Shortcut` resource by the path registered via Godot's `ED_SHORTCUT`
/// macro. Not exposed in gdext's typed API.
///
/// Returns `None` if the shortcut path is not registered or the variant
/// conversion fails.
///
/// # COMPAT: `settings.call("get_shortcut", &[path.to_variant()])`
pub(crate) fn get_shortcut(
    settings: &mut Gd<EditorSettings>,
    path: &str,
) -> Option<Gd<godot::classes::Shortcut>> {
    let variant = settings.call("get_shortcut", &[path.to_variant()]);
    variant.try_to::<Gd<godot::classes::Shortcut>>().ok()
}

/// Scroll a `Tree` widget to make the given item visible.
///
/// Wraps `Tree::scroll_to_item` — present in Godot's C++ API but not
/// always exposed in gdext's typed bindings.
///
/// # COMPAT: `tree.call("scroll_to_item", &[item.to_variant()])`
pub(crate) fn scroll_to_item(tree: &mut Gd<Tree>, item: &Gd<TreeItem>) {
    tree.call("scroll_to_item", &[item.to_variant()]);
}
