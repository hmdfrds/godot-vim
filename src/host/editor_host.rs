//! `EditorHost` trait — testable abstraction over Godot editor operations.
//!
//! File I/O handlers (`:w`, `:e`, `:q`, `:wq`) have complex decision trees with
//! security implications (scope validation, symlink detection, force behavior).
//! This trait extracts exactly the Godot API surface those handlers need, enabling
//! `MockEditorHost` for deterministic unit testing without a running Godot instance.

use godot::obj::InstanceId;

use super::error::HostError;

/// Narrow abstraction over Godot editor operations used by file I/O handlers.
///
/// Methods map 1:1 to Godot API calls. The surface is deliberately minimal —
/// only operations that `:w`, `:e`, `:q`, `:wq` actually need are exposed.
pub(crate) trait EditorHost {
    fn get_text(&self) -> String;

    /// The `res://` path of the script the attached CodeEdit edits: `None`
    /// when no script is open or the CodeEdit is outside the script editor.
    fn current_script_path(&self) -> Option<String>;

    /// Whether the attached CodeEdit has unsaved changes: its version against
    /// its saved version, the check Godot makes for script and shader tabs.
    fn is_modified(&self) -> bool;

    /// Marks the buffer clean (version == saved_version). After this call,
    /// `is_modified()` returns `false`.
    fn tag_saved_version(&mut self);

    fn set_text(&mut self, text: &str);

    /// Closes the attached CodeEdit's tab through Godot's own File > Close,
    /// at the end of the frame. Refuses with `HostError::NotClosed`, closing
    /// nothing, when no File menu of Godot's would close this editor.
    fn close_tab(&mut self) -> Result<(), HostError>;

    /// The refusal `close_tab` would give now, without closing anything.
    fn can_close(&self) -> Result<(), HostError>;

    /// Emits `name_changed` on the ScriptEditorBase to refresh the tab title.
    fn notify_name_changed(&self);

    /// Save pipeline: editor text -> Script resource -> ResourceSaver -> disk.
    /// Returns the actual save path, or a typed `HostError`.
    ///
    /// The one sink for `:w`, `:wq`, `:x`, `ZZ`, `:wqa` and `:save`: refuses
    /// with `HostError::NotCurrentScript` unless the attached CodeEdit is the
    /// editor of the script editor's current script, the script it writes.
    fn save_script(&mut self, explicit_path: Option<&str>) -> Result<String, HostError>;

    /// Open a script in the editor via ResourceLoader -> edit_script().
    fn open_script(&mut self, path: &str) -> Result<(), HostError>;

    /// Read file contents. Routes through Godot's `FileAccess` for virtual
    /// paths (`res://`, `user://`) and `std::fs` for filesystem paths.
    fn read_file(&self, path: &str) -> Result<String, HostError>;

    /// Update the in-memory Script resource without saving to disk. Used by
    /// `reload_from_disk` to keep the Script resource in sync after a reload.
    /// Like `save_script`, it touches only the attached CodeEdit's script.
    fn update_script_source(&mut self, text: &str);
}

/// Whether the attached CodeEdit is `owner`, the base editor of the script
/// editor's current tab; `None` means there is no current tab.
fn is_attached_owner(owner: Option<InstanceId>, attached: InstanceId) -> bool {
    owner == Some(attached)
}

/// The path of the script editor's current script, when `editor` is its editor.
pub(crate) fn attached_script_path(
    editor: &godot::obj::Gd<godot::classes::CodeEdit>,
) -> Option<String> {
    godot_impl::attached_script(editor)
        .ok()
        .map(|script| script.get_path().to_string())
}

mod godot_impl {
    use compact_str::CompactString;
    use godot::classes::file_access::ModeFlags;
    use godot::classes::{
        CodeEdit, EditorInterface, FileAccess, InputEventShortcut, MenuButton, ResourceLoader,
        ResourceSaver, Script, ScriptEditorBase, Shortcut, Viewport,
    };
    use godot::prelude::*;

    use super::EditorHost;
    use crate::bridge::godot_calls::{self, SHORTCUT_CLOSE_FILE};
    use crate::host::error::HostError;

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

    /// The script editor's current script, only when `editor` is its editor.
    /// godot-vim attaches to any CodeEdit (#6, #20); the Shader Editor's or an
    /// addon's text must never be written into the current script.
    pub(super) fn attached_script(editor: &Gd<CodeEdit>) -> Result<Gd<Script>, HostError> {
        let owner = current_script_editor_base().and_then(|base| base.get_base_editor());
        if !super::is_attached_owner(owner.map(|o| o.instance_id()), editor.instance_id()) {
            return Err(HostError::NotCurrentScript);
        }
        current_script().ok_or(HostError::NoFileName)
    }

    /// Names of Godot's Close shortcuts, for 4.5, where
    /// `EditorSettings.get_shortcut` is unbound: `script_editor/close_file`
    /// and the 4.5 Shader Editor's `shader_editor/close_file`. A Shortcut
    /// keeps its English name in every editor language.
    const CLOSE_SHORTCUT_NAMES: [&str; 2] = ["Close", "Close File"];

    /// The Close shortcut of the File menu that Godot hands a shortcut to
    /// from `editor`, with the viewport to push it into.
    ///
    /// Godot gives a shortcut to a visible menu only when the focus owner is
    /// inside the menu's shortcut context, and that menu's Close closes its
    /// current tab: `editor`'s, when the context holds `editor` too.
    fn close_route(editor: &Gd<CodeEdit>) -> Option<(Gd<Viewport>, Gd<Shortcut>)> {
        let viewport = editor.get_viewport()?;
        let focus = viewport.gui_get_focus_owner()?;
        if !editor.is_visible_in_tree() {
            return None;
        }
        // The very Shortcut object on 4.6+, a name match on 4.5.
        let wanted = EditorInterface::singleton()
            .get_editor_settings()
            .and_then(|mut settings| {
                godot_calls::get_shortcut_quiet(&mut settings, SHORTCUT_CLOSE_FILE)
            });
        let menus = viewport
            .find_children_ex("*")
            .type_("MenuButton")
            .recursive(true)
            .owned(false)
            .done();
        let shortcut = menus
            .iter_shared()
            .filter_map(|node| node.try_cast::<MenuButton>().ok())
            .filter(|menu| {
                menu.get_viewport().as_ref() == Some(&viewport)
                    && menu.is_visible_in_tree()
                    && !menu.is_disabled()
                    && menu
                        .get_shortcut_context()
                        .is_some_and(|ctx| holds(&ctx, editor) && holds(&ctx, &focus))
            })
            .find_map(|menu| {
                let popup = menu.get_popup()?;
                (0..popup.get_item_count())
                    .filter(|&i| !popup.is_item_disabled(i))
                    .filter_map(|i| popup.get_item_shortcut(i))
                    .find(|shortcut| match &wanted {
                        Some(wanted) => shortcut == wanted,
                        None => {
                            CLOSE_SHORTCUT_NAMES.contains(&shortcut.get_name().to_string().as_str())
                        }
                    })
            })?;
        Some((viewport, shortcut))
    }

    fn holds<T: Inherits<Node>>(ctx: &Gd<Node>, node: &Gd<T>) -> bool {
        ctx.instance_id() == node.instance_id() || ctx.is_ancestor_of(node)
    }

    /// Pushes the File menu's Close for `editor`'s tab at the end of the
    /// frame: Close frees `editor` at once, and the push runs this plugin's
    /// own `_input`, so neither may happen inside the key that asked for it.
    fn close_editor_tab(editor: &Gd<CodeEdit>) -> Result<(), HostError> {
        let Some((mut viewport, shortcut)) = close_route(editor) else {
            log::debug!("file::close refused: no File menu closes this editor");
            return Err(HostError::NotClosed);
        };
        // Shortcut::matches_event takes an InputEventShortcut carrying the
        // same object, whatever keys the user bound.
        let mut event = InputEventShortcut::new_gd();
        event.set_shortcut(&shortcut);
        viewport.call_deferred("push_input", &[event.to_variant(), false.to_variant()]);
        Ok(())
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
        let file = FileAccess::open(&GString::from(path), ModeFlags::READ).ok_or_else(|| {
            HostError::CantOpenFile {
                path: CompactString::from(path),
                detail: None,
            }
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
            attached_script(self.0)
                .ok()
                .map(|s| s.get_path().to_string())
        }

        fn is_modified(&self) -> bool {
            self.0.get_version() != self.0.get_saved_version()
        }

        fn tag_saved_version(&mut self) {
            self.0.tag_saved_version();
        }

        fn set_text(&mut self, text: &str) {
            self.0.set_text(&GString::from(text));
        }

        fn close_tab(&mut self) -> Result<(), HostError> {
            close_editor_tab(self.0)
        }

        fn can_close(&self) -> Result<(), HostError> {
            close_route(self.0).map(|_| ()).ok_or(HostError::NotClosed)
        }

        fn notify_name_changed(&self) {
            emit_name_changed();
        }

        fn save_script(&mut self, explicit_path: Option<&str>) -> Result<String, HostError> {
            let mut script =
                attached_script(self.0).inspect_err(|e| log::debug!("file::save refused: {e}"))?;

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
                std::fs::read_to_string(path).map_err(|e| HostError::CantOpenFile {
                    path: CompactString::from(path),
                    detail: Some(CompactString::from(e.to_string())),
                })
            }
        }

        fn update_script_source(&mut self, text: &str) {
            if let Ok(mut script) = attached_script(self.0) {
                script.set_source_code(&GString::from(text));
            }
        }
    }
}

pub(crate) use godot_impl::GodotEditorHost;

#[cfg(test)]
pub(super) mod mock {
    use super::EditorHost;
    use crate::host::error::HostError;
    use compact_str::CompactString;
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

    /// Which CodeEdit the mock stands for.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub(in crate::host) enum MockAttached {
        /// The script editor's current tab, editing `script_path`.
        ScriptTab,
        /// Any other CodeEdit (Shader Editor, addon). `script_path` is then
        /// the script editor's current script, a different file.
        Other,
    }

    /// Test double for `EditorHost`. All fields are public for direct scenario setup.
    pub(in crate::host) struct MockEditorHost {
        pub text: String,
        pub script_path: Option<String>,
        pub attached: MockAttached,
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
        /// No File menu of Godot's would close this editor.
        pub close_refused: bool,
    }

    impl MockEditorHost {
        pub fn new(text: &str, script_path: Option<&str>) -> Self {
            Self {
                text: text.to_string(),
                script_path: script_path.map(|s| s.to_string()),
                attached: MockAttached::ScriptTab,
                buffer_state: MockBufferState::Unmodified,
                save_called: false,
                save_result: None,
                open_result: None,
                files: HashMap::new(),
                opened_path: None,
                script_source_updated: None,
                close_refused: false,
            }
        }

        /// A CodeEdit outside the script editor while `script_path` is the
        /// script editor's current script.
        pub fn other(text: &str, script_path: Option<&str>) -> Self {
            Self {
                attached: MockAttached::Other,
                ..Self::new(text, script_path)
            }
        }

        fn owns_script(&self) -> bool {
            self.attached == MockAttached::ScriptTab
        }
    }

    impl EditorHost for MockEditorHost {
        fn get_text(&self) -> String {
            self.text.clone()
        }

        fn current_script_path(&self) -> Option<String> {
            self.script_path.clone().filter(|_| self.owns_script())
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

        fn close_tab(&mut self) -> Result<(), HostError> {
            self.can_close()?;
            self.buffer_state = MockBufferState::Closed;
            Ok(())
        }

        fn can_close(&self) -> Result<(), HostError> {
            if self.close_refused {
                return Err(HostError::NotClosed);
            }
            Ok(())
        }

        fn notify_name_changed(&self) {
            // No-op: trait requires `&self` (matching Godot's signal API), so
            // we cannot track calls without interior mutability. The important
            // behaviors (save, close, tag_saved_version) are `&mut self` and
            // fully trackable.
        }

        fn save_script(&mut self, explicit_path: Option<&str>) -> Result<String, HostError> {
            if !self.owns_script() {
                return Err(HostError::NotCurrentScript);
            }
            if let Some(ref result) = self.save_result {
                let r = result.clone();
                if let Ok(saved_path) = &r {
                    // Mirror real behavior: only tag saved when saving to the
                    // script's own path (not when `:w other.gd`).
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
                None => match &self.script_path {
                    Some(sp) if !sp.is_empty() => sp.clone(),
                    _ => return Err(HostError::NoFileName),
                },
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
                std::fs::read_to_string(path).map_err(|e| HostError::CantOpenFile {
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
            if self.owns_script() {
                self.script_source_updated = Some(text.to_string());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::is_attached_owner;
    use godot::obj::InstanceId;

    #[test]
    fn attached_owner_only_when_ids_match() {
        let script_tab = InstanceId::from_i64(1);
        let shader = InstanceId::from_i64(2);
        assert!(is_attached_owner(Some(script_tab), script_tab));
        assert!(!is_attached_owner(Some(script_tab), shader));
        assert!(!is_attached_owner(None, script_tab));
    }
}
