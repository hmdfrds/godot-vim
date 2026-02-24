//! Godot script operation helpers.
//!
//! Provides safe abstractions over Godot's script editor API,
//! encapsulating fragile `.call()` patterns in a type-safe wrapper.

use crate::bridge::godot::names::{script, script_editor, script_editor_base};
use godot::classes::{EditorInterface, Resource, ResourceSaver, Script};
use godot::global::Error;
use godot::prelude::*;

/// Edit menu option IDs for `ScriptTextEditor`.
#[allow(dead_code)]
pub mod menu_options {
    pub const EDIT_UNDO: i32 = 0;
    pub const EDIT_REDO: i32 = 1;
    pub const EDIT_CUT: i32 = 2;
    pub const EDIT_COPY: i32 = 3;
    pub const EDIT_PASTE: i32 = 4;
    pub const EDIT_SELECT_ALL: i32 = 5;
    pub const EDIT_COMPLETE: i32 = 6;
    pub const EDIT_AUTO_INDENT: i32 = 7;
    pub const EDIT_TRIM_TRAILING_WHITESPACE: i32 = 8;
    pub const EDIT_TRIM_FINAL_NEWLINES: i32 = 9;
    pub const EDIT_CONVERT_INDENT_TO_SPACES: i32 = 10;
    pub const EDIT_CONVERT_INDENT_TO_TABS: i32 = 11;
    pub const EDIT_TOGGLE_COMMENT: i32 = 12;
    pub const EDIT_MOVE_LINE_UP: i32 = 13;
    pub const EDIT_MOVE_LINE_DOWN: i32 = 14;
    pub const EDIT_INDENT: i32 = 15;
    pub const EDIT_UNINDENT: i32 = 16;
    pub const EDIT_DELETE_LINE: i32 = 17;
    pub const EDIT_DUPLICATE_SELECTION: i32 = 18;
    pub const EDIT_DUPLICATE_LINES: i32 = 19;
    pub const EDIT_PICK_COLOR: i32 = 20;
    pub const EDIT_TO_UPPERCASE: i32 = 21;
    pub const EDIT_TO_LOWERCASE: i32 = 22;
    pub const EDIT_CAPITALIZE: i32 = 23;
    pub const EDIT_EVALUATE: i32 = 24;
    pub const EDIT_TOGGLE_WORD_WRAP: i32 = 25;
    pub const EDIT_TOGGLE_FOLD_LINE: i32 = 26;
    pub const EDIT_FOLD_ALL_LINES: i32 = 27;
    pub const EDIT_CREATE_CODE_REGION: i32 = 28;
    pub const EDIT_UNFOLD_ALL_LINES: i32 = 29;
    pub const SEARCH_FIND: i32 = 30;
    pub const SEARCH_FIND_NEXT: i32 = 31;
    pub const SEARCH_FIND_PREV: i32 = 32;
    pub const SEARCH_REPLACE: i32 = 33;
    pub const SEARCH_LOCATE_FUNCTION: i32 = 34;
    pub const SEARCH_GOTO_LINE: i32 = 35;
    pub const BOOKMARK_TOGGLE: i32 = 38;
    pub const BOOKMARK_GOTO_NEXT: i32 = 39;
    pub const BOOKMARK_GOTO_PREV: i32 = 40;
    pub const BOOKMARK_REMOVE_ALL: i32 = 41;
    pub const HELP_CONTEXTUAL: i32 = 46;
    pub const LOOKUP_SYMBOL: i32 = 47;
    pub const EDIT_EMOJI_AND_SYMBOL: i32 = 48;
}

/// Safe wrapper for Godot script operations.
///
/// Abstracts away the fragile `.call()` patterns and provides
/// a clean API for script-related operations.
pub struct ScriptContext {
    script: Gd<Script>,
    path: GString,
    /// The ScriptEditorBase for the current script (needed for emitting name_changed)
    script_editor_base: Option<Gd<godot::classes::Control>>,
}

impl ScriptContext {
    /// Get current script from editor.
    ///
    /// Returns `None` if:
    /// - No script editor available
    /// - No script currently open
    /// - Script has no file path (unsaved new file)
    #[must_use]
    pub fn current() -> Option<Self> {
        let interface = EditorInterface::singleton();
        let mut script_editor = interface.get_script_editor()?;

        // Use dynamic call since Godot doesn't expose this statically
        let script = script_editor
            .call(script_editor::methods::GET_CURRENT_SCRIPT, &[])
            .try_to::<Gd<Resource>>()
            .ok()?
            .try_cast::<Script>()
            .ok()?;

        let path = script.get_path();
        if path.is_empty() {
            return None;
        }

        // Get the current ScriptEditorBase (needed for emitting name_changed)
        let script_editor_base = script_editor
            .get_current_editor()
            .map(|se| se.upcast::<godot::classes::Control>());

        Some(Self {
            script,
            path,
            script_editor_base,
        })
    }

    /// Save the script to disk.
    ///
    /// Uses `ResourceSaver` for the actual save, then emits `name_changed`
    /// signal to update the UI (removes `*` dirty marker).
    ///
    /// Returns `true` on success.
    #[must_use]
    pub fn save(&self) -> bool {
        // Sync CodeEdit text to Script source_code before saving, as the resource
        // is not automatically updated while the editor holds live changes.
        if let Some(seb) = &self.script_editor_base {
            if let Some(code_edit) =
                Self::find_code_edit_recursive(seb.clone().upcast::<godot::classes::Node>())
            {
                let current_text = code_edit.get_text();
                self.script.clone().set_source_code(&current_text);
            }
        }

        let mut saver = ResourceSaver::singleton();
        let success = saver
            .save_ex(&self.script.clone().upcast::<Resource>())
            .path(&self.path)
            .done()
            == Error::OK;

        if success {
            // Mark CodeEdit as saved to clear the undo dirty state.
            if let Some(seb) = &self.script_editor_base {
                if let Some(mut code_edit) =
                    Self::find_code_edit_recursive(seb.clone().upcast::<godot::classes::Node>())
                {
                    code_edit.tag_saved_version();
                }
            }

            // Update tab title to remove the dirty marker.
            self.emit_name_changed();
        }

        success
    }

    /// Recursively search for a CodeEdit node in the tree
    fn find_code_edit_recursive(
        node: Gd<godot::classes::Node>,
    ) -> Option<Gd<godot::classes::CodeEdit>> {
        // Check if this node is a CodeEdit
        if node.is_class("CodeEdit") {
            if let Ok(code_edit) = node.clone().try_cast::<godot::classes::CodeEdit>() {
                return Some(code_edit);
            }
        }

        // Search children
        for i in 0..node.get_child_count() {
            if let Some(child) = node.get_child(i) {
                if let Some(found) = Self::find_code_edit_recursive(child) {
                    return Some(found);
                }
            }
        }

        None
    }

    /// Emit name_changed signal on the ScriptEditorBase.
    /// This notifies the ScriptEditor to update tab names (removes `*`).
    pub fn emit_name_changed(&self) {
        if let Some(mut seb) = self.script_editor_base.clone() {
            seb.emit_signal(script_editor_base::signals::NAME_CHANGED, &[]);
        }
    }

    /// Forces the script editor to close the current tab.
    ///
    /// Uses queue_free() on the current ScriptEditorBase, matching Godot's
    /// internal _close_tab implementation (memdelete).
    ///
    /// Closes without a save prompt; call `save()` first if the file needs saving.
    pub fn close_current_tab() {
        let interface = EditorInterface::singleton();
        if let Some(script_editor) = interface.get_script_editor() {
            if let Some(current) = script_editor.get_current_editor() {
                current.upcast::<godot::classes::Node>().queue_free();
            }
        }
    }

    /// Reload script from disk, discarding unsaved changes.
    pub fn reload(&mut self) {
        self.script.call(script::methods::RELOAD, &[]);
        // Emit name_changed to update the tab title after reload
        self.emit_name_changed();
    }

    /// Check if the script has unsaved changes.
    #[allow(dead_code)]
    pub fn is_unsaved(&self) -> bool {
        if let Some(seb) = &self.script_editor_base {
            // ScriptEditorBase has is_unsaved() method
            return seb.clone().call(script_editor_base::methods::IS_UNSAVED, &[]).to::<bool>();
        }
        false
    }

    /// Returns the filesystem path of the script.
    #[allow(dead_code)]
    pub fn path(&self) -> &GString {
        &self.path
    }

    /// Save all open scripts.
    ///
    /// Iterates every open `ScriptEditorBase`, syncs its `CodeEdit` text to the
    /// script resource, saves via `ResourceSaver`, and clears the dirty marker.
    ///
    /// Returns `(saved_count, failed_count)`.
    pub fn save_all() -> (usize, usize) {
        let mut saved = 0usize;
        let mut failed = 0usize;

        // Save the current tab first — it needs CodeEdit→Script sync since
        // Godot only syncs on tab switch, not continuously.
        let current_path = if let Some(ctx) = Self::current() {
            let path = ctx.path.clone();
            if ctx.save() {
                saved += 1;
            } else {
                failed += 1;
            }
            Some(path)
        } else {
            None
        };

        // Save all other open scripts. Non-current tabs already have their
        // source_code synced (Godot calls apply_code() on tab switch),
        // so ResourceSaver is sufficient.
        let interface = EditorInterface::singleton();
        let Some(script_editor) = interface.get_script_editor() else {
            return (saved, failed);
        };

        let scripts = script_editor.get_open_scripts();
        let mut saver = ResourceSaver::singleton();
        let mut other_saved = false;

        for script in scripts.iter_shared() {
            let path = script.get_path();
            if path.is_empty() || Some(&path) == current_path.as_ref() {
                continue;
            }

            if saver
                .save_ex(&script.upcast::<Resource>())
                .path(&path)
                .done()
                == Error::OK
            {
                saved += 1;
                other_saved = true;
            } else {
                failed += 1;
            }
        }

        // For non-current editors: tag CodeEdit as saved and update tab titles.
        // Their CodeEdit content already matches disk (Godot syncs on tab switch),
        // so tag_saved_version() correctly marks the current state as saved.
        if other_saved {
            for mut seb in script_editor.get_open_script_editors().iter_shared() {
                let node = seb.clone().upcast::<godot::classes::Node>();
                if let Some(mut ce) = Self::find_code_edit_recursive(node) {
                    ce.tag_saved_version();
                }
                seb.emit_signal(script_editor_base::signals::NAME_CHANGED, &[]);
            }
        }

        (saved, failed)
    }
}
