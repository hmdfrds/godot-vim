//! Plugin lifecycle helpers: symmetric init/teardown pairs for settings,
//! mapping timer, and editor signals.
//!
//! Every `init_*` has a matching `teardown_*` so signal connect/disconnect
//! pairing is visually verifiable. Floating window lifecycle lives in
//! [`super::floating`].

use godot::classes::{EditorInterface, Timer};
use godot::prelude::*;

use super::{
    GodotVimPlugin, SIG_EDITOR_SCRIPT_CHANGED, SIG_GUI_FOCUS_CHANGED,
    SIG_SETTINGS_CHANGED, SIG_TIMEOUT,
};
use super::signals::{connect_deferred, connect_immediate, safe_disconnect};

impl GodotVimPlugin {
    // ── Settings ──────────────────────────────────────────────────────

    /// Register EditorSettings keys, read initial values, source the config
    /// file, and connect `settings_changed` for live reload.
    pub(super) fn init_settings(&mut self) {
        let Some(mut editor_settings) = EditorInterface::singleton().get_editor_settings() else {
            log::warn!("EditorSettings unavailable, using default VimOptions");
            return;
        };

        crate::settings::registration::register_all(&mut editor_settings);
        log::debug!("enter_tree: settings registered");

        let snapshot = crate::settings::reader::read_all(&editor_settings);
        crate::logging::set_level(snapshot.log_level);

        if let Some(controller) = &mut self.controller {
            controller.apply_settings(&snapshot);
        }

        self.settings = Some(snapshot);

        if !self.source_config_from_disk("enter_tree") {
            let path = self.resolve_config_path().path;
            log::debug!("enter_tree: no config file found at '{path}'");
        }

        let callable = self.base().callable("on_settings_changed");
        connect_immediate(&mut editor_settings, SIG_SETTINGS_CHANGED, &callable);
    }

    /// Symmetric teardown for `init_settings`.
    pub(super) fn teardown_settings(&mut self) {
        if let Some(mut editor_settings) = EditorInterface::singleton().get_editor_settings() {
            let callable = self.base().callable("on_settings_changed");
            safe_disconnect(&mut editor_settings, SIG_SETTINGS_CHANGED, &callable);
        }
    }

    // ── Mapping timer ─────────────────────────────────────────────────

    /// Create a one-shot Timer for mapping timeout resolution and add it
    /// to the scene tree so Godot manages its lifecycle.
    pub(super) fn init_mapping_timer(&mut self) {
        let mut timer = Timer::new_alloc();
        timer.set_one_shot(true);
        timer.set_wait_time(1.0); // placeholder; overridden each start
        let callable = self.base().callable("on_mapping_timeout");
        connect_immediate(&mut timer, SIG_TIMEOUT, &callable);
        self.base_mut()
            .add_child(&timer.clone().upcast::<Node>());
        self.mapping_timer = Some(timer);
    }

    /// Symmetric teardown for `init_mapping_timer`.
    pub(super) fn teardown_mapping_timer(&mut self) {
        if let Some(mut timer) = self.mapping_timer.take() {
            timer.stop();
            timer.queue_free();
        }
    }

    // ── Editor signals ────────────────────────────────────────────────

    /// Connect editor-level signals for tab switching and focus tracking.
    pub(super) fn connect_editor_signals(&mut self) {
        let interface = EditorInterface::singleton();

        let Some(mut script_editor) = interface.get_script_editor() else {
            log::warn!("ScriptEditor unavailable, GodotVim disabled");
            return;
        };

        // DEFERRED: dock navigation (Enter on ItemList) can trigger a script
        // change synchronously while input() still holds &mut self.
        let callable = self.base().callable("on_script_changed");
        connect_deferred(&mut script_editor, SIG_EDITOR_SCRIPT_CHANGED, &callable);

        if let Some(mut vp) = interface.get_base_control().and_then(|c| c.get_viewport()) {
            let callable = self.base().callable("on_focus_changed");
            connect_deferred(&mut vp, SIG_GUI_FOCUS_CHANGED, &callable);
        }
    }

    /// Symmetric teardown for `connect_editor_signals`.
    pub(super) fn disconnect_editor_signals(&mut self) {
        let interface = EditorInterface::singleton();

        if let Some(mut se) = interface.get_script_editor() {
            let callable = self.base().callable("on_script_changed");
            safe_disconnect(&mut se, SIG_EDITOR_SCRIPT_CHANGED, &callable);
        }

        if let Some(mut vp) = interface.get_base_control().and_then(|c| c.get_viewport()) {
            let callable = self.base().callable("on_focus_changed");
            safe_disconnect(&mut vp, SIG_GUI_FOCUS_CHANGED, &callable);
        }
    }
}
