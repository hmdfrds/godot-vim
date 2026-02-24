//! Key event processing logic.
//!
//! Contains the main entry point for processing key events through the consumer pipeline.

use crate::bridge::vim_adapter::mapping::MappingStore;
use std::sync::Arc;
use vim_core::inputs::VimKey;
use vim_core::state::VimState;

use super::context::KeyContext;
use super::pipeline::KeyConsumerPipeline;
use super::ConsumeResult;

/// Process a key event through the consumer pipeline.
///
/// This is the main entry point for key processing. It:
/// 1. Creates a `KeyContext` with all necessary state
/// 2. Processes through the pipeline
/// 3. Returns the result and whether input was handled
///
/// # Arguments
///
/// * `pipeline` - The consumer pipeline to process through
/// * `key` - The key event to process
/// * `vim_state` - Current Vim state
/// * `mapping_store` - Mapping store for custom key mappings (passed by reference to avoid Arc clone)
/// * `pending_keys` - Any pending keys from mapping state
/// * `completion_active` - Whether code completion popup is active
///
/// # Returns
///
/// A tuple of (`ConsumeResult`, `input_handled`, `trace_messages`)
#[must_use]
pub fn process_key_event(
    pipeline: &KeyConsumerPipeline,
    key: VimKey,
    vim_state: &VimState,
    mapping_store: &Arc<MappingStore>,
    pending_keys: &[VimKey],
    completion_active: bool,
) -> (ConsumeResult, bool, Vec<String>) {
    let mut ctx = KeyContext::new(
        key,
        vim_state,
        Arc::clone(mapping_store),
        pending_keys,
        completion_active,
    );

    let result = pipeline.process(&mut ctx);

    (result, ctx.input_handled, ctx.trace_messages)
}
