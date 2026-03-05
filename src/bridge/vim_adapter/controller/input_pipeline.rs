//! Input pipeline methods for VimController.
//!
//! Implements the input processing pipeline:
//! - `parse_and_filter` — Parse Godot event → VimKey, apply pre-filters
//! - `try_window_nav` — Intercept Ctrl+HJKL for window navigation
//! - `record_macro_key` — Record raw key before mapping expansion
//! - `dispatch_by_priority` — Classify Exclusive/Passive/Ignore
//! - `dispatch_exclusive` / `dispatch_passive` — Final dispatch

use crate::bridge::navigation::window::nav::{handle_window_nav, NavDirection, WindowNavResult};
use crate::bridge::safety::input::parse_godot_event;
use crate::bridge::settings;
use crate::bridge::settings::accessors::VimSettings;
use crate::bridge::types::command::EditorCommand;
use crate::bridge::types::cursor::CursorPos;
use crate::bridge::vim_adapter::contracts::{ExecutionContext, InputPolicy};
use crate::bridge::vim_adapter::convert::key_event_to_vim_key;
use crate::bridge::vim_adapter::core::cast::i32_to_usize;
use crate::bridge::vim_adapter::core::column_codec;
use crate::bridge::vim_adapter::core::snapshot::LazyGodotSnapshot;
use crate::bridge::vim_adapter::handlers::cmdline::IncsearchHandler;
use crate::bridge::vim_wrapper::VimController;
use vim_core::inputs::whitelist::{classify_input, InputPriority};
use vim_core::inputs::{KeyCode, VimModifiers};
use vim_core::state::mode::Mode;

impl VimController {
    /// Parses the Godot event into a `VimKey` and applies pre-filters.
    ///
    /// Returns `None` (caller should return) if:
    /// - Editor is not focused (filters echoed events from floating windows)
    /// - Vim is disabled in settings
    /// - Event is not a recognized key event
    pub(crate) fn parse_and_filter(
        &self,
        event: &godot::prelude::Gd<godot::classes::InputEvent>,
    ) -> Option<vim_core::inputs::VimKey> {
        if let Some(editor) = self.get_editor() {
            if !editor.has_focus() {
                return None;
            }
        }

        if !settings::VimSettings::enabled() {
            return None;
        }

        let key_event = parse_godot_event(event)?;
        let vim_key = key_event_to_vim_key(&key_event);
        log::trace!("VimKey: {}", vim_key);
        Some(vim_key)
    }

    /// Intercepts Ctrl+HJKL for window navigation (if enabled).
    ///
    /// These bypass all Vim processing (mappings, macros, etc.) to allow
    /// jumping between docks/editors even when CodeEdit has focus.
    /// Returns `true` if the key was consumed.
    pub(crate) fn try_window_nav(&mut self, vim_key: &vim_core::inputs::VimKey) -> bool {
        if !VimSettings::window_nav_enabled() {
            return false;
        }

        // Strict Ctrl+Key check
        if vim_key.modifiers != VimModifiers::CTRL {
            return false;
        }

        let direction = match vim_key.code {
            KeyCode::Char('h') => "left",
            KeyCode::Char('j') => "down",
            KeyCode::Char('k') => "up",
            KeyCode::Char('l') => "right",
            _ => return false,
        };

        self.execute_window_nav_command(direction);
        self.set_input_handled();
        true
    }

    /// Helper to execute window navigation immediately
    pub(crate) fn execute_window_nav_command(&mut self, direction: &str) {
        let nav_dir = match direction {
            "left" => NavDirection::Left,
            "right" => NavDirection::Right,
            "up" => NavDirection::Prev,
            "down" => NavDirection::Next,
            _ => return,
        };

        if let Some(editor) = self.get_editor() {
            let control = editor.upcast::<godot::classes::Control>();
            let result = handle_window_nav(&control, nav_dir);

            match result {
                WindowNavResult::Focused(target) => {
                    self.observe_dock_control(target);
                }
                WindowNavResult::Ignored => {}
            }
        }
    }

    /// Records the raw key for macro playback before mapping expansion.
    ///
    /// Records every key except 'q' (which toggles recording).
    /// This ensures macros replay raw input and mappings are re-evaluated during playback.
    pub(crate) fn record_macro_key(&mut self, vim_key: &vim_core::inputs::VimKey) {
        if self.engine.recording_register().is_some()
            && !matches!(vim_key.code, vim_core::inputs::KeyCode::Char('q'))
        {
            self.engine.record_macro_key(*vim_key);
        }
    }

    /// Classifies input priority and dispatches to the appropriate handler.
    pub(crate) fn dispatch_by_priority(&mut self, vim_key: &vim_core::inputs::VimKey) {
        let user_passthrough = settings::VimSettings::key_passthrough_list();
        let mut priority = classify_input(vim_key, &self.engine.mode(), &user_passthrough);

        // Upgrade priority for completion interaction keys when completion popup is active
        if let Some(editor) = self.get_editor() {
            if self
                .input
                .completion_manager
                .should_intercept(vim_key, &editor)
            {
                log::debug!(
                    "Upgrading priority to Exclusive for completion key: {}",
                    vim_key
                );
                priority = InputPriority::Exclusive;
            }
        }

        match priority {
            InputPriority::Ignore => {}
            InputPriority::Exclusive => self.dispatch_with_policy(vim_key, InputPolicy::Exclusive),
            InputPriority::Passive => self.dispatch_with_policy(vim_key, InputPolicy::Passive),
        }
    }

    fn dispatch_with_policy(&mut self, vim_key: &vim_core::inputs::VimKey, policy: InputPolicy) {
        match policy {
            InputPolicy::Exclusive => self.dispatch_exclusive(vim_key),
            InputPolicy::Passive => self.dispatch_passive(vim_key),
        }
    }

    /// Handles Exclusive-priority input: Vim consumes the event entirely.
    ///
    /// Processes through a chain of handlers in priority order:
    /// CmdLine → ESC highlights → Completion → Macro recording → Quantum insert → Fast motion → Full Vim action
    pub(crate) fn dispatch_exclusive(&mut self, vim_key: &vim_core::inputs::VimKey) {
        self.set_input_handled();

        if self.try_handle_cmdline_mode(vim_key) {
            return;
        }

        // Normal mode ESC: clear search highlights
        if matches!(self.engine.mode(), Mode::Normal) && vim_key.code == KeyCode::Esc {
            self.clear_incsearch_highlights();
            return;
        }

        if self.handle_code_completion(vim_key) {
            return;
        }

        if self.try_handle_macro_recording(vim_key) {
            return;
        }

        // Quantum Insert: batch rapid typing for O(1)
        if self.try_handle_quantum_insert(vim_key, true) {
            return;
        }

        self.execute_key_with_policy(vim_key, InputPolicy::Exclusive);
    }

    /// Handles Passive-priority input: Godot handles rendering, Vim updates state only.
    ///
    /// Does not consume the input. Executes through vim-core to update state (for `.` repeat)
    /// but ignores side effects that would duplicate Godot's work (e.g., TypeChar).
    pub(crate) fn dispatch_passive(&mut self, vim_key: &vim_core::inputs::VimKey) {
        // Handle 'q' to stop recording
        if self.try_handle_macro_recording(vim_key) {
            return;
        }

        self.execute_key_with_policy(vim_key, InputPolicy::Passive);
    }

    fn execute_key_with_policy(&mut self, vim_key: &vim_core::inputs::VimKey, policy: InputPolicy) {
        if policy == InputPolicy::Exclusive {
            self.execute_vim_action(vim_key);
            return;
        }

        // Passive policy: Godot handles rendering while vim-core updates state/repeat buffers.
        let Some(editor) = self.get_editor() else {
            return;
        };

        let doc = LazyGodotSnapshot::new(&editor);
        let line = i32_to_usize(editor.get_caret_line());
        let editor_col = i32_to_usize(editor.get_caret_column());
        let byte_col = column_codec::editor_col_to_byte_in_editor(&editor, line, editor_col);
        let cursor = CursorPos::new(line, byte_col);
        let context = ExecutionContext::from_snapshot(cursor, &doc);

        let Some(output) = self
            .engine
            .process_key_with_policy(vim_key, policy, &doc, context)
        else {
            return;
        };

        let is_backspace_key = matches!(vim_key.code, vim_core::inputs::KeyCode::Backspace)
            || (matches!(vim_key.code, vim_core::inputs::KeyCode::Char('h'))
                && vim_key
                    .modifiers
                    .contains(vim_core::inputs::VimModifiers::CTRL));
        let is_replace_backspace = is_backspace_key
            && matches!(
                self.engine.mode(),
                vim_core::state::mode::Mode::Replace(vim_core::state::mode::ReplaceMode::Overwrite) | vim_core::state::mode::Mode::Replace(vim_core::state::mode::ReplaceMode::Virtual)
            );

        if is_replace_backspace {
            self.set_input_handled();
        }

        // Apply transaction if any (e.g., Replace mode Backspace restore)
        if let Some(tx) = output.transaction {
            self.apply_transaction(tx);
            self.set_input_handled();
            self.sync_cursor_to_editor();
        }

        // Passive mode: commands are not dispatched (Godot handles rendering).
        // Log filtered requests for debugging only.
        for cmd in &output.commands {
            match cmd {
                EditorCommand::TypeChar(_) | EditorCommand::Backspace => {}
                _ => {
                    log::debug!("Passive mode ignored command: {cmd:?}");
                }
            }
        }
    }
}
