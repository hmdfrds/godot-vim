//! Effect Converter — Translates vim-core effects into VimOutput.
//!
//! This is the single translation point between `EffectAccumulator`
//! (vim-core's raw side effects) and `VimOutput` (godot-vim's clean output).
//!
//! # Data Flow
//!
//! ```text
//! engine::execute_action()
//!       │
//!       ▼
//! EffectAccumulator { transaction, requests, pending_keys, state_diff }
//!       │
//!       ▼  effects_to_output()
//! VimOutput { mode, cursor, commands, transaction, pending_keys }
//! ```

use vim_core::inputs::VimKey;
use vim_core::protocol::messages::ProtocolRequest;
use vim_core::runtime::EffectAccumulator;
use vim_core::state::mode::Mode;
use vim_core::state::VimState;

use crate::bridge::vim_adapter::contracts::DispatchBatch;
use crate::bridge::vim_adapter::convert;
use crate::bridge::vim_adapter::output::VimOutput;

/// Convert an `EffectAccumulator` into a `VimOutput`.
///
/// This is the single translation point from vim-core's raw effects
/// to godot-vim's clean output type. Called by `VimEngine::process_key()`
/// and `VimEngine::process_action()`.
///
/// # Arguments
///
/// * `effects` - The accumulated side effects from engine execution
/// * `prev_mode` - Mode before execution (for detecting mode changes)
/// * `state` - Current VimState after execution (for reading cursor, mode)
pub fn effects_to_output(
    effects: &mut EffectAccumulator,
    _prev_mode: &Mode,
    _state: &VimState,
) -> DispatchBatch {
    // Drop state_diff — vim_state is mutated directly by engine execution (SSOT)
    effects.state_diff.take();

    // Convert protocol requests -> editor commands
    let mut commands = Vec::with_capacity(effects.requests.len());

    for req in &effects.requests {
        match req {
            ProtocolRequest::ShellOp(stable_req) => {
                let shell_req: vim_core::prelude::ShellRequest = stable_req.clone().into();
                commands.push(convert::shell_request_to_command(&shell_req));
            }
            ProtocolRequest::ClipboardSet(text) => {
                commands.push(crate::bridge::types::command::EditorCommand::ClipboardSet(
                    text.clone(),
                ));
            }
            ProtocolRequest::ShowMessage { text, .. } => {
                commands.push(crate::bridge::types::command::EditorCommand::Message(
                    text.clone(),
                ));
            }
            _ => {}
        }
    }

    // Extract transaction and pending keys
    let transaction = effects.transaction.take();
    let pending_keys: Vec<VimKey> = std::mem::take(&mut effects.pending_keys).to_vec();

    VimOutput {
        commands,
        transaction,
        pending_keys,
    }
}
