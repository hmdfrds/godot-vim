//! Vim-like file operations on Godot's FileSystem dock.
//!
//! Adds nvim-tree-style keybindings (`a` create, `d` delete, `r` rename,
//! `y` yank path, `R` refresh) when focus is on the FileSystem dock's Tree
//! or ItemList. Routes through `GodotVimCore::handle_input_impl` before
//! the generic dock navigation in `dock.rs`.

use godot::classes::{
    Control, DirAccess, DisplayServer, EditorInterface, FileAccess, HBoxContainer, Input,
    InputEventKey, ItemList, Label, LineEdit, Node, Tree, VBoxContainer,
};
use godot::global::Key;
use godot::prelude::*;

use crate::bridge::godot_calls;

use crate::scene_tree::find_child_of_type;

use super::dock::DockInputResult;
use super::focus::DockKind;

/// Tracks what the shared LineEdit prompt is doing.
enum PromptMode {
    Inactive,
    Create { target_dir: String },
}

pub(crate) struct FileSystemExplorer {
    prompt: Option<Gd<LineEdit>>,
    prompt_label: Option<Gd<Label>>,
    prompt_container: Option<Gd<Node>>,
    prompt_mode: PromptMode,
    active_control: Option<Gd<Control>>,
    callable_submitted: Option<Callable>,
    callable_gui_input: Option<Callable>,
}

impl FileSystemExplorer {
    pub(crate) fn new() -> Self {
        Self {
            prompt: None,
            prompt_label: None,
            prompt_container: None,
            prompt_mode: PromptMode::Inactive,
            active_control: None,
            callable_submitted: None,
            callable_gui_input: None,
        }
    }

    pub(crate) fn set_callables(&mut self, submitted: Callable, gui_input: Callable) {
        self.callable_submitted = Some(submitted);
        self.callable_gui_input = Some(gui_input);
    }

    pub(crate) fn cleanup(&mut self) {
        if let Some(mut container) = self.prompt_container.take() {
            if container.is_instance_valid() {
                container.queue_free();
            }
        }
        self.prompt.take();
        self.prompt_label.take();
        self.prompt_mode = PromptMode::Inactive;
        self.active_control = None;
    }

    pub(crate) fn handle_key(
        &mut self,
        key_event: &Gd<InputEventKey>,
        control: &Gd<Control>,
        kind: DockKind,
    ) -> DockInputResult {
        self.validate_cache();

        // If the prompt is visible but the Tree/ItemList has focus (not our
        // LineEdit), the user clicked away mid-prompt. Auto-dismiss.
        if !matches!(self.prompt_mode, PromptMode::Inactive) {
            self.dismiss_prompt();
        }

        if key_event.is_ctrl_pressed() || key_event.is_alt_pressed() || key_event.is_meta_pressed()
        {
            return DockInputResult::Ignored;
        }

        let shift = key_event.is_shift_pressed();
        let key = resolve_key(key_event);

        match (key, shift) {
            (Some(Key::A), false) => self.begin_create(control, kind),
            (Some(Key::D), false) => self.begin_delete(control, kind),
            (Some(Key::R), false) => self.begin_rename(control, kind),
            (Some(Key::Y), false) => self.yank_path(control, kind),
            (Some(Key::R), true) => self.refresh(),
            _ => DockInputResult::Ignored,
        }
    }

    pub(crate) fn is_prompt_active(&self, line_edit: &Gd<LineEdit>) -> bool {
        match &self.prompt {
            Some(prompt) if prompt.is_instance_valid() => {
                prompt.instance_id() == line_edit.instance_id()
            }
            _ => false,
        }
    }

    fn yank_path(&self, control: &Gd<Control>, kind: DockKind) -> DockInputResult {
        if let Some(path) = get_selected_path(control, kind) {
            DisplayServer::singleton().clipboard_set(&GString::from(&path));
            log::info!("filesystem_explorer: yanked path '{}'", path);
        }
        DockInputResult::Handled
    }

    fn refresh(&self) -> DockInputResult {
        if let Some(mut fs) = EditorInterface::singleton().get_resource_filesystem() {
            fs.scan();
            log::info!("filesystem_explorer: triggered filesystem scan");
        }
        DockInputResult::Handled
    }

    fn begin_create(&mut self, control: &Gd<Control>, kind: DockKind) -> DockInputResult {
        let target_dir = match get_selected_path(control, kind) {
            Some(path) if path.ends_with('/') => path,
            Some(path) => parent_dir(&path),
            None => "res://".to_string(),
        };
        self.active_control = Some(control.clone());
        self.show_prompt("New: ", None, PromptMode::Create { target_dir });
        DockInputResult::Handled
    }

    fn begin_delete(&mut self, _control: &Gd<Control>, _kind: DockKind) -> DockInputResult {
        trigger_dock_shortcut(godot_calls::SHORTCUT_FS_DELETE);
        DockInputResult::Handled
    }

    fn begin_rename(&mut self, _control: &Gd<Control>, _kind: DockKind) -> DockInputResult {
        trigger_dock_shortcut(godot_calls::SHORTCUT_FS_RENAME);
        DockInputResult::Handled
    }

    // ── Prompt lifecycle ──

    fn ensure_prompt(&mut self) {
        if let Some(ref p) = self.prompt {
            if p.is_instance_valid() {
                return;
            }
        }

        let (Some(callable_submitted), Some(callable_gui)) =
            (&self.callable_submitted, &self.callable_gui_input)
        else {
            log::warn!("filesystem_explorer: callables not set, cannot create prompt");
            return;
        };

        let Some(fs_dock) = EditorInterface::singleton().get_file_system_dock() else {
            return;
        };
        let dock_node: Gd<Node> = fs_dock.upcast();
        let Some(main_vb) = find_child_of_type::<VBoxContainer>(&dock_node, 3) else {
            log::warn!("filesystem_explorer: could not find main VBoxContainer in FileSystem dock");
            return;
        };

        let mut hbox = HBoxContainer::new_alloc();
        let label = Label::new_alloc();
        let mut line_edit = LineEdit::new_alloc();

        line_edit.set_h_size_flags(godot::classes::control::SizeFlags::EXPAND_FILL);
        line_edit.set_clear_button_enabled(true);

        hbox.add_child(&label);
        hbox.add_child(&line_edit);
        hbox.set_visible(false);

        let mut line_edit_obj = line_edit.clone().upcast::<Object>();
        if line_edit_obj.connect("text_submitted", callable_submitted) != godot::global::Error::OK {
            log::warn!("filesystem_explorer: failed to connect text_submitted");
        }
        if line_edit_obj.connect("gui_input", callable_gui) != godot::global::Error::OK {
            log::warn!("filesystem_explorer: failed to connect gui_input");
        }

        let mut main_vb_node: Gd<Node> = main_vb.upcast();
        main_vb_node.add_child(&hbox);

        self.prompt_container = Some(hbox.clone().upcast());
        self.prompt_label = Some(label);
        self.prompt = Some(line_edit);
    }

    fn show_prompt(&mut self, label_text: &str, prefill: Option<&str>, mode: PromptMode) {
        self.ensure_prompt();
        self.prompt_mode = mode;

        let Some(ref mut label) = self.prompt_label else {
            return;
        };
        if !label.is_instance_valid() {
            return;
        }
        label.set_text(label_text);

        let Some(ref mut line_edit) = self.prompt else {
            return;
        };
        if !line_edit.is_instance_valid() {
            return;
        }
        if let Some(text) = prefill {
            line_edit.set_text(text);
            let dot_pos = text.rfind('.').unwrap_or(text.len());
            line_edit.select_ex().from(0).to(dot_pos as i32).done();
        } else {
            line_edit.set_text("");
        }

        if let Some(ref mut container) = self.prompt_container {
            if container.is_instance_valid() {
                if let Ok(mut ctrl) = container.clone().try_cast::<Control>() {
                    ctrl.set_visible(true);
                }
            }
        }

        line_edit.grab_focus();
    }

    fn set_label(&mut self, text: &str) {
        if let Some(ref mut label) = self.prompt_label {
            if label.is_instance_valid() {
                label.set_text(text);
            }
        }
    }

    fn show_prompt_error(&mut self, msg: &str) {
        self.set_label(&format!("Error: {} ", msg));
    }

    pub(crate) fn dismiss_prompt(&mut self) {
        if let Some(ref mut container) = self.prompt_container {
            if container.is_instance_valid() {
                if let Ok(mut ctrl) = container.clone().try_cast::<Control>() {
                    ctrl.set_visible(false);
                }
            }
        }
        if let Some(ref mut line_edit) = self.prompt {
            if line_edit.is_instance_valid() {
                line_edit.set_text("");
            }
        }
        self.prompt_mode = PromptMode::Inactive;

        if let Some(ref control) = self.active_control {
            if control.is_instance_valid() {
                control
                    .clone()
                    .upcast::<Node>()
                    .call_deferred("grab_focus", &[]);
            }
        }
        self.active_control = None;
    }

    pub(crate) fn on_prompt_submitted(&mut self, text: String) {
        if text.is_empty() {
            self.dismiss_prompt();
            return;
        }

        self.set_label("New: ");

        let mode = std::mem::replace(&mut self.prompt_mode, PromptMode::Inactive);
        let success = match mode {
            PromptMode::Create { target_dir } => self.execute_create(&text, &target_dir),
            PromptMode::Inactive => true,
        };

        if success {
            self.dismiss_prompt();
        }
        // On failure, execute_create already called show_prompt_error
        // and restored prompt_mode, so the prompt stays open for retry.
    }

    fn execute_create(&mut self, name: &str, target_dir: &str) -> bool {
        if let Err(msg) = validate_path(name) {
            self.prompt_mode = PromptMode::Create {
                target_dir: target_dir.to_string(),
            };
            self.show_prompt_error(&msg);
            return false;
        }

        let full_path = format!("{}{}", target_dir, name);
        let is_dir = name.ends_with('/');

        if is_dir {
            if DirAccess::dir_exists_absolute(&full_path) {
                self.prompt_mode = PromptMode::Create {
                    target_dir: target_dir.to_string(),
                };
                self.show_prompt_error("Already exists");
                return false;
            }
            if DirAccess::make_dir_recursive_absolute(&full_path) != godot::global::Error::OK {
                self.prompt_mode = PromptMode::Create {
                    target_dir: target_dir.to_string(),
                };
                self.show_prompt_error("Failed to create directory");
                return false;
            }
        } else {
            if FileAccess::file_exists(&full_path) {
                self.prompt_mode = PromptMode::Create {
                    target_dir: target_dir.to_string(),
                };
                self.show_prompt_error("Already exists");
                return false;
            }
            let parent = parent_dir(&full_path);
            if !DirAccess::dir_exists_absolute(&parent)
                && DirAccess::make_dir_recursive_absolute(&parent) != godot::global::Error::OK
            {
                self.prompt_mode = PromptMode::Create {
                    target_dir: target_dir.to_string(),
                };
                self.show_prompt_error("Failed to create parent directories");
                return false;
            }
            let file = FileAccess::open(&full_path, godot::classes::file_access::ModeFlags::WRITE);
            if file.is_none() {
                self.prompt_mode = PromptMode::Create {
                    target_dir: target_dir.to_string(),
                };
                self.show_prompt_error("Failed to create file");
                return false;
            }
        }

        log::info!("filesystem_explorer: created '{}'", full_path);
        scan_and_navigate(&full_path);
        true
    }

    fn validate_cache(&mut self) {
        if let Some(ref ctrl) = self.active_control {
            if !ctrl.is_instance_valid() {
                self.active_control = None;
            }
        }
    }
}

/// Check logical keycode first, fall back to physical for non-Latin layouts.
fn resolve_key(key_event: &Gd<InputEventKey>) -> Option<Key> {
    let logical = key_event.get_keycode();
    let physical = key_event.get_physical_keycode();
    if is_fs_key(logical) {
        Some(logical)
    } else if is_fs_key(physical) {
        Some(physical)
    } else {
        None
    }
}

fn is_fs_key(key: Key) -> bool {
    matches!(key, Key::A | Key::D | Key::R | Key::Y)
}

pub(crate) fn is_in_filesystem_dock(control: &Gd<Control>) -> bool {
    let Some(fs_dock) = EditorInterface::singleton().get_file_system_dock() else {
        return false;
    };
    let dock_node: Gd<Node> = fs_dock.upcast();
    dock_node.is_ancestor_of(control)
}

fn validate_path(name: &str) -> Result<(), String> {
    if name.is_empty() {
        return Err("Empty name".to_string());
    }
    if name.starts_with('/') {
        return Err("Name must not start with /".to_string());
    }
    if name.contains("..") {
        return Err("Path traversal not allowed".to_string());
    }
    if name.contains('\\') {
        return Err("Backslashes not allowed".to_string());
    }
    if name.contains('\0') {
        return Err("Null bytes not allowed".to_string());
    }
    Ok(())
}

fn scan_and_navigate(path: &str) {
    if let Some(mut fs) = EditorInterface::singleton().get_resource_filesystem() {
        fs.scan();
    }
    if let Some(mut dock) = EditorInterface::singleton().get_file_system_dock() {
        // Deferred: scan() is async, so the tree hasn't rebuilt yet.
        // navigate_to_path expands collapsed ancestors via uncollapse_tree(),
        // but only works if the item exists in the tree — deferring gives the
        // scan at least one frame to process.
        let path_variant = Variant::from(GString::from(path));
        dock.call_deferred("navigate_to_path", &[path_variant]);
    }
}

fn parent_dir(path: &str) -> String {
    let trimmed = path.trim_end_matches('/');
    match trimmed.rfind('/') {
        Some(pos) => trimmed[..=pos].to_string(),
        None => "res://".to_string(),
    }
}

/// Trigger a FileSystem dock shortcut by its registered path.
///
/// Looks up the `Shortcut` from `EditorSettings`, extracts the first
/// `InputEventKey` from its events array, and injects it via
/// `Input::parse_input_event`. We send an `InputEventKey` (not
/// `InputEventShortcut`) because `FileSystemDock::_tree_gui_input`
/// casts the event to `InputEventKey` first — `InputEventShortcut`
/// would fail the cast and be silently ignored.
fn trigger_dock_shortcut(path: &str) {
    let editor_iface = EditorInterface::singleton();
    let Some(mut settings) = editor_iface.get_editor_settings() else {
        return;
    };
    let Some(shortcut) = godot_calls::get_shortcut(&mut settings, path) else {
        log::warn!("filesystem_explorer: shortcut '{}' not found", path);
        return;
    };

    let events = shortcut.get_events();
    for i in 0..events.len() {
        let Some(variant) = events.get(i) else {
            continue;
        };
        let Ok(source) = variant.try_to::<Gd<InputEventKey>>() else {
            continue;
        };

        let mut event = InputEventKey::new_gd();
        event.set_keycode(source.get_keycode());
        event.set_physical_keycode(source.get_physical_keycode());
        event.set_ctrl_pressed(source.is_ctrl_pressed());
        event.set_shift_pressed(source.is_shift_pressed());
        event.set_alt_pressed(source.is_alt_pressed());
        event.set_meta_pressed(source.is_meta_pressed());
        event.set_pressed(true);
        Input::singleton().parse_input_event(&event);
        return;
    }

    log::warn!(
        "filesystem_explorer: no InputEventKey in shortcut '{}'",
        path
    );
}

fn get_selected_path(control: &Gd<Control>, kind: DockKind) -> Option<String> {
    match kind {
        DockKind::Tree => {
            let tree = control.clone().try_cast::<Tree>().ok()?;
            let item = tree.get_selected()?;
            let metadata = item.get_metadata(0);
            let path = metadata.try_to::<GString>().ok()?;
            let path_str = path.to_string();
            if path_str == "Favorites" {
                return None;
            }
            Some(path_str)
        }
        DockKind::ItemList => {
            let mut list = control.clone().try_cast::<ItemList>().ok()?;
            let selected = list.get_selected_items();
            if selected.is_empty() {
                return None;
            }
            let idx = selected.get(0)?;
            let metadata = list.get_item_metadata(idx);
            let path = metadata.try_to::<GString>().ok()?;
            Some(path.to_string())
        }
        DockKind::RichTextLabel => None,
    }
}
