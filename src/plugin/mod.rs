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
mod caret_reconcile;
mod discovery;
mod floating;
mod input;
mod lifecycle;
mod outcome;
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

/// A `Tree`/`ItemList` whose incremental type-to-search is suppressed.
///
/// Godot's `Tree` type-searches on bare printable keys, so a reserved prefix
/// key would otherwise do both jobs at once: buffer in the shell plane and
/// start a type-search in the control. The previous value is carried rather
/// than assumed, because the user (or the editor) may have turned search off
/// already and restoring `true` would be a behaviour change we never made.
struct SearchSuppression {
    control: InstanceId,
    previous: bool,
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
    /// The shell plane's own timeout timer — see `init_panel_timer` for why
    /// this is not `mapping_timer`.
    panel_timer: Option<Gd<Timer>>,
    /// The shell plane's pending prefix buffer.
    ///
    /// Empty unless the user bound a multi-key `panelmap`: the shipped keyset
    /// reserves nothing, so this costs one `Option` compare per keystroke and
    /// nothing else. Cleared on execute, no-match, timeout, focus-owner
    /// change, plugin disable and config reload — deliberately **not** on echo.
    pending: crate::actions::sequence::Pending,
    /// The one control whose incremental type-to-search we turned off because
    /// its surface reserves a bare prefix key. See
    /// `input::GodotVimCore::sync_search_suppression`.
    search_suppression: Option<SearchSuppression>,
    settings: Option<crate::settings::SettingsSnapshot>,
    /// Lazily created on first `:mappings` invocation.
    mapping_dialog: Option<Gd<crate::ui::mapping_dialog::MappingDialog>>,
    /// Tracks whether a pending `caret_changed` signal was caused by the
    /// Vim engine (suppress) or an external source like a mouse click (process).
    caret_reconciler: caret_reconcile::CaretReconciler,
    pending_tooltip: Option<PendingTooltip>,
    tracked_windows: Vec<TrackedWindow>,
    /// True while the engine is actively processing a keystroke.
    /// Used by [`ProcessingKeyGuard`] for RAII-based keystroke processing tracking.
    processing_key: bool,
    fs_explorer: crate::navigation::FileSystemExplorer,
    /// Parsed `:set langmap`, rebuilt whenever config is sourced.
    ///
    /// `None` means the option is empty, which is the overwhelmingly common
    /// case — the probe pipeline then skips the remap entirely.
    langmap: Option<vim_core::keymap::LangmapTable>,
    /// Every shell-side verb the plugin knows, by id.
    ///
    /// Built once from `actions::specs::SHIPPED`. Later phases register
    /// provider tables into the same registry; nothing about the dispatcher
    /// changes when they do.
    actions: crate::actions::action::ActionRegistry,
    /// Per-line load errors from the user's vimrc, surfaced by `:panelmap`.
    binding_diagnostics: Vec<crate::actions::bind::PanelDiagnostic>,
    /// Which key, on which surface, means which verb.
    ///
    /// Rebuilt from the provider defaults (and, from the config phase, the
    /// resolved vimrc) whenever config is sourced; `generation` bumps with
    /// every rebuild, which is what invalidates the sampled focus chain.
    bindings: crate::actions::bind::BindingIndex,
    /// The focus chain, sampled once per distinct focus/mode/prompt state
    /// rather than once per keystroke. See `plugin::input::ChainCache`.
    chain_cache: Option<input::ChainCache>,
    /// Desired master-enable state (mirrors plugins/GodotVim/enabled).
    enabled: bool,
    /// Disabled->enabled EDGE detector (NOT a correctness gate): apply_enabled_state
    /// always reconciles toward `enabled`; `wired` only fires the one-shot
    /// re-discovery + config-source exactly once per enable transition.
    wired: bool,
}

#[godot_api]
impl INode for GodotVimCore {
    fn init(base: Base<Node>) -> Self {
        install_panic_hook();
        // Registry first, then the index over it: `builtin_index` rejects a
        // default naming an unregistered action, which is the load-time check
        // that keeps a typo from becoming a key that consumes and does
        // nothing.
        let actions = crate::actions::specs::registry();
        let bindings = crate::actions::bind::builtin_index(&actions);
        Self {
            base,
            binding_diagnostics: Vec::new(),
            controller: None,
            attached_editor: None,
            last_editor_id: None,
            ui: crate::ui::UiCoordinator::new(),
            mapping_timer: None,
            panel_timer: None,
            pending: crate::actions::sequence::Pending::default(),
            search_suppression: None,
            settings: None,
            mapping_dialog: None,
            caret_reconciler: caret_reconcile::CaretReconciler::new(),
            pending_tooltip: None,
            tracked_windows: Vec::new(),
            processing_key: false,
            fs_explorer: crate::navigation::FileSystemExplorer::new(),
            langmap: None,
            actions,
            bindings,
            chain_cache: None,
            enabled: true,
            wired: false,
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
                self.init_panel_timer();
                self.init_fs_explorer_callables();
                self.wired = false;
                self.apply_enabled_state();

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
        // Before the timers are freed and while the controls still exist:
        // `allow_search` lives on the focused `Tree`, which outlives the
        // plugin, so leaving it off would break type-to-search for the rest of
        // the editor session.
        panic_guard("exit_tree:sequence", || self.clear_pending_sequence(), ());
        panic_guard("exit_tree:timer", || self.teardown_mapping_timer(), ());
        panic_guard("exit_tree:panel_timer", || self.teardown_panel_timer(), ());
        panic_guard(
            "exit_tree:dialog",
            || {
                if let Some(mut dialog) = self.mapping_dialog.take() {
                    if dialog.is_instance_valid() {
                        dialog.queue_free();
                    }
                }
            },
            (),
        );
        panic_guard("exit_tree:fs_explorer", || self.fs_explorer.cleanup(), ());
        // Unconditional: even if a guard above caught a panic, null the
        // controller so enter_tree can reinitialize cleanly. Orphaned signals
        // from a panicking teardown step fire into handlers that check
        // self.controller.is_none() and return early.
        self.wired = false;
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
        if !self.enabled {
            return;
        }
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

    /// Called via `call_deferred` on the disabled→enabled edge to reattach to
    /// whatever CodeEdit is currently focused. Skipped when disabled or no
    /// controller is present (deferred call arrived after exit_tree).
    #[func]
    fn rediscover_and_attach(&mut self) {
        if !self.enabled || self.controller.is_none() {
            return;
        }
        panic_guard(
            "rediscover_and_attach",
            || {
                if let Some(code_edit) = discovery::find_active_code_edit() {
                    self.base_mut()
                        .call_deferred("perform_attach", &[code_edit.to_variant()]);
                } else if let Some(focus) = EditorInterface::singleton()
                    .get_base_control()
                    .and_then(|c| c.get_viewport())
                    .and_then(|v| v.gui_get_focus_owner())
                {
                    if let Some(code_edit) = discovery::find_code_edit_from_control(&focus) {
                        self.base_mut()
                            .call_deferred("perform_attach", &[code_edit.to_variant()]);
                    }
                }
            },
            (),
        );
    }

    /// Focus tracking for `gui_focus_changed`, on the main editor viewport
    /// (`lifecycle.rs`) and on every floating script window (`floating.rs`).
    ///
    /// `focused_node` is a `Variant`, not a `Gd<Control>`, and that is forced.
    /// Both connections are DEFERRED, so the call sits in Godot's message
    /// queue until end of frame and the Control that took focus can be freed
    /// in the gap: closing several script tabs at once moves focus onto a
    /// `CodeEdit` and frees it in the same frame. gdext converts varcall
    /// parameters *before* the handler body runs and refuses to build a
    /// `Gd<T>` from a freed object, so with a typed parameter this function
    /// could not defend itself at all -- the call died at the ABI boundary and
    /// the engine printed the failure. `Option<Gd<Control>>` fails identically,
    /// because gdext reserves `None` for NIL variants rather than dead objects.
    ///
    /// The aliveness check is not removed or weakened, it is relocated:
    /// `try_to` below runs the very same gdext check on the very same variant,
    /// one frame inside the boundary, where a dead object is an early return
    /// instead of an abandoned call. gdext documents `try_to::<Gd<...>>()` as
    /// the way to detect this case. `on_script_changed` and `perform_attach`
    /// take `Variant` for the same reason.
    #[func]
    fn on_focus_changed(&mut self, focused_node: Variant) {
        if !self.enabled {
            return;
        }
        // §5.10's focus-owner clearing site, and deliberately BEFORE the
        // controller guard below: a pending prefix and a suppressed
        // `allow_search` are shell-plane state, and must be released when the
        // user tabs away whether or not a script is open. The per-keystroke
        // check in `sync_search_suppression` cannot cover this on its own —
        // focus can move with no keystroke at all, and the shell timer would
        // then fire into a control the sequence was never typed at, leaving a
        // `Tree` unable to type-search until the next key.
        panic_guard(
            "on_focus_changed:sequence",
            || self.clear_pending_sequence(),
            (),
        );
        if self.controller.is_none() {
            return;
        }
        panic_guard(
            "on_focus_changed",
            || {
                // `Err` means the control that took focus did not survive to
                // delivery. Routine, not a fault: there is no subtree left to
                // search. Kept as a `match` rather than `let ... else` because
                // the `ConvertError` is the only place a dead object and a
                // genuinely wrong payload stay distinguishable, now that this
                // handler makes that call instead of the engine.
                let focused_node = match focused_node.try_to::<Gd<Control>>() {
                    Ok(control) => control,
                    Err(err) => {
                        log::trace!("on_focus_changed: no live Control to attach to: {err}");
                        return;
                    }
                };
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
        if !self.enabled {
            return;
        }
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
        if !self.enabled {
            return;
        }
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
        if !self.enabled {
            return;
        }
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

    /// Fires when the attached CodeEdit leaves the scene tree (tab closed,
    /// foreign editor freed). Performs an identity-guarded immediate detach so
    /// we don't hold a dangling handle until the next focus event.
    #[func]
    fn on_attached_editor_tree_exited(&mut self) {
        if !self.enabled {
            return;
        }
        if self.controller.is_none() {
            return;
        }
        panic_guard(
            "on_attached_editor_tree_exited",
            || {
                // tree_exited also fires on benign reparents; only act if this is
                // OUR attached editor leaving for good. detach() is self-completing
                // + safe (checks is_instance_valid internally).
                if self.attached_editor.is_some() {
                    self.detach();
                    self.last_editor_id = None;
                }
            },
            (),
        );
    }

    #[func]
    fn evict_stale_wrappers(&mut self) {
        if !self.enabled {
            return;
        }
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
        if !self.enabled {
            return;
        }
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
        if !self.enabled {
            return;
        }
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
        if !self.enabled {
            return;
        }
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
                // (including UndoStore with text snapshots) in the HashMap
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

    /// The shell plane's timeout. Deliberately **not** guarded on
    /// `self.controller.is_some()`: the headline capability of the sequence
    /// phase is that a dock prefix resolves with no script open, and a
    /// controller guard here would reintroduce exactly the dependency the
    /// separate timer exists to remove.
    #[func]
    fn on_panel_timeout(&mut self) {
        if !self.enabled {
            return;
        }
        let ok = panic_guard(
            "on_panel_timeout",
            || {
                self.on_panel_timeout_impl();
                true
            },
            false,
        );
        if !ok {
            // No engine was involved, so the controller needs no recovery —
            // but the shell plane's own state does, and `allow_search` in
            // particular lives on a control that outlives the plugin.
            self.clear_pending_sequence();
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

    /// Signal handler for `text_set`. Fired when `CodeEdit.set_text()` is
    /// called programmatically (file reload, VCS revert). Differs from
    /// `text_changed`: Godot destroys its undo stack, resets the caret to
    /// (0,0), and clears selections. We fence our undo tree, clear the
    /// UndoStore, remap marks through the diff, and restore the cursor.
    #[func]
    fn on_text_set(&mut self) {
        if self.processing_key {
            return;
        }
        if self.controller.is_none() {
            return;
        }
        let ok = panic_guard(
            "on_text_set",
            || {
                let Some(editor) = &self.attached_editor else {
                    return true;
                };
                if !editor.is_instance_valid() {
                    return true;
                }
                let controller = self.controller.as_mut().unwrap();
                if controller.reconcile_text_set(editor) {
                    log::info!(
                        "on_text_set: buffer replaced externally, undo cleared, marks remapped"
                    );
                }
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
                    self.caret_reconciler
                        .expect_vim_move(editor.get_caret_line(), editor.get_caret_column());
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
        if !self.enabled {
            return;
        }
        panic_guard(
            "on_fs_prompt_submitted",
            || self.fs_explorer.on_prompt_submitted(text.to_string()),
            (),
        );
    }

    #[func]
    fn on_fs_prompt_gui_input(&mut self, event: Gd<InputEvent>) {
        if !self.enabled {
            return;
        }
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
        if !self.enabled {
            return;
        }
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
    /// to defaults for missing or wrong-type values. The engine receives
    /// only the fields that differ from the last push, so a user's `:set`
    /// is not reverted by an unrelated change.
    #[func]
    fn on_settings_changed(&mut self) {
        // Intentionally ungated — must observe re-enable to become active again.
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

                self.enabled = snapshot.enabled;
                // CLONE: the trailing live-reload code still borrows `&snapshot`,
                // so storing by move would be a use-after-move.
                self.settings = Some(snapshot.clone());
                self.apply_enabled_state();

                if let Some(controller) = &mut self.controller {
                    controller.apply_settings(&snapshot);
                }

                // Re-sync indent settings from the attached CodeEdit.
                // EditorSettings changes can affect indent_size / tab_size,
                // so the engine must pick up the new values, but only when
                // the CodeEdit's values actually moved: this signal fires for
                // every EditorSettings write, and an unconditional sync
                // reverted a user's `:set ts` on any unrelated click.
                if let Some(ref editor) = self.attached_editor {
                    if editor.is_instance_valid() {
                        if let Some(controller) = &mut self.controller {
                            attach::resync_indent_from_editor(editor, controller);
                        }
                    }
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

    /// Reconcile the plugin toward the desired `self.enabled` state.
    ///
    /// Branch-on-desired: always runs toward the target state; no
    /// `enabled == wired` early-return. `wired` is purely an edge
    /// detector — it fires the one-shot re-discovery + config-source
    /// exactly once per disabled→enabled transition.
    fn apply_enabled_state(&mut self) {
        if self.enabled {
            let was_inert = !self.wired;
            panic_guard(
                "enable:input",
                || {
                    self.base_mut().set_process_input(true);
                },
                (),
            );
            panic_guard("enable:signals", || self.connect_editor_signals(), ());
            panic_guard(
                "enable:floating",
                || self.init_floating_window_tracking(),
                (),
            );
            self.wired = true;
            if was_inert {
                // disabled→enabled edge: single startup-equivalent config load + re-discovery.
                // Forget the last push first, so the engine gets every field
                // before the vimrc is sourced on top, exactly as at startup.
                if let Some(s) = self.settings.clone() {
                    if let Some(c) = &mut self.controller {
                        c.forget_applied_settings();
                        c.apply_settings(&s);
                    }
                }
                self.source_config_from_disk("enable");
                self.last_editor_id = None;
                self.base_mut().call_deferred("rediscover_and_attach", &[]);
                self.base_mut()
                    .call_deferred("on_floating_window_focused", &[]);
            }
        } else {
            panic_guard("disable:detach", || self.detach(), ());
            // §5.10 — `pending` clears on plugin disable, and the suppressed
            // `allow_search` comes back with it. A disabled plugin that left a
            // dock Tree unable to type-search would be an invisible,
            // unattributable regression.
            panic_guard("disable:sequence", || self.clear_pending_sequence(), ());
            panic_guard("disable:signals", || self.disconnect_editor_signals(), ());
            panic_guard(
                "disable:floating",
                || self.teardown_floating_window_tracking(),
                (),
            );
            panic_guard("disable:fs", || self.fs_explorer.cleanup(), ());
            panic_guard(
                "disable:dialog",
                || {
                    if let Some(mut d) = self.mapping_dialog.take() {
                        if d.is_instance_valid() {
                            d.queue_free();
                        }
                    }
                },
                (),
            );
            self.last_editor_id = None;
            panic_guard(
                "disable:input",
                || {
                    self.base_mut().set_process_input(false);
                },
                (),
            );
            self.wired = false; // unconditional: safe because there is no latch to trap on
        }
    }

    /// Execute a pending UI action that requires plugin-level access (scene tree,
    /// settings snapshot) which the controller cannot reach directly.
    ///
    /// `OpenMappingDialog`, `SourceConfigFile`, and `ShowTooltip` reach the
    /// plugin layer; the controller handles all other variants inline before
    /// storing. The catch-all arm is defense-in-depth.
    pub(super) fn handle_pending_ui_action(
        &mut self,
        action: crate::bridge::godot_host::PendingUiAction,
    ) {
        use crate::bridge::godot_host::PendingUiAction;
        match action {
            PendingUiAction::RunRegistryAction { name, count } => {
                self.run_registry_action(&name, count);
            }
            PendingUiAction::PanelCommand(args) => {
                self.panel_command(&args);
            }
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
            PendingUiAction::ShowTooltip {
                symbol,
                line,
                col,
                warp_pos,
            } => {
                if let Some(editor) = &self.attached_editor {
                    let editor_id = editor.instance_id();
                    let now = godot::classes::Time::singleton().get_ticks_usec();
                    self.pending_tooltip = Some(PendingTooltip {
                        symbol,
                        line,
                        col,
                        warp_pos,
                        editor_id,
                        created_at_usec: now,
                        phase: TooltipPhase::WaitingForRelease,
                    });
                    self.base_mut().set_process(true);
                    log::debug!(
                        "handle_pending_ui_action: queued deferred tooltip for '{}'",
                        self.pending_tooltip.as_ref().unwrap().symbol
                    );
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
                    // The engine is not trusted to still hold the last push:
                    // a panic inside attach_session or detach_session unwinds
                    // with the fresh placeholder engine in place. The next
                    // settings push then writes every field.
                    controller.forget_applied_settings();
                    if has_valid_editor {
                        let mut editor = self.attached_editor.as_ref().unwrap().clone();
                        controller.recover_from_panic(&mut editor);

                        // Refresh UI so the user sees Normal mode + error message
                        // immediately, not stale pre-panic state.
                        //
                        // Intentional exception to EngineOutcome: panic recovery
                        // is defense-in-depth with unconditional-reset semantics,
                        // not the conditional-update pattern that EngineOutcome
                        // enforces. Using ui.update() directly is correct here.
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
                        controller.detach_session();
                        self.attached_editor = None;
                        self.last_editor_id = None;
                        self.ui.reset_cached_state();
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
        // The shell plane's own state, on the same unconditional footing: a
        // pending prefix that survived a panic would resolve against an index
        // the recovery may have rebuilt, and a suppressed `allow_search` that
        // survived would leave a dock Tree unable to type-search with no
        // plugin state left to explain it. This is the `panic_guard` recovery
        // path the design names explicitly.
        panic_guard("recover:sequence", || self.clear_pending_sequence(), ());
        // Always reset — trivially infallible, must happen regardless of
        // whether recovery itself panicked.
        self.caret_reconciler.reset();
        self.processing_key = false;
        // Clear pending tooltip directly rather than via cancel_pending_tooltip()
        // because set_process(false) is safe here (poll_pending_tooltip won't
        // run again until re-enabled) and direct field clear is simpler in a
        // panic recovery context.
        self.pending_tooltip = None;
        self.base_mut().set_process(false);
    }

    /// Drop the shell plane's between-keystroke state, whole.
    ///
    /// One function for all four clearing sites (teardown, disable, panic
    /// recovery, timeout failure) because the three pieces must always move
    /// together: a buffer with no timer never resolves, and a timer with no
    /// buffer fires into nothing.
    pub(super) fn clear_pending_sequence(&mut self) {
        self.pending.clear();
        if let Some(timer) = self.panel_timer.as_mut() {
            timer.stop();
        }
        self.restore_search_suppression();
    }

    /// How long the shell timer waits, in ms.
    ///
    /// See [`crate::actions::sequence::timeoutlen_ms`] for the priority order.
    /// The `SettingsSnapshot` arm is not a nicety: with no controller reachable
    /// at all the engine cannot be asked, and that is precisely the state — no
    /// script open, docks the only thing on screen — in which a panel sequence
    /// is the only thing the user can do.
    fn shell_timeoutlen_ms(&self) -> i64 {
        crate::actions::sequence::timeoutlen_ms(
            self.controller.as_ref().map(VimController::timeoutlen),
            self.settings.as_ref().map(|s| s.timeoutlen),
        )
    }

    /// Arm the shell timer for one `timeoutlen`.
    pub(super) fn arm_panel_timer(&mut self) {
        let ms = self.shell_timeoutlen_ms();
        let Some(timer) = self.panel_timer.as_mut() else {
            // No timer means the buffer would never resolve. Better to drop it
            // than to leave keys silently eaten forever.
            log::warn!("sequence: no shell timer; dropping the pending prefix");
            self.pending.clear();
            return;
        };
        timer.set_wait_time(ms as f64 / 1000.0);
        timer.start();
        let buffered: String = self
            .pending
            .keys()
            .iter()
            .map(vim_core::keymap::KeyEvent::to_vim_notation)
            .collect();
        log::trace!(
            "sequence: shell timer armed ({ms}ms) on {} with '{buffered}'",
            self.pending.surface().unwrap_or("<none>")
        );
    }

    pub(super) fn stop_panel_timer(&mut self) {
        if let Some(timer) = self.panel_timer.as_mut() {
            timer.stop();
        }
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
            // No file: the engine keeps whatever options it has, so the cache
            // must be reconciled here too. Returning early without this leaves
            // a stale table after a vimrc is deleted.
            self.rebuild_langmap();
            self.rebuild_bindings();
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
        self.rebuild_langmap();
        self.rebuild_bindings();
        true
    }

    /// Rebuild the panel binding index, atomically.
    ///
    /// Everything decidable is in [`build_index`]; what is left here is the
    /// three effects that need `&mut self` — logging the diagnostics, swapping
    /// the index in under a fresh generation, and dropping the chain cache.
    ///
    /// The swap is what makes it atomic: a fresh index is built to completion
    /// and only then installed, so one broken line can never leave a half-built
    /// table live.
    fn rebuild_bindings(&mut self) {
        let generation = self.bindings.generation.wrapping_add(1);
        let text = self.read_sandboxed_config();
        let source = self.resolve_config_path().path;
        let (mut index, diagnostics) = build_index(&self.actions, text.as_deref(), &source);
        for d in &diagnostics {
            // Warn-and-skip per line: one bad binding must not cost the user
            // the rest of their config. The log is the *secondary* channel and
            // always has been — the default Log Level is Off, so a diagnostic
            // that went only here reached nobody. `:panelmap` with no argument
            // prints the same list, which is what makes the claim below true
            // rather than aspirational: a diagnostic nobody can read is the
            // same as a silent dead key.
            log::warn!("panelmap: {d}");
        }
        self.binding_diagnostics = diagnostics;

        index.generation = generation;
        self.bindings = index;
        // The chain cache is keyed on the generation, so this is belt and
        // braces — but a cache that outlives its index is the kind of bug
        // that only shows up after a hot reload.
        self.invalidate_focus_chain();
        log::debug!(
            "panelmap: rebuilt {} binding(s) at generation {generation}",
            self.bindings.len()
        );
    }

    /// The active vimrc's text, after the project-vimrc security policy.
    ///
    /// `None` when no config file exists — the zero-config case, where the
    /// shipped defaults are the whole keyset.
    fn read_sandboxed_config(&self) -> Option<String> {
        let resolved = self.resolve_config_path();
        let text = crate::config::writer::read_file(&resolved.path)?;
        let project_vimrc = self
            .settings
            .as_ref()
            .map_or(crate::settings::ProjectVimrc::Sandbox, |s| s.project_vimrc);
        crate::config::sandbox::apply_vimrc_policy(&text, resolved.is_project_level, project_vimrc)
    }

    /// `:panelmap` and `:panelmap <lhs>` — the introspector.
    ///
    /// Printed with `godot_print!` rather than `log::info!` and to the Output
    /// panel rather than the status bar, for two separate reasons: the
    /// default `log` level is Off, so a `log::` call reaches nobody by
    /// default; and a resolution trace is a dozen lines, which the one-line
    /// status bar cannot show.
    fn panel_command(&mut self, args: &str) {
        let args = args.trim();
        if args.is_empty() {
            godot_print!(
                "{}",
                crate::actions::introspect::list_report(
                    &self.bindings,
                    &self.actions,
                    &self.binding_diagnostics,
                )
            );
            return;
        }
        // Sampled against the CURRENT focus, which is the whole point: the
        // answer to "why is my key dead" depends on where the keystroke would
        // land, and by the time the user has typed `:panelmap d` the command
        // line owns focus. That is a real caveat and the report prints the
        // chain it used so the user can see it.
        let chain = EditorInterface::singleton()
            .get_base_control()
            .and_then(|c| c.get_viewport())
            .map(|vp| {
                crate::actions::surface::FocusChain::sample(
                    &vp,
                    self.attached_editor
                        .as_ref()
                        .filter(|e| e.is_instance_valid())
                        .map(Gd::instance_id),
                    self.controller.as_ref().map(VimController::mode),
                    self.fs_explorer.prompt_instance(),
                )
            })
            .unwrap_or_default();
        let Some(path) = self.bindings.forest().classify(&chain) else {
            godot_warn!("panelmap: no surface claimed the current focus");
            return;
        };
        // The live path can never be a completion path: the command line owns
        // focus while `:panelmap` is typed, so the mode is CommandLine, and
        // `handle_set_mode` (`effects/mode.rs`) has already cancelled the
        // popup. The report answers anyway, through the same `Forest::overlay`
        // and the same `resolve`, with one input flipped.
        let hypothetical = crate::actions::surface::OverlayFacts {
            at_attached_editor: true,
            mode: Some(vim_core::primitives::Mode::Insert),
            selected_index: 0,
            caret: (0, 0),
        };
        let overlay = self.bindings.forest().overlay(&hypothetical);
        let controller = self.controller.as_ref();
        let claims =
            |k: vim_core::keymap::KeyEvent| controller.is_some_and(|c| c.could_start_mapping(k));
        godot_print!(
            "{}",
            crate::actions::introspect::explain_report(
                args,
                &chain,
                &path,
                overlay.as_ref(),
                &self.bindings,
                &self.actions,
                &claims,
            )
        );
    }

    /// Run a shell-side action invoked by name rather than by keystroke.
    ///
    /// The `:action <id>` / `<Action>(<id>)` path. Two rules from the design
    /// govern it, and both are why it lives here rather than in the host layer:
    ///
    /// 1. **Capabilities are not consulted.** There is no keystroke, no
    ///    surface and no sampled widget to derive them from, so gating here
    ///    would decline everything invisibly. `Caps` gates bindings only.
    /// 2. **`host_invocable: false` fails loudly.** An action that needs a
    ///    focused panel — `godotvim.item.next` has nothing to move — reports
    ///    a real error rather than silently doing nothing.
    fn run_registry_action(&mut self, name: &str, count: u32) {
        let Some(id) = self.actions.id_of(name) else {
            log::warn!("action: unknown action '{name}'");
            return;
        };
        let Some(spec) = self.actions.get(id) else {
            return;
        };
        if !spec.host_invocable {
            log::warn!("action: '{name}' requires panel focus");
            return;
        }

        // The focus owner, when there is one. Actions needing a specific
        // widget re-assert it in their own body — the same predicate the
        // binding path uses, a no-op there and the real guard here.
        let target = EditorInterface::singleton()
            .get_base_control()
            .and_then(|c| c.get_viewport())
            .and_then(|vp| vp.gui_get_focus_owner())
            .map(|o| o.upcast::<godot::classes::Control>());

        let mut params = crate::actions::action::Params::new();
        params.set_int("count", i64::from(count));
        // The explorer is lent here too, so `host_invocable: true` on
        // `godotvim.fs.create` is honest: the prompt it opens is owned by
        // `self.fs_explorer`, and without the loan the action would decline
        // from the command line while working from a key.
        let mut cx =
            crate::actions::action::ActionCtx::new(target, params).with_fs(&mut self.fs_explorer);
        let outcome = self.actions.run(id, &mut cx);
        log::debug!("action: '{name}' -> {outcome}");
    }

    /// Re-read `:set langmap` into the cached table used by the shell-side
    /// probe pipeline (`crate::actions::keys`).
    ///
    /// Called on every config-source path — startup, `:source`, the mapping
    /// dialog's save, and the no-file case — so a deleted vimrc cannot leave
    /// a stale table behind.
    ///
    /// It is deliberately NOT hooked to interactive ex-commands: a `:set
    /// langmap=` typed at the command line updates the engine's table but not
    /// this cache, so the two planes disagree until the next source. Hooking
    /// every ex-command execution for one option is not obviously worth the
    /// coupling; the gap is recorded rather than papered over.
    ///
    /// A malformed table is dropped with a warning rather than failing the
    /// load — one bad option must not take the rest of the user's config
    /// with it.
    fn rebuild_langmap(&mut self) {
        let spec = self
            .controller
            .as_ref()
            .map(|c| c.engine().options().langmap().to_string())
            .unwrap_or_default();
        if spec.is_empty() {
            self.langmap = None;
            return;
        }
        match vim_core::keymap::LangmapTable::parse(&spec) {
            Ok(table) => {
                log::info!("langmap: loaded '{spec}'");
                self.langmap = Some(table);
            }
            Err(err) => {
                log::warn!("langmap: ignoring malformed '{spec}': {err:?}");
                self.langmap = None;
            }
        }
    }
}

/// Build a complete panel binding index from the builtins plus one vimrc.
///
/// The pure half of [`GodotVimCore::rebuild_bindings`], lifted out because
/// everything it decides is decidable without a `Gd<T>` — and everything the
/// caller keeps needs `&mut self`, so the two could not be tested together at
/// any price.
///
/// `text` is the vimrc **after** the project-vimrc security policy, and `None`
/// covers three states that must all behave identically: no config file at
/// all, `ProjectVimrc::Disabled`, and a file the sandbox rejected outright.
/// Applying the builtins outside that `if` is the whole reason it is a
/// parameter rather than something read here — the naive placement means a
/// security setting, or simply having no config, destroys the builtin
/// Ctrl+hjkl defaults.
///
/// `source` is only ever a diagnostic label: it is what a rejected line is
/// reported against, so `:panelmap` can name the file and the line number.
fn build_index(
    registry: &crate::actions::action::ActionRegistry,
    text: Option<&str>,
    source: &str,
) -> (
    crate::actions::bind::BindingIndex,
    Vec<crate::actions::bind::PanelDiagnostic>,
) {
    let mut index = crate::actions::bind::builtin_index(registry);
    let mut diagnostics = Vec::new();
    // The user layer, on top of the builtins. Two layers within ONE file —
    // `config::path::resolve` picks exactly one vimrc (EditorSettings
    // override, else res://, else user://), never several. Layering between
    // files is not a thing here and must not become one.
    if let Some(text) = text {
        crate::actions::bind::apply_text(
            &mut index,
            registry,
            text,
            &vim_core::keymap::MappingOwner::User,
            source,
            crate::actions::bind::Provenance::User,
            &mut diagnostics,
        );
    }
    (index, diagnostics)
}

#[cfg(test)]
mod build_index_tests {
    use super::build_index;
    use crate::actions::specs;

    /// The shipped keyset with no user config at all — the zero-config case.
    fn builtin_count() -> usize {
        let reg = specs::registry();
        build_index(&reg, None, "user://.godot-vimrc").0.len()
    }

    #[test]
    fn no_config_file_still_yields_the_builtin_keyset() {
        // The regression this function's shape exists to prevent: reading the
        // vimrc INSIDE the `if let Some(text)` and building the builtins there
        // too means a user with no config — or with `ProjectVimrc::Disabled` —
        // loses Ctrl+hjkl entirely.
        assert!(
            builtin_count() > 0,
            "the builtin defaults must load with no vimrc"
        );
        let reg = specs::registry();
        let (index, diagnostics) = build_index(&reg, None, "user://.godot-vimrc");
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        assert!(
            index
                .rule_for("panel", &[vim_core::keymap::KeyEvent::ctrl('h')])
                .is_some(),
            "panel <C-h> is a shipped default and must survive the no-file path"
        );
    }

    #[test]
    fn an_empty_vimrc_is_indistinguishable_from_no_vimrc() {
        let reg = specs::registry();
        assert_eq!(
            build_index(&reg, Some(""), "user://.godot-vimrc").0.len(),
            builtin_count()
        );
    }

    #[test]
    fn a_user_line_layers_on_top_of_the_builtins() {
        let reg = specs::registry();
        let (index, diagnostics) = build_index(
            &reg,
            Some("panelmap dock q godotvim.item.activate\n"),
            "user://.godot-vimrc",
        );
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        assert_eq!(
            index.len(),
            builtin_count() + 1,
            "the user rule must ADD to the builtins, not replace them"
        );
        assert!(index
            .rule_for(
                "dock",
                &[vim_core::keymap::KeyEvent::new(
                    vim_core::keymap::Key::Char('q'),
                    vim_core::keymap::Modifiers::NONE,
                )]
            )
            .is_some());
    }

    #[test]
    fn one_bad_line_costs_only_itself_and_names_its_source() {
        // Warn-and-skip is per line, and the diagnostic has to carry the file
        // and the line number or `:panelmap` cannot tell the user where to
        // look — which is the same as the rejection being silent.
        let reg = specs::registry();
        let (index, diagnostics) = build_index(
            &reg,
            Some(
                "panelmap dock q godotvim.item.activate\n\
                 panelmap dock w godotvim.item.nextt\n",
            ),
            "res://.godot-vimrc",
        );
        assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
        let rendered = diagnostics[0].to_string();
        assert!(
            rendered.starts_with("res://.godot-vimrc:2:"),
            "the diagnostic must name the file and the line: {rendered}"
        );
        assert_eq!(
            index.len(),
            builtin_count() + 1,
            "the good line above the bad one must still install"
        );
    }
}
