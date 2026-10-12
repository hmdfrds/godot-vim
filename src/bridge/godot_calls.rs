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

use std::cell::Cell;

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

// ── Section 3: the Godot 4.6 shortcut-API gate ──────────────────────────

/// Bound to ClassDB from Godot 4.6 only; see [`has_shortcut_api`].
const GET_SHORTCUT: &str = "get_shortcut";

thread_local! {
    /// Memoized answer to "does this build expose the shortcut API at all?".
    ///
    /// `thread_local` rather than a `static AtomicBool` because every caller is
    /// on Godot's main thread and a `Cell` needs no synchronization there.
    /// `None` means "not asked yet"; the answer cannot change within a process,
    /// since it is a property of the engine binary that dlopened us.
    static SHORTCUT_API: Cell<Option<bool>> = const { Cell::new(None) };
}

/// Ask `probe` at most once per cell, and remember the answer either way.
///
/// Split from [`has_shortcut_api`] because everything above it needs a
/// `Gd<EditorSettings>` and this does not. "Either way" is the part worth
/// pinning: a memo that only caches `true` would re-run `has_method` on every
/// keystroke that reaches the FileSystem dock **and** re-emit the warning into
/// the Output panel each time — on the one Godot version where the warning is
/// the user's only explanation of what changed.
fn memoized(cell: &Cell<Option<bool>>, probe: impl FnOnce() -> bool) -> bool {
    if let Some(known) = cell.get() {
        return known;
    }
    let answer = probe();
    cell.set(Some(answer));
    answer
}

/// Whether `EditorSettings` exposes `get_shortcut` / `get_shortcut_list`.
///
/// `addons/godot_vim/godot_vim.gdextension` declares
/// `compatibility_minimum = "4.5"`, but both methods were bound to ClassDB
/// only by commit `8806036528` ("Add ability to add new EditorSettings
/// shortcuts"), whose earliest containing tag is `4.6-stable`
/// (`git describe --contains` → `4.6-stable~977^2`). `4.5-stable`,
/// `4.5.1-stable` and `4.5.2-stable` all ship without them.
///
/// This matters because gdext generates the vararg `Object::call` as
/// `try_call(..).unwrap_or_else(|e| panic!("{e}"))`, and `Object::callp` sets
/// `CALL_ERROR_INVALID_METHOD` for an unknown method — so an unguarded call
/// **panics** on 4.5 rather than returning nil. Before this gate existed,
/// pressing `d` or `r` in the FileSystem dock on 4.5 aborted
/// `handle_input_impl` inside `panic_guard("input", …)`.
///
/// The declared fallback: every wrapper below returns `None` when the API is
/// absent, each caller declines, the key is **not** consumed, and Godot's own
/// Delete/F2 accelerators still fire in the FileSystem dock. Honest
/// degradation rather than a black hole.
///
/// `godot_warn!` rather than `log::warn!` on purpose: the default log level is
/// `Off` (`src/settings/defaults.rs`), so the logging facade would swallow the
/// one message that explains the degradation.
pub(crate) fn has_shortcut_api(settings: &Gd<EditorSettings>) -> bool {
    SHORTCUT_API.with(|cached| {
        let first_ask = cached.get().is_none();
        let available = memoized(cached, || settings.has_method(GET_SHORTCUT));
        if first_ask && !available {
            godot_warn!(
                "GodotVim: EditorSettings.get_shortcut is unavailable on this Godot build \
                 (it requires 4.6+). Shortcut delegation is disabled: FileSystem d/r fall \
                 back to Godot's own Delete/F2 accelerators, the documentation tooltip \
                 declines, and :actionlist omits editor shortcuts."
            );
        }
        available
    })
}

/// Look up an editor shortcut by its registered path.
///
/// Wraps `EditorSettings::get_shortcut` — an internal method that retrieves
/// a `Shortcut` resource by the path registered via Godot's `ED_SHORTCUT`
/// macro. Not exposed in gdext's typed API.
///
/// Returns `None` if the running build predates 4.6 (see
/// [`has_shortcut_api`]), if the shortcut path is not registered, or if the
/// variant conversion fails.
///
/// # COMPAT: `settings.try_call("get_shortcut", &[path.to_variant()])`
pub(crate) fn get_shortcut(
    settings: &mut Gd<EditorSettings>,
    path: &str,
) -> Option<Gd<godot::classes::Shortcut>> {
    if !has_shortcut_api(settings) {
        return None;
    }
    // `try_call` rather than `call` is defence in depth beyond the gate: even
    // with `has_method` true, a future signature change must never panic the
    // input handler.
    let variant = settings
        .try_call(GET_SHORTCUT, &[path.to_variant()])
        .map_err(|err| log::warn!("get_shortcut('{path}') failed: {err}"))
        .ok()?;
    variant.try_to::<Gd<godot::classes::Shortcut>>().ok()
}

/// [`get_shortcut`] for a caller with its own way on 4.5, so that asking
/// does not raise the 4.5 warning about features that have none.
pub(crate) fn get_shortcut_quiet(
    settings: &mut Gd<EditorSettings>,
    path: &str,
) -> Option<Gd<godot::classes::Shortcut>> {
    let bound = SHORTCUT_API
        .with(Cell::get)
        .unwrap_or_else(|| settings.has_method(GET_SHORTCUT));
    if !bound {
        return None;
    }
    get_shortcut(settings, path)
}

/// Every registered editor shortcut path, as Godot returns it.
///
/// Wraps `EditorSettings::get_shortcut_list`, which is bound to ClassDB by
/// the same commit as [`get_shortcut`] and therefore carries the same 4.6
/// floor. Returns the raw `Variant` because the two callers want different
/// container types out of it; `None` means "no shortcut section at all",
/// which both render as an empty list.
///
/// # COMPAT: `settings.try_call("get_shortcut_list", &[])`
pub(crate) fn get_shortcut_list(settings: &mut Gd<EditorSettings>) -> Option<Variant> {
    if !has_shortcut_api(settings) {
        return None;
    }
    settings
        .try_call("get_shortcut_list", &[])
        .map_err(|err| log::warn!("get_shortcut_list failed: {err}"))
        .ok()
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    /// What `has_shortcut_api` does either side of the `Gd<EditorSettings>`
    /// seam is Godot's; the memo in the middle is ours, and it is the part
    /// that runs at the OS key-repeat rate.
    #[test]
    fn the_capability_probe_is_asked_at_most_once() {
        for answer in [true, false] {
            let cell = Cell::new(None);
            let asks = RefCell::new(0_u32);
            for _ in 0..5 {
                let got = memoized(&cell, || {
                    *asks.borrow_mut() += 1;
                    answer
                });
                assert_eq!(got, answer);
            }
            assert_eq!(
                *asks.borrow(),
                1,
                "answer={answer}: a `false` that is not memoized re-probes and re-warns \
                 on every keystroke"
            );
        }
    }

    #[test]
    fn a_remembered_answer_is_returned_without_re_probing() {
        // The pre-seeded cell is the second and later keystrokes. `probe`
        // panicking is the assertion: it must not be reached at all.
        for seeded in [true, false] {
            let cell = Cell::new(Some(seeded));
            let got = memoized(&cell, || panic!("the probe must not run again"));
            assert_eq!(got, seeded);
            assert_eq!(cell.get(), Some(seeded), "the memo must not be disturbed");
        }
    }
}
