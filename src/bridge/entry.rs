use crate::bridge::global_input::global_input::GlobalInputHandler;
use crate::bridge::vim_adapter::mapping::{MappingPanel, MappingState};
use crate::bridge::vim_wrapper::VimController;

use crate::bridge::godot::names::{callbacks, control, script_editor, timer, viewport};
use godot::classes::editor_plugin::DockSlot;
use godot::classes::object::ConnectFlags;
use godot::classes::{
    CodeEdit, Control, EditorInterface, EditorPlugin, IEditorPlugin, InputEvent, InputEventKey,
    Node, ProjectSettings, RichTextLabel, Timer,
};
use godot::prelude::*;
use std::sync::atomic::{AtomicBool, Ordering};

/// Global flag to prevent duplicate plugin initialization during hot-reload
static PLUGIN_INITIALIZED: AtomicBool = AtomicBool::new(false);

#[derive(GodotClass)]
#[class(tool, base=EditorPlugin)]
pub struct GodotVimPlugin {
    base: Base<EditorPlugin>,
    pub(crate) vim_controller: Option<Gd<VimController>>,
    pub(crate) mapping_panel: Option<Gd<MappingPanel>>,
    /// State for global key mapping sequences (e.g., <Space>f)
    pub(crate) global_mapping_state: MappingState,
    /// Timer for global mapping timeout
    pub(crate) global_mapping_timer: Option<Gd<Timer>>,
}

#[godot_api]
impl IEditorPlugin for GodotVimPlugin {
    fn init(base: Base<EditorPlugin>) -> Self {
        crate::bridge::safety::install_panic_hook();
        Self {
            base,
            vim_controller: None,
            mapping_panel: None,
            global_mapping_state: MappingState::new(),
            global_mapping_timer: None,
        }
    }

    fn enter_tree(&mut self) {
        // Guard against double initialization (can happen during hot-reload)
        // Use atomic flag to prevent race conditions between multiple plugin instances
        if PLUGIN_INITIALIZED.swap(true, Ordering::SeqCst) {
            log::warn!("Another plugin instance already initialized, skipping");
            return;
        }

        if self.vim_controller.is_some() {
            log::warn!("enter_tree called but controller already exists, skipping");
            PLUGIN_INITIALIZED.store(false, Ordering::SeqCst);
            return;
        }

        // Register and sync settings immediately to prevent race conditions
        crate::bridge::settings::register_settings();
        crate::bridge::settings::sync_all_settings();

        // Initialize logging
        crate::logging::init_logging();
        let log_level = crate::bridge::settings::VimSettings::log_level();
        crate::logging::set_level(log_level);

        if !self.is_plugin_enabled() {
            log::info!("Plugin disabled in settings, skipping initialization");
            PLUGIN_INITIALIZED.store(false, Ordering::SeqCst);
            return;
        }

        log::info!("GodotVim {} initialized", env!("CARGO_PKG_VERSION"));

        let controller = Gd::<VimController>::from_init_fn(VimController::init);
        self.base_mut()
            .add_child(&controller.clone().upcast::<Node>());
        self.vim_controller = Some(controller);

        // ─────────────────────────────────────────────────────────────────────
        // Add Mapping Panel Dock (only if mapping is enabled)
        // ─────────────────────────────────────────────────────────────────────
        let mapping_enabled = crate::bridge::settings::VimSettings::mapping_enabled();
        if mapping_enabled {
            let mut panel = MappingPanel::new_alloc();
            panel.set_name("GodotVim Mappings");
            self.base_mut()
                .add_control_to_dock(DockSlot::RIGHT_UL, &panel.clone().upcast::<Control>());
            self.mapping_panel = Some(panel);
        }

        // Connect to ScriptEditor's script_changed signal
        let interface = EditorInterface::singleton();
        let Some(mut script_editor) = interface.get_script_editor() else {
            log::warn!("ScriptEditor unavailable, GodotVim plugin disabled");
            return;
        };

        let callable = self.base().callable(callbacks::ON_SCRIPT_CHANGED);
        if !script_editor.is_connected(script_editor::signals::EDITOR_SCRIPT_CHANGED, &callable) {
            script_editor.connect(script_editor::signals::EDITOR_SCRIPT_CHANGED, &callable);
        }

        // Connect to Viewport's focus change signal (Event-Driven Architecture)
        if let Some(mut vp) = interface.get_base_control().and_then(|c| c.get_viewport()) {
            let callable = self.base().callable(callbacks::ON_FOCUS_CHANGED);
            if !vp.is_connected(viewport::signals::GUI_FOCUS_CHANGED, &callable) {
                // Deferred connection prevents re-entrant borrow panics when focus changes
                // during input processing. The generated .connect() does not accept flags,
                // so the dynamic .call() form is used here.
                vp.call(
                    "connect",
                    &[
                        viewport::signals::GUI_FOCUS_CHANGED.to_variant(),
                        callable.to_variant(),
                        ConnectFlags::DEFERRED.ord().to_variant(),
                    ],
                );
            }
        }

        // Trigger initial attach check deferred (in case plugin loaded after editor focus)
        self.base_mut()
            .call_deferred(callbacks::ON_SCRIPT_CHANGED, &[Variant::nil()]);

        // Enable input processing for global interception; disable polling (event-driven).
        self.base_mut().set_process_input(true);
        self.base_mut().set_process(false);

        // Create global mapping timeout timer
        let mut gtimer = Timer::new_alloc();
        gtimer.set_one_shot(true);
        let callable = self.base().callable(callbacks::ON_GLOBAL_MAPPING_TIMEOUT);
        gtimer.connect(timer::signals::TIMEOUT, &callable);
        self.base_mut().add_child(&gtimer);
        self.global_mapping_timer = Some(gtimer);
    }

    fn exit_tree(&mut self) {
        log::info!("Plugin disabled");

        // Clear the global initialization flag
        PLUGIN_INITIALIZED.store(false, Ordering::SeqCst);

        let interface = EditorInterface::singleton();
        if let Some(mut se) = interface.get_script_editor() {
            let callable = self.base().callable(callbacks::ON_SCRIPT_CHANGED);
            if se.is_connected(script_editor::signals::EDITOR_SCRIPT_CHANGED, &callable) {
                se.disconnect(script_editor::signals::EDITOR_SCRIPT_CHANGED, &callable);
            }
        }

        if let Some(mut vp) = interface.get_base_control().and_then(|c| c.get_viewport()) {
            let callable = self.base().callable(callbacks::ON_FOCUS_CHANGED);
            if vp.is_connected(viewport::signals::GUI_FOCUS_CHANGED, &callable) {
                vp.disconnect(viewport::signals::GUI_FOCUS_CHANGED, &callable);
            }
        }

        if let Some(panel) = self.mapping_panel.take() {
            self.base_mut()
                .remove_control_from_docks(&panel.upcast::<Control>());
        }

        if let Some(mut controller) = self.vim_controller.take() {
            controller.bind_mut().detach();
            controller.queue_free();
        }
    }

    fn process(&mut self, _delta: f64) {}

    /// Global input interceptor for Dock Navigation and Global Mappings.
    ///
    /// Delegates to [`GlobalInputHandler`] (see `input/global_input.rs`).
    fn input(&mut self, event: Gd<InputEvent>) {
        crate::bridge::safety::guard(
            || {
                self.handle_global_input(event.clone());
            },
            (),
        );
    }
}

#[godot_api]
impl GodotVimPlugin {
    #[func]
    fn on_script_changed(&mut self, _script: Variant) {
        crate::bridge::safety::guard(
            || {
                // Find the active CodeEdit first
                let code_edit = Self::find_active_code_edit();

                // Attach deferred
                if let Some(code_edit) = code_edit {
                    self.base_mut()
                        .call_deferred(callbacks::PERFORM_ATTACH, &[code_edit.to_variant()]);
                } else {
                    // No active CodeEdit; check for the Documentation panel (EditorHelp).
                    if let Some(help) = Self::find_active_editor_help() {
                        // Focus the RichTextLabel so j/k scrolling is immediately active.
                        // Deferred to avoid re-entrant borrow via on_focus_changed.
                        if let Some(label) = crate::bridge::navigation::dock::focus::find_child_recursive_type_control::<RichTextLabel>(&help.upcast()) {
                         let mut label = label;
                         label.call_deferred(control::methods::GRAB_FOCUS, &[]);
                    }
                    }
                }
            },
            (),
        );
    }

    /// Global mapping timeout callback - resets pending key sequence.
    #[func]
    fn on_global_mapping_timeout(&mut self) {
        crate::bridge::safety::guard(
            || {
                self.global_mapping_state.reset();
            },
            (),
        );
    }

    fn find_active_editor_help() -> Option<Gd<Control>> {
        let interface = EditorInterface::singleton();
        let script_editor = interface.get_script_editor()?;
        let current_editor = script_editor.get_current_editor()?;

        if current_editor.is_class("EditorHelp") {
            return Some(current_editor.upcast());
        }
        None
    }

    /// Deferred attachment logic.
    /// Runs on idle frame to avoid signal re-entrancy between focus_changed and script_changed.
    #[func]
    fn perform_attach(&mut self, node: Variant) {
        let Ok(node) = node.try_to::<Gd<Control>>() else {
            // Object was freed before attachment completed (common when closing tabs).
            return;
        };

        if !node.is_instance_valid() {
            return;
        }

        // It IS a CodeEdit (or subclass)
        if let Ok(code_edit) = node.clone().try_cast::<CodeEdit>() {
            let current_id = code_edit.instance_id();
            if let Some(controller) = &mut self.vim_controller {
                if controller.bind().is_attached_to_editor(current_id) {
                    return;
                }
                // Deferred execution provides a fresh call stack, so binding is safe.
                controller.bind_mut().attach(code_edit);
            }
        }
    }

    /// Handles focus changes globally in the editor.
    #[func]
    fn on_focus_changed(&mut self, focused_node: Gd<Control>) {
        // Deferred to avoid re-entrant borrow during attachment.
        // Resolve CodeEdit from the focused control tree so non-script editors
        // (e.g. shader editor hosts) use the same attach path.
        if let Some(code_edit) = Self::find_code_edit_from_control(&focused_node) {
            self.base_mut()
                .call_deferred(callbacks::PERFORM_ATTACH, &[code_edit.to_variant()]);
        }

        if focused_node.is_class("LineEdit") {
            let mut line_edit = focused_node.clone().cast::<godot::classes::LineEdit>();
            let callable = self.base().callable(callbacks::ON_DOCK_SEARCH_INPUT);

            if !line_edit.is_connected(control::signals::GUI_INPUT, &callable) {
                line_edit.connect(control::signals::GUI_INPUT, &callable);
            }
        }

        // Observe Tree/ItemList/RichTextLabel for dock navigation.
        // Deferred to avoid lock collision with on_cursor_visual_update during focus changes.
        if focused_node.is_class("Tree")
            || focused_node.is_class("ItemList")
            || focused_node.is_class("RichTextLabel")
        {
            if let Some(controller) = &mut self.vim_controller {
                let mut ctrl = controller.clone();
                ctrl.call_deferred(
                    callbacks::OBSERVE_DOCK_CONTROL,
                    &[focused_node.to_variant()],
                );
            }
        }
    }

    /// Handles input on dock search bars (ESC/Enter returns focus to nav control).
    #[func]
    fn on_dock_search_input(&mut self, event: Gd<InputEvent>) {
        let Some(key_event) = event.try_cast::<InputEventKey>().ok() else {
            return;
        };

        if !key_event.is_pressed() {
            return;
        }

        let key = key_event.get_keycode();
        if matches!(
            key,
            godot::global::Key::ESCAPE | godot::global::Key::ENTER | godot::global::Key::KP_ENTER
        ) {
            let interface = EditorInterface::singleton();
            if let Some(base) = interface.get_base_control() {
                if let Some(viewport) = base.get_viewport() {
                    if let Some(focus_owner) = viewport.gui_get_focus_owner() {
                        // Deferred focus shift to avoid re-entrant borrow via gui_focus_changed.
                        if let Some(nav_control) =
                            crate::bridge::navigation::dock::focus::find_sibling_nav_control(
                                &focus_owner,
                            )
                        {
                            nav_control
                                .clone()
                                .upcast::<Node>()
                                .call_deferred(control::methods::GRAB_FOCUS, &[]);
                        }
                    }
                }
            }
        }
    }

    fn find_active_code_edit() -> Option<Gd<CodeEdit>> {
        // Primary source: currently focused control subtree.
        if let Some(code_edit) = Self::find_focused_code_edit() {
            return Some(code_edit);
        }

        // Fallback: current ScriptEditor tab (legacy path).
        let interface = EditorInterface::singleton();
        let script_editor = interface.get_script_editor()?;
        let current_editor = script_editor.get_current_editor()?;

        // Traverse children of the current script editor base to find CodeEdit
        Self::find_code_edit_recursive(&current_editor.upcast())
    }

    fn find_focused_code_edit() -> Option<Gd<CodeEdit>> {
        let interface = EditorInterface::singleton();
        let focused = interface
            .get_base_control()
            .and_then(|c| c.get_viewport())
            .and_then(|vp| vp.gui_get_focus_owner())?;
        Self::find_code_edit_from_control(&focused)
    }

    fn find_code_edit_from_control(control: &Gd<Control>) -> Option<Gd<CodeEdit>> {
        if let Ok(code_edit) = control.clone().try_cast::<CodeEdit>() {
            return Some(code_edit);
        }

        Self::find_code_edit_recursive(&control.clone().upcast::<Node>())
    }

    fn find_code_edit_recursive(node: &Gd<Node>) -> Option<Gd<CodeEdit>> {
        if let Ok(code_edit) = node.clone().try_cast::<CodeEdit>() {
            return Some(code_edit);
        }

        for child in node.get_children().iter_shared() {
            if let Some(found) = Self::find_code_edit_recursive(&child) {
                return Some(found);
            }
        }
        None
    }
    fn is_plugin_enabled(&self) -> bool {
        let settings = ProjectSettings::singleton();
        let setting_path = "editor_plugins/enabled";

        if !settings.has_setting(setting_path) {
            return false;
        }

        let enabled_plugins = settings.get_setting(setting_path).to::<PackedStringArray>();
        let plugin_path = "res://addons/godot_vim/plugin.cfg";

        for p in enabled_plugins.as_slice() {
            if *p == plugin_path.into() {
                return true;
            }
        }

        false
    }
}
