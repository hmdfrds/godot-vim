//! `EditorHost` trait — testable abstraction over Godot editor operations.
//!
//! File I/O handlers (`:w`, `:e`, `:q`, `:wq`) have complex decision trees with
//! security implications (scope validation, symlink detection, force behavior).
//! This trait extracts exactly the Godot API surface those handlers need, enabling
//! `MockEditorHost` for deterministic unit testing without a running Godot instance.

use super::error::HostError;

/// Narrow abstraction over Godot editor operations used by file I/O handlers.
///
/// Methods map 1:1 to Godot API calls. The surface is deliberately minimal —
/// only operations that `:w`, `:e`, `:q`, `:wq` actually need are exposed.
pub(crate) trait EditorHost {
    fn get_text(&self) -> String;

    /// Returns the current script's `res://` path, or `None` if no script is open.
    fn current_script_path(&self) -> Option<String>;

    fn is_modified(&self) -> bool;

    /// Marks the buffer clean (version == saved_version). After this call,
    /// `is_modified()` returns `false`.
    fn tag_saved_version(&mut self);

    fn set_text(&mut self, text: &str);

    /// Closes the current script tab via ScriptEditor's own close pipeline.
    fn close_tab(&mut self);

    /// Emits `name_changed` on the ScriptEditorBase to refresh the tab title.
    fn notify_name_changed(&self);

    /// Save pipeline: editor text -> Script resource -> ResourceSaver -> disk.
    /// Returns the actual save path, or a typed `HostError`.
    fn save_script(&mut self, explicit_path: Option<&str>) -> Result<String, HostError>;

    /// Open a script in the editor via ResourceLoader -> edit_script().
    fn open_script(&mut self, path: &str) -> Result<(), HostError>;

    /// Read file contents. Routes through Godot's `FileAccess` for virtual
    /// paths (`res://`, `user://`) and `std::fs` for filesystem paths.
    fn read_file(&self, path: &str) -> Result<String, HostError>;

    /// Update the in-memory Script resource without saving to disk. Used by
    /// `reload_from_disk` to keep the Script resource in sync after a reload.
    fn update_script_source(&mut self, text: &str);
}

mod godot_impl {
    use compact_str::CompactString;
    use godot::classes::file_access::ModeFlags;
    use godot::classes::{
        CodeEdit, EditorInterface, FileAccess, MenuButton, PopupMenu, ResourceLoader,
        ResourceSaver, Script, ScriptEditorBase,
    };
    use godot::prelude::*;

    use super::EditorHost;
    use crate::host::error::HostError;
    use crate::scene_tree::{find_descendant_by, MAX_DISCOVERY_DEPTH};

    /// Production `EditorHost` wrapping a live `Gd<CodeEdit>`.
    pub(crate) struct GodotEditorHost<'a>(pub(crate) &'a mut Gd<CodeEdit>);

    fn current_script() -> Option<Gd<Script>> {
        let editor_iface = EditorInterface::singleton();
        let mut script_editor = editor_iface.get_script_editor()?;
        let script = script_editor.get_current_script()?;
        if script.get_path().is_empty() {
            return None;
        }
        Some(script)
    }

    fn current_script_editor_base() -> Option<Gd<ScriptEditorBase>> {
        let editor_iface = EditorInterface::singleton();
        let script_editor = editor_iface.get_script_editor()?;
        script_editor.get_current_editor()
    }

    /// Trigger ScriptEditor's native "File → Close" action.
    ///
    /// Godot's `ScriptEditor::_close_tab()` manages history, menu bar sync,
    /// script list updates, and layout persistence — none of which happen if
    /// we just `queue_free()` the ScriptEditorBase. Since `_close_tab()` is
    /// private (not exposed via ClassDB), we reach it through the same public
    /// entry point Godot's own UI uses: emitting `id_pressed` on the File
    /// menu's PopupMenu with the "Close" item's ID.
    ///
    /// The emission is **deferred** (`call_deferred`) so the close happens at
    /// end-of-frame. This is critical: Godot's `_close_tab()` calls
    /// `memdelete()` (synchronous deletion) on the ScriptEditorBase and its
    /// child CodeEdit. If we emitted immediately, the CodeEdit would be freed
    /// while our `process_cycle` still holds a reference to it, causing a
    /// use-after-free panic on the subsequent `ui_snapshot()` / `ui.update()`.
    fn trigger_script_editor_close() {
        let editor_iface = EditorInterface::singleton();
        let Some(script_editor) = editor_iface.get_script_editor() else {
            log::warn!("close_tab: no script editor available");
            return;
        };

        // Find the "File" MenuButton among ScriptEditor's descendants.
        // Tree: ScriptEditor → VBoxContainer → HBoxContainer (menu_hb) → MenuButton("File")
        let file_menu: Option<Gd<MenuButton>> = find_descendant_by(
            &script_editor.clone().upcast(),
            MAX_DISCOVERY_DEPTH,
            &|node| {
                let mb = node.clone().try_cast::<MenuButton>().ok()?;
                (mb.get_text().to_string() == "File").then_some(mb)
            },
        );

        let Some(file_menu) = file_menu else {
            // Fallback: scan all MenuButton descendants for a popup containing
            // the "Close" item (handles translated UIs).
            if let Some((popup, close_id)) = find_close_item_in_any_menu(&script_editor) {
                defer_emit_id_pressed(popup, close_id);
                return;
            }
            log::warn!("close_tab: could not find File menu in ScriptEditor");
            return;
        };

        let popup: Gd<PopupMenu> = file_menu.get_popup().expect("MenuButton always has a popup");

        // Find the "Close" item by scanning for text "Close" (first exact match).
        let count = popup.get_item_count();
        for i in 0..count {
            if popup.get_item_text(i).to_string() == "Close" {
                let id = popup.get_item_id(i);
                defer_emit_id_pressed(popup, id);
                return;
            }
        }

        log::warn!("close_tab: 'Close' item not found in File menu");
    }

    /// Emit `id_pressed` on a PopupMenu at end-of-frame via `call_deferred`.
    fn defer_emit_id_pressed(popup: Gd<PopupMenu>, id: i32) {
        popup.upcast::<godot::classes::Node>().call_deferred(
            "emit_signal",
            &["id_pressed".to_variant(), id.to_variant()],
        );
    }

    /// Fallback scanner: find any MenuButton descendant whose popup has a "Close"
    /// item. Handles non-English Godot UIs where the File menu text is translated.
    fn find_close_item_in_any_menu(
        script_editor: &Gd<godot::classes::ScriptEditor>,
    ) -> Option<(Gd<PopupMenu>, i32)> {
        find_descendant_by(
            &script_editor.clone().upcast(),
            MAX_DISCOVERY_DEPTH,
            &|node| {
                let mb = node.clone().try_cast::<MenuButton>().ok()?;
                let popup = mb.get_popup()?;
                let count = popup.get_item_count();
                for i in 0..count {
                    let text = popup.get_item_text(i).to_string();
                    if text == "Close" {
                        let id = popup.get_item_id(i);
                        return Some((popup.clone(), id));
                    }
                }
                None
            },
        )
    }

    fn emit_name_changed() {
        if let Some(mut base) = current_script_editor_base() {
            let err = base.emit_signal(&StringName::from("name_changed"), &[]);
            if err != godot::global::Error::OK {
                log::warn!("Failed to emit 'name_changed' signal: {err:?}");
            }
        }
    }

    const MAX_READ_FILE_SIZE: usize = 10 * 1024 * 1024;

    fn read_via_godot(path: &str) -> Result<String, HostError> {
        let file = FileAccess::open(&GString::from(path), ModeFlags::READ)
            .ok_or_else(|| HostError::CantOpenFile {
                path: CompactString::from(path),
                detail: None,
            })?;
        let length = usize::try_from(file.get_length()).unwrap_or(usize::MAX);
        if length > MAX_READ_FILE_SIZE {
            return Err(HostError::CantOpenFile {
                path: CompactString::from(path),
                detail: Some(CompactString::from(format!(
                    "File too large (>10MB): {} bytes",
                    length
                ))),
            });
        }
        let text = file.get_as_text().to_string();
        Ok(text)
    }

    impl<'a> EditorHost for GodotEditorHost<'a> {
        fn get_text(&self) -> String {
            self.0.get_text().to_string()
        }

        fn current_script_path(&self) -> Option<String> {
            current_script().map(|s| s.get_path().to_string())
        }

        fn is_modified(&self) -> bool {
            let Some(base) = current_script_editor_base() else {
                return false;
            };
            let Some(editor) = base.get_base_editor() else {
                return false;
            };
            let Ok(text_edit) = editor.try_cast::<godot::classes::TextEdit>() else {
                return true;
            };
            text_edit.get_version() != text_edit.get_saved_version()
        }

        fn tag_saved_version(&mut self) {
            self.0.tag_saved_version();
        }

        fn set_text(&mut self, text: &str) {
            self.0.set_text(&GString::from(text));
        }

        fn close_tab(&mut self) {
            trigger_script_editor_close();
        }

        fn notify_name_changed(&self) {
            emit_name_changed();
        }

        fn save_script(&mut self, explicit_path: Option<&str>) -> Result<String, HostError> {
            let mut script =
                current_script().ok_or(HostError::NoFileName)?;

            let text = self.0.get_text();
            script.set_source_code(&text);

            let save_path = match explicit_path {
                Some(p) => p.to_string(),
                None => {
                    let script_path = script.get_path().to_string();
                    if script_path.is_empty() {
                        return Err(HostError::NoFileName);
                    }
                    script_path
                }
            };

            let original_path = script.get_path().to_string();

            let err = ResourceSaver::singleton()
                .save_ex(&script)
                .path(&GString::from(&save_path))
                .done();
            if err != godot::global::Error::OK {
                return Err(HostError::WriteFailed {
                    path: CompactString::from(&save_path),
                    detail: Some(CompactString::from(format!("{err:?}"))),
                });
            }

            if save_path == original_path || explicit_path.is_none() {
                self.0.tag_saved_version();
                emit_name_changed();
            }
            log::info!("file::saved: {}", save_path);

            Ok(save_path)
        }

        fn open_script(&mut self, path: &str) -> Result<(), HostError> {
            let mut loader = ResourceLoader::singleton();
            let resource = loader.load_ex(&GString::from(path)).done();
            match resource {
                Some(res) => {
                    if let Ok(script) = res.try_cast::<Script>() {
                        EditorInterface::singleton().edit_script(&script);
                        Ok(())
                    } else {
                        Err(HostError::CantOpenFile {
                            path: CompactString::from(path),
                            detail: Some(CompactString::from("Resource is not a script")),
                        })
                    }
                }
                None => Err(HostError::CantOpenFile {
                    path: CompactString::from(path),
                    detail: None,
                }),
            }
        }

        fn read_file(&self, path: &str) -> Result<String, HostError> {
            if crate::host::file::is_godot_path(path) {
                read_via_godot(path)
            } else {
                crate::host::file::check_fs_file_size(path)?;
                std::fs::read_to_string(path)
                    .map_err(|e| HostError::CantOpenFile {
                        path: CompactString::from(path),
                        detail: Some(CompactString::from(e.to_string())),
                    })
            }
        }

        fn update_script_source(&mut self, text: &str) {
            if let Some(mut script) = current_script() {
                script.set_source_code(&GString::from(text));
            }
        }
    }
}

pub(crate) use godot_impl::GodotEditorHost;

#[cfg(test)]
pub(super) mod mock {
    use compact_str::CompactString;
    use super::EditorHost;
    use crate::host::error::HostError;
    use std::collections::HashMap;

    /// Lifecycle state of the mock buffer, replacing three booleans
    /// (`modified`, `saved`, `closed`) whose 8 combinations included 5
    /// illegal states (e.g. `modified=true, closed=true, saved=false`).
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub(in crate::host) enum MockBufferState {
        /// Clean buffer — no unsaved changes, not yet saved or closed.
        Unmodified,
        /// Buffer has unsaved changes.
        Modified,
        /// `tag_saved_version()` was called — buffer is clean and persisted.
        Saved,
        /// `close_tab()` was called — buffer is gone.
        Closed,
    }

    /// Test double for `EditorHost`. All fields are public for direct scenario setup.
    pub(in crate::host) struct MockEditorHost {
        pub text: String,
        pub script_path: Option<String>,
        pub buffer_state: MockBufferState,
        /// Audit trail: `true` if `tag_saved_version()` was ever called.
        /// Separate from `buffer_state` because a subsequent `close_tab()`
        /// transitions the state to `Closed`, losing the save information.
        pub save_called: bool,
        /// Override for `save_script()`. `None` = default behavior.
        pub save_result: Option<Result<String, HostError>>,
        /// Override for `open_script()`. `None` = default behavior.
        pub open_result: Option<Result<(), HostError>>,
        /// Virtual filesystem for `read_file()`.
        pub files: HashMap<String, String>,
        /// Records the path passed to `open_script()`.
        pub opened_path: Option<String>,
        /// Records the text passed to `update_script_source()`.
        pub script_source_updated: Option<String>,
    }

    impl MockEditorHost {
        pub fn new(text: &str, script_path: Option<&str>) -> Self {
            Self {
                text: text.to_string(),
                script_path: script_path.map(|s| s.to_string()),
                buffer_state: MockBufferState::Unmodified,
                save_called: false,
                save_result: None,
                open_result: None,
                files: HashMap::new(),
                opened_path: None,
                script_source_updated: None,
            }
        }
    }

    impl EditorHost for MockEditorHost {
        fn get_text(&self) -> String {
            self.text.clone()
        }

        fn current_script_path(&self) -> Option<String> {
            self.script_path.clone()
        }

        fn is_modified(&self) -> bool {
            matches!(self.buffer_state, MockBufferState::Modified)
        }

        fn tag_saved_version(&mut self) {
            self.buffer_state = MockBufferState::Saved;
            self.save_called = true;
        }

        fn set_text(&mut self, text: &str) {
            self.text = text.to_string();
        }

        fn close_tab(&mut self) {
            self.buffer_state = MockBufferState::Closed;
        }

        fn notify_name_changed(&self) {
            // No-op: trait requires `&self` (matching Godot's signal API), so
            // we cannot track calls without interior mutability. The important
            // behaviors (save, close, tag_saved_version) are `&mut self` and
            // fully trackable.
        }

        fn save_script(&mut self, explicit_path: Option<&str>) -> Result<String, HostError> {
            if let Some(ref result) = self.save_result {
                let r = result.clone();
                if r.is_ok() {
                    // Mirror real behavior: only tag saved when saving to the
                    // script's own path (not when `:w other.gd`).
                    let saved_path = r.as_ref().unwrap();
                    let is_same_path = self.script_path.as_deref() == Some(saved_path.as_str());
                    if is_same_path || explicit_path.is_none() {
                        self.buffer_state = MockBufferState::Saved;
                        self.save_called = true;
                    }
                }
                return r;
            }
            let path = match explicit_path {
                Some(p) => p.to_string(),
                None => {
                    match &self.script_path {
                        Some(sp) if !sp.is_empty() => sp.clone(),
                        _ => return Err(HostError::NoFileName),
                    }
                }
            };
            self.buffer_state = MockBufferState::Saved;
            self.save_called = true;
            Ok(path)
        }

        fn open_script(&mut self, path: &str) -> Result<(), HostError> {
            self.opened_path = Some(path.to_string());
            if let Some(ref result) = self.open_result {
                return result.clone();
            }
            Ok(())
        }

        fn read_file(&self, path: &str) -> Result<String, HostError> {
            if let Some(content) = self.files.get(path) {
                return Ok(content.clone());
            }
            // Non-Godot paths fall through to real filesystem so tests can
            // use actual temp files.
            if !path.starts_with("res://") && !path.starts_with("user://") {
                crate::host::file::check_fs_file_size(path)?;
                std::fs::read_to_string(path)
                    .map_err(|e| HostError::CantOpenFile {
                        path: CompactString::from(path),
                        detail: Some(CompactString::from(e.to_string())),
                    })
            } else {
                Err(HostError::CantOpenFile {
                    path: CompactString::from(path),
                    detail: None,
                })
            }
        }

        fn update_script_source(&mut self, text: &str) {
            self.script_source_updated = Some(text.to_string());
        }
    }
}
