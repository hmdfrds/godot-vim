//! Core Node that manages controller lifecycle and input routing.
//!
//! [`GodotVimCore`] is the Godot-visible Rust class owned by a GDScript
//! `EditorPlugin`. It handles editor attachment/detachment, signal wiring,
//! settings synchronization, and dispatches keystrokes to the
//! [`crate::controller::VimController`].
//!
//! The split between a GDScript `EditorPlugin` and this Rust `Node` works
//! around godotengine/godot#86035, a bug where GDScript cannot extend a
//! GDExtension `EditorPlugin` subclass. By using `base=Node` here, Rust is
//! not auto-registered as an `EditorPlugin`, and the GDScript layer can use
//! plain `extends EditorPlugin` instead.

mod attach;
mod discovery;
mod floating;
mod input;
mod lifecycle;
mod processing_guard;
mod signals;

use godot::classes::{
    CodeEdit, Control, DisplayServer, EditorInterface, INode, Input, InputEvent, InputEventKey,
    Time, Timer,
};
use godot::global::Key;
use godot::prelude::*;

use crate::controller::VimController;
use crate::safety::{install_panic_hook, panic_guard};

use floating::{disconnect_viewport_signals, TrackedWindow};
use signals::{SIG_CONFIG_SAVED, SIG_TREE_EXITED, SIG_WINDOW_VISIBILITY_CHANGED};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TooltipPhase {
    WaitingForRelease,
    WarpedPendingEmit,
}

struct PendingTooltip {
    symbol: String,
    line: i32,
    col: i32,
    warp_pos: Option<Vector2i>,
    editor_id: InstanceId,
    created_at_usec: u64,
    phase: TooltipPhase,
}

#[derive(GodotClass)]
#[class(tool, base=Node)]
pub struct GodotVimCore {
    base: Base<Node>,
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
    pending_tooltip: Option<PendingTooltip>,
    tracked_windows: Vec<TrackedWindow>,
    /// True while the engine is actively processing a keystroke.
    /// Used by [`ProcessingKeyGuard`] for RAII-based keystroke processing tracking.
    processing_key: bool,
    fs_explorer: crate::navigation::FileSystemExplorer,
}

#[godot_api]
impl INode for GodotVimCore {
    fn init(base: Base<Node>) -> Self {
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
            pending_tooltip: None,
            tracked_windows: Vec::new(),
            processing_key: false,
            fs_explorer: crate::navigation::FileSystemExplorer::new(),
        }
    }

    fn input(&mut self, event: Gd<InputEvent>) {
        panic_guard("input", || self.handle_input_impl(event), ());
    }

    fn process(&mut self, _delta: f64) {
        panic_guard("process", || self.poll_pending_tooltip(), ());
    }

    fn enter_tree(&mut self) {
        self.base_mut().set_process(false);
        panic_guard(
            "enter_tree",
            || {
                if self.controller.is_some() {
                    return;
                }

                self.controller = Some(VimController::new());
                self.init_settings();
                self.init_mapping_timer();
                self.base_mut().set_process_input(true);
                self.connect_editor_signals();
                self.init_floating_window_tracking();
                self.init_fs_explorer_callables();
                self.base_mut()
                    .call_deferred("on_script_changed", &[Variant::nil()]);

                log::info!("GodotVim initialized");
            },
            (),
        );
    }

    fn exit_tree(&mut self) {
        if self.controller.is_none() {
            return;
        }
        self.cancel_pending_tooltip();
        log::info!("GodotVim shutting down");
        panic_guard(
            "exit_tree:floating",
            || self.teardown_floating_window_tracking(),
            (),
        );
        panic_guard("exit_tree:detach", || self.detach(), ());
        panic_guard("exit_tree:signals", || self.disconnect_editor_signals(), ());
        panic_guard("exit_tree:settings", || self.teardown_settings(), ());
        panic_guard("exit_tree:timer", || self.teardown_mapping_timer(), ());
        panic_guard(
            "exit_tree:dialog",
            || {
                if let Some(mut dialog) = self.mapping_dialog.take() {
                    dialog.queue_free();
                }
            },
            (),
        );
        panic_guard("exit_tree:fs_explorer", || self.fs_explorer.cleanup(), ());
        // Unconditional: even if a guard above caught a panic, null the
        // controller so enter_tree can reinitialize cleanly. Orphaned signals
        // from a panicking teardown step fire into handlers that check
        // self.controller.is_none() and return early.
        self.controller = None;
        self.settings = None;
        self.last_editor_id = None;
    }
}

// Signal handlers -- thin routing wrappers that delegate to impl methods.

#[godot_api]
impl GodotVimCore {
    #[func]
    fn on_script_changed(&mut self, _script: Variant) {
        if self.controller.is_none() {
            return;
        }
        panic_guard(
            "on_script_changed",
            || {
                if let Some(code_edit) = discovery::find_active_code_edit() {
                    self.base_mut()
                        .call_deferred("perform_attach", &[code_edit.to_variant()]);
                } else {
                    self.base_mut().call_deferred("perform_detach", &[]);
                }
            },
            (),
        );
    }

    #[func]
    fn on_focus_changed(&mut self, focused_node: Gd<Control>) {
        if self.controller.is_none() {
            return;
        }
        panic_guard(
            "on_focus_changed",
            || {
                if let Some(code_edit) = discovery::find_code_edit_from_control(&focused_node) {
                    self.base_mut()
                        .call_deferred("perform_attach", &[code_edit.to_variant()]);
                }
            },
            (),
        );
    }

    #[func]
    fn on_window_visibility_changed(&mut self, visible: bool) {
        if self.controller.is_none() {
            return;
        }
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
        if self.controller.is_none() {
            return;
        }
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
                        node_class,
                        wrapper_id.to_i64()
                    );
                    if self
                        .tracked_windows
                        .iter()
                        .any(|tw| tw.wrapper_id == wrapper_id)
                    {
                        log::debug!(
                            "on_child_entered_tree: already tracked #{}, skipping",
                            wrapper_id.to_i64()
                        );
                        return;
                    }
                    let callables = self.floating_callables();
                    let mut n = node;
                    signals::connect_immediate(
                        &mut n,
                        SIG_WINDOW_VISIBILITY_CHANGED,
                        &callables.visibility_changed,
                    );
                    signals::connect_immediate(&mut n, SIG_TREE_EXITED, &callables.tree_exited);
                    log::debug!("on_child_entered_tree: connected window_visibility_changed + tree_exited on #{}", wrapper_id.to_i64());
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
    fn on_wrapper_tree_exited(&mut self) {
        if self.controller.is_none() {
            return;
        }
        panic_guard(
            "on_wrapper_tree_exited",
            || {
                self.base_mut().call_deferred("evict_stale_wrappers", &[]);
            },
            (),
        );
    }

    #[func]
    fn evict_stale_wrappers(&mut self) {
        if self.controller.is_none() {
            return;
        }
        panic_guard(
            "evict_stale_wrappers",
            || {
                let callables = self.floating_callables();

                let before = self.tracked_windows.len();
                self.tracked_windows.retain(|tw| {
                    let Ok(wrapper) = Gd::<Node>::try_from_instance_id(tw.wrapper_id) else {
                        // Wrapper freed — disconnect viewport signals if any.
                        if let Some(window_id) = tw.window_id {
                            disconnect_viewport_signals(window_id, &callables);
                        }
                        log::debug!(
                            "evict_stale_wrappers: evicted freed wrapper #{}",
                            tw.wrapper_id.to_i64()
                        );
                        return false;
                    };

                    // Wrapper still exists but left the tree.
                    if !wrapper.is_inside_tree() {
                        // Disconnect wrapper-level signals so orphaned connections
                        // don't fire on a wrapper no longer in tracked_windows.
                        let mut w = wrapper;
                        signals::safe_disconnect(
                            &mut w,
                            SIG_WINDOW_VISIBILITY_CHANGED,
                            &callables.visibility_changed,
                        );
                        signals::safe_disconnect(&mut w, SIG_TREE_EXITED, &callables.tree_exited);
                        if let Some(window_id) = tw.window_id {
                            disconnect_viewport_signals(window_id, &callables);
                        }
                        log::debug!(
                            "evict_stale_wrappers: evicted out-of-tree wrapper #{}",
                            tw.wrapper_id.to_i64()
                        );
                        return false;
                    }

                    true
                });

                let evicted = before - self.tracked_windows.len();
                if evicted > 0 {
                    log::debug!(
                        "evict_stale_wrappers: evicted {} entries, {} remaining",
                        evicted,
                        self.tracked_windows.len()
                    );
                }
            },
            (),
        );
    }

    #[func]
    fn on_floating_window_focused(&mut self) {
        if self.controller.is_none() {
            return;
        }
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
                    let Ok(window) = window_node.try_cast::<godot::classes::Window>() else {
                        log::warn!(
                            "on_floating_window_focused: window #{} not a Window",
                            window_id.to_i64()
                        );
                        continue;
                    };
                    // Only consider the window that actually has OS-level focus.
                    // Each Viewport maintains independent gui_focus_owner state,
                    // so checking all windows would match stale focus owners.
                    if !window.has_focus() {
                        log::trace!(
                            "on_floating_window_focused: window #{} does not have OS focus, skipping",
                            window_id.to_i64()
                        );
                        continue;
                    }
                    let viewport = window.clone().upcast::<godot::classes::Viewport>();
                    if let Some(focus_owner) = viewport.gui_get_focus_owner() {
                        let focus_class = focus_owner.get_class().to_string();
                        log::trace!(
                            "on_floating_window_focused: window #{} focus_owner class={}",
                            window_id.to_i64(),
                            focus_class
                        );
                        if let Some(code_edit) =
                            crate::plugin::discovery::find_code_edit_from_control(&focus_owner)
                        {
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
        if self.controller.is_none() {
            return;
        }
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
            // Prevent the dedup guard from blocking re-attachment after
            // a panic. Without this, the next focus event for the same
            // editor would be skipped.
            self.last_editor_id = None;
            // Disconnect any orphaned signal connections from a partial
            // attach. Since attached_editor is now stored before signal
            // connections, detach() has the editor reference and can
            // disconnect via safe_disconnect (no-op for signals that
            // were never connected).
            panic_guard("perform_attach:cleanup_detach", || self.detach(), ());
        }
    }

    /// Deferred detach entry point. Called via `call_deferred` from
    /// `on_script_changed` when no active CodeEdit exists (last tab closed
    /// or switched to a non-CodeEdit editor view such as the 2D/3D viewport).
    #[func]
    fn perform_detach(&mut self) {
        if self.controller.is_none() {
            return;
        }
        let ok = panic_guard(
            "perform_detach",
            || {
                // Re-discovery guard: between the deferred call being queued
                // and now, a competing `perform_attach` may have already run,
                // or the ScriptEditor may have recovered a CodeEdit. If so,
                // skip the detach — there is a valid editor to stay attached to.
                if discovery::find_active_code_edit().is_some() {
                    return true;
                }

                self.detach();
                // Standalone detach (not a precondition to attach) — clear
                // the dedup guard so re-attachment works when a CodeEdit
                // reappears. Must be inside the guard so it only runs when
                // the detach actually executes, not when the re-discovery
                // guard skips it.
                self.last_editor_id = None;

                // Sweep stale buffer entries now that we've detached.
                // Without this, closing all tabs leaves stale BufferState
                // (including UndoTree with text snapshots) in the HashMap
                // until the next attach() call.
                if let Some(controller) = &mut self.controller {
                    controller.sweep_stale_buffers();
                }

                true
            },
            false,
        );
        if !ok {
            self.recover_controller_from_panic();
            // Clear the dedup guard so re-attachment works when a CodeEdit
            // reappears. Without this, a panic during detach would leave
            // last_editor_id set, and the next perform_attach for the same
            // editor would skip.
            self.last_editor_id = None;
        }
    }

    #[func]
    #[allow(clippy::needless_pass_by_value)] // gdext requires Gd<T> by value
    fn handle_gui_input(&mut self, event: Gd<InputEvent>) {
        if self.controller.is_none() {
            return;
        }
        let ok = panic_guard(
            "handle_gui_input",
            || {
                self.handle_gui_input_impl(event);
                true
            },
            false,
        );
        if !ok {
            self.recover_controller_from_panic();
        }
    }

    #[func]
    fn on_mapping_timeout(&mut self) {
        if self.controller.is_none() {
            return;
        }
        let ok = panic_guard(
            "on_mapping_timeout",
            || {
                self.on_mapping_timeout_impl();
                true
            },
            false,
        );
        if !ok {
            self.recover_controller_from_panic();
        }
    }

    #[func]
    fn on_caret_changed(&mut self) {
        if self.controller.is_none() {
            return;
        }
        let ok = panic_guard(
            "on_caret_changed",
            || {
                self.on_caret_changed_impl();
                true
            },
            false,
        );
        if !ok {
            self.recover_controller_from_panic();
        }
    }

    /// Signal handler for `text_changed`. Detects external text changes
    /// (Find-and-Replace, refactoring, external formatters) and reconciles
    /// them with the engine for undo/dot-repeat tracking.
    #[func]
    fn on_text_changed(&mut self) {
        // Text changes caused by Vim's own effects are already tracked.
        if self.processing_key {
            return;
        }
        if self.controller.is_none() {
            return;
        }
        let ok = panic_guard(
            "on_text_changed",
            || {
                let Some(editor) = &self.attached_editor else {
                    return true;
                };
                if !editor.is_instance_valid() {
                    return true;
                }
                let controller = self.controller.as_mut().unwrap();
                if controller.reconcile_external_edit(editor) {
                    log::debug!("on_text_changed: reconciled external text change");
                }
                true
            },
            false,
        );
        if !ok {
            self.recover_controller_from_panic();
        }
    }

    #[func]
    fn on_scrollbar_changed(&mut self, _value: f64) {
        if self.controller.is_none() {
            return;
        }
        panic_guard(
            "on_scrollbar_changed",
            || {
                self.update_cursor_if_attached();
            },
            (),
        );
    }

    #[func]
    fn on_editor_draw(&mut self) {
        if self.controller.is_none() {
            return;
        }
        panic_guard("on_editor_draw", || self.update_cursor_if_attached(), ());
    }

    #[func]
    fn on_fs_prompt_submitted(&mut self, text: GString) {
        panic_guard(
            "on_fs_prompt_submitted",
            || self.fs_explorer.on_prompt_submitted(text.to_string()),
            (),
        );
    }

    #[func]
    fn on_fs_prompt_gui_input(&mut self, event: Gd<InputEvent>) {
        panic_guard(
            "on_fs_prompt_gui_input",
            || {
                let Ok(key_event) = event.try_cast::<InputEventKey>() else {
                    return;
                };
                if !key_event.is_pressed() {
                    return;
                }
                if key_event.get_keycode() == Key::ESCAPE {
                    self.fs_explorer.dismiss_prompt();
                }
            },
            (),
        );
    }

    #[func]
    fn on_config_saved(&mut self) {
        if self.controller.is_none() {
            return;
        }
        let ok = panic_guard(
            "on_config_saved",
            || {
                self.source_config_from_disk("on_config_saved");
                true
            },
            false,
        );
        if !ok {
            self.recover_controller_from_panic();
        }
    }

    /// Fires for ALL EditorSettings changes (not just ours), so we
    /// unconditionally re-read the full snapshot. The reader falls back
    /// to defaults for missing or wrong-type values.
    #[func]
    fn on_settings_changed(&mut self) {
        if self.controller.is_none() {
            return;
        }
        let ok = panic_guard(
            "on_settings_changed",
            || {
                let Some(editor_settings) = EditorInterface::singleton().get_editor_settings()
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

impl GodotVimCore {
    fn init_fs_explorer_callables(&mut self) {
        let base = self.base().clone();
        self.fs_explorer.set_callables(
            base.callable("on_fs_prompt_submitted"),
            base.callable("on_fs_prompt_gui_input"),
        );
    }

    /// Execute a pending UI action that requires plugin-level access (scene tree,
    /// settings snapshot) which the controller cannot reach directly.
    ///
    /// Only `OpenMappingDialog` and `SourceConfigFile` reach the plugin layer;
    /// the controller handles all other variants inline before storing. The
    /// catch-all arm is defense-in-depth.
    pub(super) fn handle_pending_ui_action(
        &mut self,
        action: crate::bridge::godot_host::PendingUiAction,
    ) {
        use crate::bridge::godot_host::PendingUiAction;
        match action {
            PendingUiAction::OpenMappingDialog => {
                let resolved = self.resolve_config_path();

                if self.mapping_dialog.is_none() {
                    let mut dialog = crate::ui::mapping_dialog::MappingDialog::new_alloc();
                    let callable = self.base().callable("on_config_saved");
                    signals::connect_immediate(&mut dialog, SIG_CONFIG_SAVED, &callable);
                    self.base_mut().add_child(&dialog.clone().upcast::<Node>());
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
                    log::warn!("pending_ui_action: SourceConfigFile — file not found at '{path}'",);
                }
            }
            other => {
                log::warn!(
                    "handle_pending_ui_action: unexpected variant {:?} reached plugin layer",
                    other,
                );
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
                let has_valid_editor = self
                    .attached_editor
                    .as_ref()
                    .is_some_and(|e| e.is_instance_valid());

                if let Some(controller) = &mut self.controller {
                    if has_valid_editor {
                        let mut editor = self.attached_editor.as_ref().unwrap().clone();
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
                    } else {
                        // No editor, or editor exists but is invalid (freed
                        // during the panic). Canonical Tier 1 cleanup only.
                        log::warn!(
                            "recover_controller_from_panic: no valid editor, Tier 1 cleanup only"
                        );
                        controller.force_cleanup_without_editor();
                    }
                }
            },
            (),
        );
        // Stop the mapping timer — emergency_reset cleared all pending mapping
        // state, so a stale timeout firing would be a wasted no-op.
        if let Some(timer) = self.mapping_timer.as_mut() {
            timer.stop();
        }
        // Always reset — trivially infallible (u32 assignment), must happen
        // regardless of whether recovery itself panicked.
        self.pending_caret_suppressions = 0;
        self.processing_key = false;
        // Clear pending tooltip directly rather than via cancel_pending_tooltip()
        // because set_process(false) is safe here (poll_pending_tooltip won't
        // run again until re-enabled) and direct field clear is simpler in a
        // panic recovery context.
        self.pending_tooltip = None;
        self.base_mut().set_process(false);
    }

    fn cancel_pending_tooltip(&mut self) {
        if self.pending_tooltip.is_some() {
            self.pending_tooltip = None;
            self.base_mut().set_process(false);
        }
    }

    fn poll_pending_tooltip(&mut self) {
        let Some(pending) = &self.pending_tooltip else {
            self.base_mut().set_process(false);
            return;
        };

        // Stale editor check
        let editor_valid = self
            .attached_editor
            .as_ref()
            .is_some_and(|e| e.is_instance_valid() && e.instance_id() == pending.editor_id);
        if !editor_valid {
            log::debug!("poll_pending_tooltip: editor changed, cancelling");
            self.pending_tooltip = None;
            self.base_mut().set_process(false);
            return;
        }

        // Timeout check (500ms)
        let now = Time::singleton().get_ticks_usec();
        if now.saturating_sub(pending.created_at_usec) > 500_000 {
            log::debug!("poll_pending_tooltip: timeout, cancelling");
            self.pending_tooltip = None;
            self.base_mut().set_process(false);
            return;
        }

        match pending.phase {
            TooltipPhase::WaitingForRelease => {
                if Input::singleton().is_anything_pressed() {
                    return; // Keep polling
                }
                // All keys released — warp mouse
                if let Some(pos) = pending.warp_pos {
                    DisplayServer::singleton().warp_mouse(pos);
                }
                // MUST use mutable reference to transition phase
                self.pending_tooltip.as_mut().unwrap().phase = TooltipPhase::WarpedPendingEmit;
            }
            TooltipPhase::WarpedPendingEmit => {
                // One frame after warp — emit signal
                let pending = self.pending_tooltip.take().unwrap();
                self.base_mut().set_process(false);

                let Some(editor) = &self.attached_editor else {
                    return;
                };
                if !editor.is_instance_valid() {
                    return;
                }
                let mut ed = editor.clone();
                ed.emit_signal(
                    "symbol_hovered",
                    &[
                        pending.symbol.to_variant(),
                        pending.line.to_variant(),
                        pending.col.to_variant(),
                    ],
                );
                log::debug!(
                    "poll_pending_tooltip: emitted symbol_hovered for '{}' at {}:{}",
                    pending.symbol,
                    pending.line,
                    pending.col
                );
            }
        }
    }

    fn update_cursor_if_attached(&mut self) {
        let Some(editor) = &self.attached_editor else {
            return;
        };
        if !editor.is_instance_valid() {
            return;
        }
        self.ui.update_cursor_position(editor);
        // Recompute inccommand pixel rects from stored logical positions.
        // Scroll and resize change the viewport, making cached pixel
        // coordinates from `get_rect_at_line_column` stale.
        self.ui.recompute_inccommand_rects(editor);
        self.ui.recompute_block_visual_rects(editor);
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
        let text = crate::config::sandbox::apply_vimrc_policy(
            &text,
            resolved.is_project_level,
            project_vimrc,
        );
        if let Some(text) = text {
            if let Some(controller) = &mut self.controller {
                controller.reload_config(&text);
                log::info!("{caller}: sourced config from '{}'", resolved.path);
            }
        }
        true
    }
}
