//! Key processing methods for VimController.
//!
//! Handles the input pipeline: mapping, cmdline mode, macro recording,
//! code completion, and vim command execution.

mod completion_flow;
mod macro_flow;
mod mapping_flow;
mod policy_flow;

use crate::bridge::vim_wrapper::VimController;
use vim_core::inputs::VimKey;

impl VimController {
    /// Process a `VimKey` through the core Vim state machine.
    ///
    /// This is the internal processing path used for:
    /// - Timeout key replay (with `allow_mapping` control)
    /// - Direct key processing (bypassing Godot input)
    ///
    /// Macro recording is not done here. Recording happens in `try_handle_macro_recording`
    /// which runs before this function. This ensures macros record raw keys (before mapping
    /// expansion), so mappings are re-evaluated during playback.
    pub(crate) fn process_vim_key_internal(
        &mut self,
        vim_key: &VimKey,
        allow_mapping: bool,
        from_user_input: bool,
    ) {
        // Try mapping path first
        if allow_mapping && self.try_process_mapping(vim_key, from_user_input) {
            return;
        }

        // Quantum insert fast path
        if self.try_handle_quantum_insert(vim_key, from_user_input) {
            return;
        }

        // Canonical Vim processing
        self.execute_vim_action(vim_key);
    }

    pub(crate) fn execute_vim_action(&mut self, vim_key: &VimKey) {
        self.commit_quantum_buffer();
        self.execute_key_with_visuals(
            vim_key,
            crate::bridge::vim_adapter::contracts::InputPolicy::Exclusive,
        );
    }

    /// Runs the key through the consumer pipeline and logs a summary trace.
    pub(crate) fn trace_key_pipeline(&self, vim_key: &VimKey) {
        if !log::log_enabled!(log::Level::Trace) {
            return;
        }
        if let Some(editor) = self.get_editor() {
            let completion_active = editor.get_code_completion_selected_index() >= 0;
            let (result, _handled, trace) = crate::bridge::vim_adapter::key::process_key_event(
                &self.input.key_pipeline,
                *vim_key,
                self.engine.vim_state_ref(),
                &self.input.mapping_store,
                self.input.mapping_state.pending_keys(),
                completion_active,
            );
            log::trace!("Pipeline: {} → {:?}", trace.join(" → "), result);
        }
    }
}
