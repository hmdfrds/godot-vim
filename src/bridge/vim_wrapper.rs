//! VimController - The Godot-Vim bridge.
//!
//! This module contains the core VimController struct and its Godot API.
//! Handler methods are split into submodules under `controller/`.
//!
//! # Extracted Modules
//!
//! Signal handlers, cmdline callbacks, dock handlers, and utilities
//! are extracted into sibling files:
//! - `vim_wrapper_signals.rs` — signal `#[func]` methods
//! - `vim_wrapper_cmdline.rs` — cmdline callback `#[func]` methods
//! - `vim_wrapper_dock.rs` — dock observation `#[func]` methods
//! - `vim_wrapper_util.rs` — cursor utilities and `extract_word_at_col`

use crate::bridge::components::line_numbers::LineNumberManager;
use crate::bridge::godot::api::{get_editor_config, EditorConfig};
use crate::bridge::settings;
use crate::bridge::settings::accessors::VimSettings;
use crate::bridge::vim_adapter::controller::attach_session::AttachSession;
use crate::bridge::vim_adapter::mapping::GodotMappingLoader;
use crate::bridge::vim_adapter::subsystems::dock::DockSubsystem;
use crate::bridge::vim_adapter::subsystems::input::InputSubsystem;
use crate::bridge::vim_adapter::subsystems::ui::UiSubsystem;
use crate::bridge::vim_adapter::subsystems::visuals::VisualSubsystem;

use crate::bridge::godot::names::{callbacks, timer};
use crate::bridge::vim_adapter::engine::VimEngine;

use godot::classes::{CodeEdit, Control, INode, InputEvent, Node, Timer};
use godot::prelude::*;
use std::sync::Arc;
use vim_core::inputs::mapping::MappingStore;

#[derive(GodotClass)]
#[class(base=Node)]
pub struct VimController {
    #[base]
    base: Base<Node>,

    // ── Core State (cross-cutting — stays flat) ─────────────────────────
    /// The VimEngine — owns VimState, EffectAccumulator, and Config.
    pub(crate) engine: VimEngine,
    /// Reference to the attached CodeEdit (for signal disconnection)
    pub(crate) attached_editor: Option<Gd<CodeEdit>>,
    /// Explicit attach lifecycle state machine.
    pub(crate) attach_session: AttachSession,
    /// Editor configuration (indent size, tabs vs spaces, etc.)
    pub(crate) editor_config: EditorConfig,

    // ── Subsystems ──────────────────────────────────────────────────────
    /// Input processing: mappings, quantum insert, completion
    pub(crate) input: InputSubsystem,
    /// UI components: cmdline, cursor overlay, line numbers
    pub(crate) ui: UiSubsystem,
    /// Visual state: search highlights, substitute preview, messages
    pub(crate) visuals: VisualSubsystem,
    /// Dock observation and deferred command queue
    pub(crate) dock: DockSubsystem,
}

#[godot_api]
impl INode for VimController {
    fn init(base: Base<Node>) -> Self {
        Self {
            base,
            engine: VimEngine::new(),
            attached_editor: None,
            attach_session: AttachSession::new(),
            editor_config: EditorConfig::default(),
            input: InputSubsystem::new(),
            ui: UiSubsystem::new(),
            visuals: VisualSubsystem::new(),
            dock: DockSubsystem::new(),
        }
    }

    fn ready(&mut self) {
        // Fetch editor configuration from Godot settings
        self.editor_config = get_editor_config();

        // Cache vim-core config to avoid per-motion FFI calls.
        self.refresh_cached_config();

        // Restore persisted runtime state (schema-validated envelope).
        self.restore_runtime_state();

        // Load key mappings from settings.
        self.reload_mapping_store("ready");

        // Create mapping timeout timer
        let mut timer = Timer::new_alloc();
        timer.set_one_shot(true);
        let callable = self.base().callable(callbacks::ON_MAPPING_TIMEOUT);
        timer.connect(timer::signals::TIMEOUT, &callable);
        self.base_mut().add_child(&timer);
        log::debug!(
            "Mapping timer created in_tree={} wait_time={}",
            timer.is_inside_tree(),
            timer.get_wait_time()
        );
        self.input.mapping_timer = Some(timer);

        // Initialize LineNumberManager
        let line_number_manager = LineNumberManager::new_alloc();
        self.base_mut()
            .add_child(&line_number_manager.clone().upcast::<Node>());
        self.ui.line_number_manager = Some(line_number_manager);

        // Enable process for queued command execution.
        self.base_mut().set_process(true);

        // Register Godot-specific commands
        self.register_shell_commands();
    }

    fn process(&mut self, _delta: f64) {
        crate::bridge::safety::guard(
            || {
                if !self.dock.pending_commands.is_empty() {
                    log::debug!(
                        "Processing {} pending commands",
                        self.dock.pending_commands.len()
                    );
                }
                self.flush_command_queue();
            },
            (),
        );
    }
}

#[godot_api]
impl VimController {
    // ═══════════════════════════════════════════════════════════════════════════════
    // Public Godot API (#[func] methods)
    // ═══════════════════════════════════════════════════════════════════════════════

    /// Designated initializer called by `GodotVimPlugin`
    #[func]
    pub fn set_log_level(&mut self, level: i32) {
        let _ = self;
        let settings_level = crate::bridge::settings::LogLevel::from(i64::from(level));
        let filter = match settings_level {
            crate::bridge::settings::LogLevel::Error => log::LevelFilter::Error,
            crate::bridge::settings::LogLevel::Warn => log::LevelFilter::Warn,
            crate::bridge::settings::LogLevel::Info => log::LevelFilter::Info,
            crate::bridge::settings::LogLevel::Debug => log::LevelFilter::Debug,
            crate::bridge::settings::LogLevel::Trace => log::LevelFilter::Trace,
            crate::bridge::settings::LogLevel::Off => log::LevelFilter::Off,
        };
        log::set_max_level(filter);
    }

    /// Reloads key mappings from `ProjectSettings`.
    #[func]
    pub fn reload_mappings(&mut self) {
        log::trace!("Reloading mappings from settings");
        self.reload_mapping_store("reload_mappings");
        log::debug!("Loaded {} mappings", self.input.mapping_store.count());
    }

    /// Request command completions for a partial query.
    /// Returns a PackedStringArray for Godot usage.
    #[func]
    pub fn request_completion(&self, query: String) -> PackedStringArray {
        let completions = self.engine.complete_ex(&query);
        PackedStringArray::from_iter(completions.iter().map(GString::from))
    }

    fn register_shell_commands(&mut self) {
        let reg = self.engine.registry_mut();

        // Debugging
        reg.register_simple("GodotBreakpoint");
        reg.register_simple("toggle_breakpoint");
        reg.register_simple("GodotContinue");
        reg.register_simple("debug_continue");
        reg.register_simple("GodotNext");
        reg.register_simple("debug_next");
        reg.register_simple("GodotStepIn");
        reg.register_simple("debug_step_in");
        reg.register_simple("GodotStepOut");
        reg.register_simple("debug_step_out");
        reg.register_simple("GodotPause");
        reg.register_simple("debug_pause");

        // Custom commands - Tool panels
        reg.register_simple("Scene");
        reg.register_simple("FileSystem");
        reg.register_simple("Inspector");
        reg.register_simple("Script");
        reg.register_simple("FocusDock");
        reg.register_simple("window_nav");

        // Scene Control
        reg.register_simple("run");
        reg.register_simple("play");
        reg.register_simple("runcurrent");
        reg.register_simple("playcurrent");
        reg.register_simple("stop");

        // Scene Management
        reg.register_simple("save");
        reg.register_simple("saveall");

        // Editor State
        reg.register_simple("zen");
        reg.register_simple("unzen");
        reg.register_simple("restart");
    }

    /// Reloads all settings from `ProjectSettings`.
    /// Call this after modifying settings in Project Settings to apply changes immediately.
    #[func]
    pub fn reload_settings(&mut self) {
        log::trace!("Reloading all settings");

        // Sync log level
        settings::sync_all_settings();

        if let Some(mut line_manager) = self.ui.line_number_manager.clone() {
            line_manager.bind_mut().sync_settings();
        }

        // Reload mappings
        self.reload_mapping_store("reload_settings");

        // Refresh cached vim-core config.
        self.refresh_cached_config();

        // Re-apply cursor visuals with current settings
        if let Some(mut editor) = self.get_editor() {
            let mode = self.engine.mode();
            self.update_cursor_visuals(&mode, &mut editor);
        }

        log::info!("Settings reloaded");
    }

    // ═══════════════════════════════════════════════════════════════════════════════
    // Messaging (used by many callers, stays inline)
    // ═══════════════════════════════════════════════════════════════════════════════

    /// Queues an informational message to be shown in the cmdline.
    pub fn show_cmdline_message(&mut self, message: &str) {
        self.visuals.pending_message = Some(message.to_string());
    }

    /// Displays the pending message in the cmdline. Call after mode change.
    pub fn flush_pending_message(&mut self) {
        if let Some(message) = self.visuals.pending_message.take() {
            if let Some(cmdline) = self.ui.cmdline.as_mut().filter(|c| c.is_instance_valid()) {
                cmdline.bind_mut().show_message(&message);
            }
        }
    }

    // ═══════════════════════════════════════════════════════════════════════════════
    // Utility Methods (kept inline — used everywhere)
    // ═══════════════════════════════════════════════════════════════════════════════

    /// Helper to get the attached `CodeEdit` (returns None if freed)
    pub(crate) fn get_editor(&self) -> Option<Gd<CodeEdit>> {
        self.attached_editor.as_ref().and_then(|editor| {
            if editor.is_instance_valid() {
                Some(editor.clone())
            } else {
                None
            }
        })
    }

    /// Marks the input as handled in the current editor's viewport.
    /// Uses the editor's own viewport for floating windows; `self.base().get_viewport()`
    /// would target the main window and leave the event unhandled in the floating one.
    pub(crate) fn set_input_handled(&self) {
        if let Some(editor) = self.get_editor() {
            if let Some(mut vp) = editor.get_viewport() {
                vp.set_input_as_handled();
            }
        }
    }

    /// Refresh the cached vim-core config.
    /// Called in ready(), reload_settings(), and attach() to avoid
    /// per-motion FFI calls and allocation overhead.
    pub(crate) fn refresh_cached_config(&mut self) {
        self.engine.update_config(
            self.editor_config.indent_size,
            self.editor_config.use_tabs,
            VimSettings::is_keyword(),
            VimSettings::scroll_offset() as usize,
            VimSettings::yank_to_clipboard(),
            VimSettings::delete_to_clipboard(),
        );
    }

    fn reload_mapping_store(&mut self, source: &str) {
        match GodotMappingLoader::load() {
            Ok(store) => {
                self.input.mapping_store = Arc::new(store);
            }
            Err(error) => {
                self.input.mapping_store = Arc::new(MappingStore::default());
                log::error!(
                    "Mapping schema error in {}: {}, mappings disabled until fixed",
                    source,
                    error
                );
                self.show_cmdline_message(
                    "Mapping configuration error: use plugins/GodotVim/mapping/all only.",
                );
            }
        }
    }

    // ═══════════════════════════════════════════════════════════════════════════════
    // Signal Handler Stubs — bodies in vim_wrapper_signals.rs
    // ═══════════════════════════════════════════════════════════════════════════════

    #[func]
    fn on_mapping_timeout(&mut self) {
        self.on_mapping_timeout_impl();
    }

    #[func]
    fn on_scrollbar_changed(&mut self, value: f64) {
        self.on_scrollbar_changed_impl(value);
    }

    #[func]
    pub(crate) fn on_cursor_visual_update(&mut self) {
        self.on_cursor_visual_update_impl();
    }

    #[func]
    fn on_caret_moved(&mut self) {
        self.on_caret_moved_impl();
    }

    #[func]
    #[allow(
        clippy::needless_pass_by_value,
        reason = "Godot signal callbacks require Gd<T> by value"
    )]
    fn handle_gui_input(&mut self, event: Gd<InputEvent>) {
        self.handle_gui_input_impl(event);
    }

    // ═══════════════════════════════════════════════════════════════════════════════
    // Cmdline Callback Stubs — bodies in vim_wrapper_cmdline.rs
    // ═══════════════════════════════════════════════════════════════════════════════

    #[func]
    #[allow(
        clippy::needless_pass_by_value,
        reason = "Godot signal callbacks require GString by value"
    )]
    fn on_cmd_submitted(&mut self, text: GString) {
        self.on_cmd_submitted_impl(text);
    }

    #[func]
    #[allow(
        clippy::needless_pass_by_value,
        reason = "Godot signal callbacks require GString by value"
    )]
    fn on_cmd_text_changed(&mut self, new_text: GString) {
        self.on_cmd_text_changed_impl(new_text);
    }

    #[func]
    fn on_cmd_input_gui_input(&mut self, event: Gd<InputEvent>) {
        self.on_cmd_input_gui_input_impl(event);
    }

    // ═══════════════════════════════════════════════════════════════════════════════
    // Dock Handler Stubs — bodies in vim_wrapper_dock.rs
    // ═══════════════════════════════════════════════════════════════════════════════

    #[func]
    pub fn observe_dock_control(&mut self, control: Gd<Control>) {
        self.observe_dock_control_impl(control);
    }

    #[func]
    fn on_dock_gui_input(&mut self, event: Gd<InputEvent>) {
        self.on_dock_gui_input_impl(event);
    }

    #[func]
    pub fn execute_command_deferred(&mut self, cmd: String, args: PackedStringArray) {
        self.execute_command_deferred_body(cmd, args);
    }
}
