//! Top-level `EditorPlugin` that manages controller lifecycle and input routing.
//!
//! [`GodotVimPlugin`] is the single Godot-visible class. It handles editor
//! attachment/detachment, signal wiring, settings synchronization, and
//! dispatches keystrokes to the [`crate::controller::VimController`].

mod attach;
mod discovery;
mod floating;
mod input;
mod lifecycle;
mod signals;

use godot::classes::{
    CodeEdit, Control, EditorInterface, EditorPlugin, IEditorPlugin, InputEvent, Timer,
};
use godot::prelude::*;

use crate::controller::VimController;
use crate::safety::{install_panic_hook, panic_guard};

use floating::TrackedWindow;

const SIG_SETTINGS_CHANGED: &str = "settings_changed";
const SIG_EDITOR_SCRIPT_CHANGED: &str = "editor_script_changed";
const SIG_GUI_FOCUS_CHANGED: &str = "gui_focus_changed";
const SIG_TIMEOUT: &str = "timeout";
const SIG_CONFIG_SAVED: &str = "config_saved";
const SIG_WINDOW_VISIBILITY_CHANGED: &str = "window_visibility_changed";
const SIG_CHILD_ENTERED_TREE: &str = "child_entered_tree";

#[derive(GodotClass)]
#[class(tool, base=EditorPlugin)]
pub struct GodotVimPlugin {
    base: Base<EditorPlugin>,
    /// `None` between `exit_tree` and the next `enter_tree` (or before first init).
    controller: Option<VimController>,
    /// The CodeEdit that Vim input is currently routed to.
    attached_editor: Option<Gd<CodeEdit>>,
    /// Persists across detach/reattach to skip redundant focus events.
    /// Godot InstanceIds are globally unique and never recycled, so this
    /// is safe against ABA problems.
    last_editor_id: Option<InstanceId>,
    ui: crate::ui::UiCoordinator,
    /// Fires after `timeoutlen` ms to resolve partially-matched key mappings.
    mapping_timer: Option<Gd<Timer>>,
    settings: Option<crate::settings::SettingsSnapshot>,
    /// Lazily created on first `:mappings` invocation.
    mapping_dialog: Option<Gd<crate::ui::mapping_dialog::MappingDialog>>,
    /// Counter (not bool) because fast typing queues multiple deferred
    /// `caret_changed` callbacks per frame. Each suppressed callback
    /// decrements by 1.
    pending_caret_suppressions: u32,
    tracked_windows: Vec<TrackedWindow>,
}

#[godot_api]
impl IEditorPlugin for GodotVimPlugin {
    fn init(base: Base<EditorPlugin>) -> Self {
        install_panic_hook();
        Self {
            base,
            controller: None,
            attached_editor: None,
            last_editor_id: None,
            ui: crate::ui::UiCoordinator::new(),
            mapping_timer: None,
            settings: None,
            mapping_dialog: None,
            pending_caret_suppressions: 0,
            tracked_windows: Vec::new(),
        }
    }

    fn input(&mut self, event: Gd<InputEvent>) {
        panic_guard("input", || self.handle_input_impl(event), ());
    }

    fn enter_tree(&mut self) {
        panic_guard(
            "enter_tree",
            || {
                // gdext auto-registers EditorPlugin classes, creating an extension
                // instance with no script attached. Only the addon instance (from
                // plugin.cfg) has the GDScript set — this is a Godot architectural
                // invariant: set_addon_plugin_enabled() calls set_script() BEFORE
                // add_child(), while add_extension_editor_plugin() never sets a script.
                // We only initialize the addon instance so the Project Settings
                // plugin checkbox works as the user expects.
                if self.base().get_script().is_none() {
                    return;
                }

                if self.controller.is_some() {
                    return;
                }

                self.controller = Some(VimController::new());
                self.init_settings();
                self.init_mapping_timer();
                self.base_mut().set_process_input(true);
                self.connect_editor_signals();
                self.init_floating_window_tracking();
                self.base_mut()
                    .call_deferred("on_script_changed", &[Variant::nil()]);

                log::info!("GodotVim initialized");
            },
            (),
        );
    }

    fn exit_tree(&mut self) {
        panic_guard(
            "exit_tree",
            || {
                if self.controller.is_none() {
                    return;
                }
                log::info!("GodotVim shutting down");
                self.teardown_floating_window_tracking();
                self.detach();
                self.disconnect_editor_signals();
                self.teardown_settings();
                self.teardown_mapping_timer();
                if let Some(mut dialog) = self.mapping_dialog.take() {
                    dialog.queue_free();
                }
                self.controller = None;
                self.settings = None;
                self.last_editor_id = None;
            },
            (),
        );
    }
}

// Signal handlers -- thin routing wrappers that delegate to impl methods.

#[godot_api]
impl GodotVimPlugin {
    #[func]
    fn on_script_changed(&mut self, _script: Variant) {
        if self.controller.is_none() { return; }
        panic_guard(
            "on_script_changed",
            || {
                if let Some(code_edit) = discovery::find_active_code_edit() {
                    self.base_mut()
                        .call_deferred("perform_attach", &[code_edit.to_variant()]);
                }
            },
            (),
        );
    }

    #[func]
    fn on_focus_changed(&mut self, focused_node: Gd<Control>) {
        if self.controller.is_none() { return; }
        panic_guard(
            "on_focus_changed",
            || {
                if let Some(code_edit) =
                    discovery::find_code_edit_from_control(&focused_node)
                {
                    self.base_mut()
                        .call_deferred("perform_attach", &[code_edit.to_variant()]);
                }
            },
            (),
        );
    }

    #[func]
    fn on_window_visibility_changed(&mut self, visible: bool) {
        panic_guard(
            "on_window_visibility_changed",
            || {
                log::trace!(
                    "on_window_visibility_changed: visible={} tracked_count={}",
                    visible,
                    self.tracked_windows.len()
                );
                if visible {
                    self.connect_floating_viewport();
                } else {
                    self.disconnect_floating_viewport();
                }
            },
            (),
        );
    }

    #[func]
    fn on_child_entered_tree(&mut self, node: Gd<Node>) {
        panic_guard(
            "on_child_entered_tree",
            || {
                if !floating::is_window_wrapper(&node) {
                    return;
                }

                {
                    let node_class = node.get_class().to_string();
                    let wrapper_id = node.instance_id();
                    log::debug!(
                        "on_child_entered_tree: detected WindowWrapper (class={}) id=#{}",
                        node_class, wrapper_id.to_i64()
                    );
                    if self.tracked_windows.iter().any(|tw| tw.wrapper_id == wrapper_id) {
                        log::debug!("on_child_entered_tree: already tracked #{}, skipping", wrapper_id.to_i64());
                        return;
                    }
                    let callable = self.base().callable("on_window_visibility_changed");
                    let mut n = node;
                    signals::connect_immediate(&mut n, SIG_WINDOW_VISIBILITY_CHANGED, &callable);
                    log::debug!("on_child_entered_tree: connected window_visibility_changed on #{}", wrapper_id.to_i64());
                    self.tracked_windows.push(TrackedWindow {
                        wrapper_id,
                        window_id: None,
                    });
                }
            },
            (),
        );
    }

    #[func]
    fn on_floating_window_focused(&mut self) {
        panic_guard(
            "on_floating_window_focused",
            || {
                log::trace!(
                    "on_floating_window_focused: checking {} tracked windows",
                    self.tracked_windows.len()
                );
                for tw in &self.tracked_windows {
                    let Some(window_id) = tw.window_id else {
                        log::trace!(
                            "on_floating_window_focused: wrapper #{} has no window_id, skipping",
                            tw.wrapper_id.to_i64()
                        );
                        continue;
                    };
                    let Ok(window_node) = Gd::<Node>::try_from_instance_id(window_id) else {
                        log::debug!(
                            "on_floating_window_focused: window #{} freed, skipping",
                            window_id.to_i64()
                        );
                        continue;
                    };
                    let Ok(viewport) = window_node.try_cast::<godot::classes::Viewport>() else {
                        log::warn!(
                            "on_floating_window_focused: window #{} not a Viewport",
                            window_id.to_i64()
                        );
                        continue;
                    };
                    if let Some(focus_owner) = viewport.gui_get_focus_owner() {
                        let focus_class = focus_owner.get_class().to_string();
                        log::trace!(
                            "on_floating_window_focused: window #{} focus_owner class={}",
                            window_id.to_i64(), focus_class
                        );
                        if let Some(code_edit) = crate::plugin::discovery::find_code_edit_from_control(&focus_owner) {
                            log::debug!(
                                "on_floating_window_focused: found CodeEdit #{} in floating window #{}, attaching",
                                code_edit.instance_id().to_i64(), window_id.to_i64()
                            );
                            self.base_mut()
                                .call_deferred("perform_attach", &[code_edit.to_variant()]);
                            return;
                        }
                    } else {
                        log::trace!(
                            "on_floating_window_focused: window #{} has no focus_owner",
                            window_id.to_i64()
                        );
                    }
                }
                log::trace!("on_floating_window_focused: no CodeEdit found in any floating window");
            },
            (),
        );
    }

    /// Deferred attach entry point. Called via `call_deferred` from
    /// signal handlers to avoid borrowing conflicts with `&mut self`.
    #[func]
    fn perform_attach(&mut self, node: Variant) {
        if self.controller.is_none() { return; }
        let ok = panic_guard(
            "perform_attach",
            || {
                let Ok(control) = node.try_to::<Gd<Control>>() else {
                    return true;
                };
                if !control.is_instance_valid() {
                    return true;
                }
                let Ok(code_edit) = control.try_cast::<CodeEdit>() else {
                    return true;
                };

                let current_id = code_edit.instance_id();
                if self.last_editor_id == Some(current_id) {
                    return true;
                }

                self.attach(code_edit);
                true
            },
            false,
        );
        if !ok {
            self.recover_controller_from_panic();
        }
    }

    #[func]
    #[allow(clippy::needless_pass_by_value)] // gdext requires Gd<T> by value
    fn handle_gui_input(&mut self, event: Gd<InputEvent>) {
        let ok = panic_guard("handle_gui_input", || { self.handle_gui_input_impl(event); true }, false);
        if !ok {
            self.recover_controller_from_panic();
        }
    }

    #[func]
    fn on_mapping_timeout(&mut self) {
        let ok = panic_guard("on_mapping_timeout", || { self.on_mapping_timeout_impl(); true }, false);
        if !ok {
            self.recover_controller_from_panic();
        }
    }

    #[func]
    fn on_caret_changed(&mut self) {
        let ok = panic_guard("on_caret_changed", || { self.on_caret_changed_impl(); true }, false);
        if !ok {
            self.recover_controller_from_panic();
        }
    }

    /// Catches external text changes (Find-and-Replace, plugins, auto-format)
    /// that bypass the Vim keystroke pipeline's inline cache invalidation.
    #[func]
    fn on_text_changed(&mut self) {
        panic_guard(
            "on_text_changed",
            || {
                if let Some(controller) = &mut self.controller {
                    controller.invalidate_text_cache();
                }
            },
            (),
        );
    }

    #[func]
    fn on_scrollbar_changed(&mut self, _value: f64) {
        panic_guard("on_scrollbar_changed", || self.update_cursor_if_attached(), ());
    }

    #[func]
    fn on_editor_draw(&mut self) {
        panic_guard("on_editor_draw", || self.update_cursor_if_attached(), ());
    }

    #[func]
    fn on_config_saved(&mut self) {
        if self.controller.is_none() { return; }
        let ok = panic_guard("on_config_saved", || { self.source_config_from_disk("on_config_saved"); true }, false);
        if !ok {
            self.recover_controller_from_panic();
        }
    }

    /// Fires for ALL EditorSettings changes (not just ours), so we
    /// unconditionally re-read the full snapshot. The reader falls back
    /// to defaults for missing or wrong-type values.
    #[func]
    fn on_settings_changed(&mut self) {
        if self.controller.is_none() { return; }
        let ok = panic_guard(
            "on_settings_changed",
            || {
                let Some(editor_settings) =
                    EditorInterface::singleton().get_editor_settings()
                else {
                    return true;
                };

                let snapshot = crate::settings::reader::read_all(&editor_settings);
                log::debug!("settings_changed: log_level={:?}", snapshot.log_level);
                crate::logging::set_level(snapshot.log_level);

                if let Some(controller) = &mut self.controller {
                    controller.apply_settings(&snapshot);
                }

                let mode = self
                    .controller
                    .as_ref()
                    .map_or(vim_core::primitives::Mode::Normal, |c| c.mode());
                if let Some(mut editor) = self.attached_editor.clone() {
                    if editor.is_instance_valid() {
                        self.ui.apply_settings(&snapshot, mode, &mut editor);
                    }
                }

                self.settings = Some(snapshot);

                true
            },
            false,
        );
        if !ok {
            self.recover_controller_from_panic();
        }
    }
}

impl GodotVimPlugin {
    /// Execute a pending UI action that requires plugin-level access (scene tree,
    /// settings snapshot) which the controller cannot reach directly.
    pub(super) fn handle_pending_ui_action(
        &mut self,
        action: crate::controller::PendingUiAction,
    ) {
        use crate::controller::PendingUiAction;
        match action {
            PendingUiAction::OpenMappingDialog => {
                let resolved = self.resolve_config_path();

                if self.mapping_dialog.is_none() {
                    let mut dialog =
                        crate::ui::mapping_dialog::MappingDialog::new_alloc();
                    let callable = self.base().callable("on_config_saved");
                    signals::connect_immediate(&mut dialog, SIG_CONFIG_SAVED, &callable);
                    self.base_mut()
                        .add_child(&dialog.clone().upcast::<Node>());
                    self.mapping_dialog = Some(dialog);
                }

                if let Some(mut dialog) = self.mapping_dialog.clone() {
                    dialog.bind_mut().open_with_config(&resolved.path);
                    log::debug!("pending_ui_action: opened MappingDialog");
                }
            }
            PendingUiAction::SourceConfigFile => {
                if !self.source_config_from_disk("pending_ui_action") {
                    let path = self.resolve_config_path().path;
                    log::warn!(
                        "pending_ui_action: SourceConfigFile — file not found at '{path}'",
                    );
                }
            }
        }
    }

    /// Post-panic recovery: reset controller to clean Normal-mode state, clear
    /// the text cache, drain orphaned undo groups, refresh the UI to reflect
    /// the recovered state. Called from every `panic_guard` callsite that
    /// mutates the controller.
    ///
    /// The recovery body is itself wrapped in `panic_guard` for defense-in-depth.
    /// If recovery panics (double-panic), Tier 1 (engine reset) has already
    /// completed inside `recover_from_panic`, so the engine is in a known-good
    /// state. Godot state may be slightly messy but no UB occurs.
    fn recover_controller_from_panic(&mut self) {
        panic_guard(
            "recover_controller_from_panic",
            || {
                if let (Some(controller), Some(editor)) =
                    (&mut self.controller, &mut self.attached_editor)
                {
                    if editor.is_instance_valid() {
                        let mut editor = editor.clone();
                        controller.recover_from_panic(&mut editor);

                        // Invalidate thread-local caches that may hold stale
                        // pre-panic data (shaped glyphs, auto-brace pairs).
                        crate::ui::cursor_shape::invalidate_shaped_cache();
                        crate::bridge::port_impl::invalidate_brace_pair_cache();

                        // Refresh UI so the user sees Normal mode + error message
                        // immediately, not stale pre-panic state.
                        let editor_id = editor.instance_id();
                        let snap = controller.ui_snapshot(editor_id);
                        self.ui.update(&snap, &mut editor);
                    }
                }
            },
            (),
        );
        // Always reset — trivially infallible (u32 assignment), must happen
        // regardless of whether recovery itself panicked.
        self.pending_caret_suppressions = 0;
    }

    fn update_cursor_if_attached(&mut self) {
        let Some(editor) = &self.attached_editor else { return; };
        if !editor.is_instance_valid() { return; }
        self.ui.update_cursor_position(editor);
        // Recompute inccommand pixel rects from stored logical positions.
        // Scroll and resize change the viewport, making cached pixel
        // coordinates from `get_rect_at_line_column` stale.
        self.ui.recompute_inccommand_rects(editor);
    }

    fn resolve_config_path(&self) -> crate::config::path::ResolvedConfig {
        let override_path = self
            .settings
            .as_ref()
            .map_or("", |s| s.config_file_path.as_str());
        crate::config::path::resolve(override_path)
    }

    /// Load config from disk, apply the project-vimrc security policy, and
    /// reload into the engine. Returns `true` if the file existed (regardless
    /// of whether the policy allowed sourcing it).
    fn source_config_from_disk(&mut self, caller: &str) -> bool {
        let resolved = self.resolve_config_path();
        let Some(text) = crate::config::writer::read_file(&resolved.path) else {
            return false;
        };
        let project_vimrc = self
            .settings
            .as_ref()
            .map_or(crate::settings::ProjectVimrc::Sandbox, |s| s.project_vimrc);
        let text = crate::config::sandbox::apply_vimrc_policy(&text, resolved.is_project_level, project_vimrc);
        if let Some(text) = text {
            if let Some(controller) = &mut self.controller {
                controller.reload_config(&text);
                log::info!("{caller}: sourced config from '{}'", resolved.path);
            }
        }
        true
    }
}
