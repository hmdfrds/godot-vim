//! Plugin lifecycle helpers: symmetric init/teardown pairs for settings,
//! mapping timer, and editor signals.
//!
//! Every `init_*` has a matching `teardown_*` so signal connect/disconnect
//! pairing is visually verifiable. Floating window lifecycle lives in
//! [`super::floating`].

use godot::classes::{EditorInterface, ProjectSettings, Timer};
use godot::prelude::*;

use super::signals::{
    connect_deferred, connect_immediate, safe_disconnect, SIG_EDITOR_SCRIPT_CHANGED,
    SIG_GUI_FOCUS_CHANGED, SIG_SETTINGS_CHANGED, SIG_TIMEOUT,
};
use super::GodotVimCore;

impl GodotVimCore {
    // ── Settings ──────────────────────────────────────────────────────

    /// Register EditorSettings keys, read initial values, and connect
    /// `settings_changed` for live reload. Config-sourcing and
    /// `apply_settings` are deferred to the enabled edge in
    /// `apply_enabled_state` so startup equals the runtime enable path.
    pub(super) fn init_settings(&mut self) {
        let Some(mut editor_settings) = EditorInterface::singleton().get_editor_settings() else {
            log::warn!("EditorSettings unavailable, using default VimOptions");
            self.enabled = crate::settings::defaults::ENABLED;
            return;
        };

        crate::settings::registration::register_all(&mut editor_settings);
        log::debug!("enter_tree: settings registered");

        let snapshot = crate::settings::reader::read_all(&editor_settings);
        crate::logging::set_level(snapshot.log_level);

        self.enabled = snapshot.enabled;
        self.settings = Some(snapshot);

        let callable = self.base().callable("on_settings_changed");
        connect_immediate(&mut editor_settings, SIG_SETTINGS_CHANGED, &callable);

        self.report_user_only_overrides();
        let callable = self.base().callable("on_project_settings_changed");
        connect_immediate(
            &mut ProjectSettings::singleton(),
            SIG_SETTINGS_CHANGED,
            &callable,
        );
    }

    /// Symmetric teardown for `init_settings`.
    pub(super) fn teardown_settings(&mut self) {
        if let Some(mut editor_settings) = EditorInterface::singleton().get_editor_settings() {
            let callable = self.base().callable("on_settings_changed");
            safe_disconnect(&mut editor_settings, SIG_SETTINGS_CHANGED, &callable);
        }
        let callable = self.base().callable("on_project_settings_changed");
        safe_disconnect(
            &mut ProjectSettings::singleton(),
            SIG_SETTINGS_CHANGED,
            &callable,
        );
    }

    /// Warns once per project override of a user-only setting, which the
    /// reader ignores. `godot_warn!` because Log Level defaults to Off.
    pub(super) fn report_user_only_overrides(&mut self) {
        let project = ProjectSettings::singleton();
        let current = crate::settings::overrides::overridden(|name| project.has_setting(name));
        if let Some(message) = self.override_notice.update(&current) {
            godot_warn!("{message}");
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
        self.base_mut().add_child(&timer.clone().upcast::<Node>());
        self.mapping_timer = Some(timer);
    }

    /// Symmetric teardown for `init_mapping_timer`.
    pub(super) fn teardown_mapping_timer(&mut self) {
        if let Some(mut timer) = self.mapping_timer.take() {
            timer.stop();
            timer.queue_free();
        }
    }

    // ── Shell (panel) sequence timer ──────────────────────────────────

    /// Create the shell plane's own one-shot Timer for pending-prefix
    /// resolution.
    ///
    /// **A second timer, never `mapping_timer`**, and the reason is not
    /// tidiness. `on_mapping_timeout_impl` opens with
    /// `let Some(editor) = &self.attached_editor else { return; }` — it
    /// early-returns with no editor attached, which is the *common* case for
    /// dock browsing, so a shared timer would silently never resolve a dock
    /// prefix. And when an editor *is* attached it would flush the **engine's**
    /// typeahead into the open file while focus is on a dock, which is a text
    /// mutation the user did not ask for.
    pub(super) fn init_panel_timer(&mut self) {
        let mut timer = Timer::new_alloc();
        timer.set_one_shot(true);
        timer.set_wait_time(1.0); // placeholder; overridden each start
        let callable = self.base().callable("on_panel_timeout");
        connect_immediate(&mut timer, SIG_TIMEOUT, &callable);
        self.base_mut().add_child(&timer.clone().upcast::<Node>());
        self.panel_timer = Some(timer);
    }

    /// Symmetric teardown for `init_panel_timer`.
    pub(super) fn teardown_panel_timer(&mut self) {
        if let Some(mut timer) = self.panel_timer.take() {
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

        // DEFERRED: focus moves mid-teardown, so immediate delivery would
        // re-enter the plugin from inside Godot's own signal emission. The
        // cost is that the focused Control can be freed before the queue
        // drains, which is why `on_focus_changed` takes a `Variant`.
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
