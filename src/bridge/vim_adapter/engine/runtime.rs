use vim_core::domain::snapshot::DocumentSnapshot;
use vim_core::runtime::execute_action_with_capabilities;
use vim_core::inputs::commands::Action;
use vim_core::inputs::commands::parser::{parse_command, parse_ex_command};
use vim_core::inputs::commands::parser::ex_completion::complete_command;
use vim_core::inputs::VimKey;
use vim_core::state::store::{decode_state, encode_state, PersistError};
use vim_core::state::mode::Mode;
use vim_core::state::VimState;

use crate::bridge::types::key::KeyEvent;
use crate::bridge::vim_adapter::contracts::{ExecutionContext, InputPolicy};
use crate::bridge::vim_adapter::convert;
use crate::bridge::vim_adapter::effect_converter;
use crate::bridge::vim_adapter::output::VimOutput;

use super::VimEngine;

impl VimEngine {
    /// Creates a new engine with default state.
    #[must_use]
    pub fn new() -> Self {
        let mut state = VimState::new();
        state.capabilities.has_clipboard = true;

        Self {
            state,
            effects: vim_core::runtime::EffectAccumulator::new(),
            config: vim_core::state::config::Config::default(),
        }
    }

    /// Replace runtime state from a persisted, schema-validated payload.
    pub(crate) fn import_persisted_state_json(&mut self, raw: &str) -> Result<(), PersistError> {
        let mut restored = decode_state(raw)?;
        // Capability ports are runtime-derived; keep editor clipboard support enabled.
        restored.capabilities.has_clipboard = true;
        self.state = restored;
        Ok(())
    }

    /// Export runtime state into a schema-versioned payload.
    pub(crate) fn export_persisted_state_json(&self) -> Result<String, PersistError> {
        encode_state(&self.state)
    }

    /// Reset runtime state to defaults (used after schema mismatch / decode failure).
    pub(crate) fn reset_runtime_state(&mut self) {
        let mut state = VimState::new();
        state.capabilities.has_clipboard = true;
        self.state = state;
    }

    /// Canonical key execution entrypoint with explicit input policy.
    pub fn process_key_with_policy<D: DocumentSnapshot + ?Sized>(
        &mut self,
        key: &VimKey,
        policy: InputPolicy,
        doc: &D,
        context: ExecutionContext<'_>,
    ) -> Option<VimOutput> {
        let has_count = match policy {
            InputPolicy::Exclusive => vim_core::runtime::transition::has_count(&self.state),
            InputPolicy::Passive => false,
        };
        let action = parse_command(key, &self.state, has_count).ok()?;
        Some(self.process_action_with_context(action, doc, context))
    }

    /// Internal runtime action entrypoint for adapter controller paths.
    pub(crate) fn process_action_with_context<D: DocumentSnapshot + ?Sized>(
        &mut self,
        action: Action,
        doc: &D,
        context: ExecutionContext<'_>,
    ) -> VimOutput {
        self.execute_action(action, doc, context)
    }

    /// Internal Ex command execution path.
    ///
    /// Parsing remains runtime-owned and does not leak into controller modules.
    pub(crate) fn process_ex_command_with_context<D: DocumentSnapshot + ?Sized>(
        &mut self,
        command: &str,
        doc: &D,
        context: ExecutionContext<'_>,
    ) -> VimOutput {
        let action = self.parse_ex_action(command);
        self.execute_action(action, doc, context)
    }

    fn execute_action<D: DocumentSnapshot + ?Sized>(
        &mut self,
        action: Action,
        doc: &D,
        context: ExecutionContext<'_>,
    ) -> VimOutput {
        let prev_mode: Mode = self.state.mode();
        let position = convert::cursor_to_position(&context.cursor);

        self.effects.clear();

        execute_action_with_capabilities(
            &mut self.state,
            action,
            doc,
            position,
            &self.config,
            context.capabilities,
            &mut self.effects,
        );

        effect_converter::effects_to_output(&mut self.effects, &prev_mode, &self.state)
    }

    /// Convert bridge key event type into canonical vim key.
    #[inline]
    #[must_use]
    pub(crate) fn to_vim_key(key: &KeyEvent) -> VimKey {
        convert::key_event_to_vim_key(key)
    }

    /// Ex completion against the canonical command registry.
    #[must_use]
    pub(crate) fn complete_ex(&self, query: &str) -> Vec<String> {
        complete_command(query, &self.state.registry)
    }

    fn parse_ex_action(&self, command: &str) -> Action {
        parse_ex_command(command, &self.state.registry)
    }
}
