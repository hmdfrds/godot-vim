//! Input processing subsystem — mappings, quantum insert, completion.

use crate::bridge::vim_adapter::key::KeyConsumerPipeline;
use crate::bridge::vim_adapter::mapping::{MappingState, MappingStore};
use godot::classes::Timer;
use godot::prelude::*;
use std::sync::Arc;

/// Input processing state: mappings, quantum insert, completion.
pub struct InputSubsystem {
    /// Custom keymapping state (tracks pending keys and timeout)
    pub mapping_state: MappingState,
    /// Custom keymapping store (jj -> Esc, etc.) - Arc for sharing with `KeyContext`
    pub mapping_store: Arc<MappingStore>,
    /// Timer for mapping timeout (flushes pending keys)
    pub mapping_timer: Option<Gd<Timer>>,
    /// Local buffer for quantum insert optimization (batches rapid typing)
    pub quantum_buffer: String,
    /// Completion state manager
    pub completion_manager: crate::bridge::vim_adapter::managers::completion::CompletionManager,
    /// Key consumer pipeline for processing key events
    pub key_pipeline: KeyConsumerPipeline,
}

impl InputSubsystem {
    /// Creates a new InputSubsystem with default state.
    pub fn new() -> Self {
        Self {
            mapping_state: MappingState::default(),
            mapping_store: Arc::new(MappingStore::default()),
            mapping_timer: None,
            quantum_buffer: String::new(),
            completion_manager: crate::bridge::vim_adapter::managers::completion::CompletionManager,
            key_pipeline: crate::bridge::vim_adapter::key::build_pipeline(),
        }
    }
}
