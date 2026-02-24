//! Key input handling infrastructure.
//!
//! Implements a consumer pipeline pattern 
//! where each `KeyConsumer` handles a specific concern (mapping, completion, etc.),
//! and the pipeline orchestrates them in order.
//!
//! # Architecture
//!
//! - `KeyContext`: Shared context passed through the pipeline
//! - `KeyConsumer`: Trait for individual key handlers
//! - `KeyConsumerPipeline`: Orchestrates consumer chain
//! - `processor`: Key event processing entry point
//!
//!
//! # Usage
//!
//! ```ignore
//! use crate::bridge::vim_adapter::key::{build_pipeline, process_key_event, KeyContext};
//!
//! let pipeline = build_pipeline();
//! let result = process_key_event(&pipeline, key, &vim_state, mapping_store, &[], false);
//! ```

mod builder;
mod consumer;
mod context;
mod pipeline;
mod processor;

pub mod consumers;

// Re-export public API
pub use builder::build_pipeline;
pub use consumer::ConsumeResult;
pub use context::KeyContext;
pub use pipeline::KeyConsumerPipeline;
pub use processor::process_key_event;

#[cfg(test)]
mod consumer_tests;
