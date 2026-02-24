//! Key consumer pipeline - orchestrates the chain of key handlers.

use super::consumer::{ConsumeResult, KeyConsumer};
use super::context::KeyContext;

/// Pipeline that processes keys through a chain of consumers.
///
/// Each consumer is tried in order until one returns `Consumed` or `Passthrough`.
pub struct KeyConsumerPipeline {
    consumers: Vec<Box<dyn KeyConsumer>>,
}

impl KeyConsumerPipeline {
    /// Creates a new empty pipeline.
    #[must_use]
    pub fn new() -> Self {
        Self {
            consumers: Vec::new(),
        }
    }

    /// Adds a consumer to the end of the pipeline.
    pub fn add<C: KeyConsumer + 'static>(&mut self, consumer: C) {
        self.consumers.push(Box::new(consumer));
    }

    /// Processes a key through all applicable consumers.
    ///
    /// Returns the final `ConsumeResult`:
    /// - `Consumed` if any consumer consumed the key
    /// - `Passthrough` if the key should be passed to Godot
    /// - `NotConsumed` if no consumer handled the key
    pub fn process(&self, ctx: &mut KeyContext) -> ConsumeResult {
        for consumer in &self.consumers {
            if ctx.stop_processing {
                break;
            }

            if !consumer.is_applicable(ctx) {
                continue;
            }

            let result = consumer.consume(ctx);
            ctx.trace(format!("{}:{:?}", consumer.name(), result));

            match result {
                ConsumeResult::Consumed => {
                    return ConsumeResult::Consumed;
                }
                ConsumeResult::Passthrough => {
                    return ConsumeResult::Passthrough;
                }
                ConsumeResult::NotConsumed => {
                    // Continue to next consumer
                }
            }
        }

        ConsumeResult::NotConsumed
    }

    /// Returns the number of consumers in the pipeline.
    #[cfg(test)]
    #[must_use]
    pub fn len(&self) -> usize {
        self.consumers.len()
    }
}

impl Default for KeyConsumerPipeline {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bridge::vim_adapter::key::build_pipeline;
    use crate::bridge::vim_adapter::mapping::MappingStore;
    use std::sync::Arc;
    use vim_core::inputs::{KeyCode, VimKey, VimModifiers};
    use vim_core::state::VimState;

    /// Helper to create a test context
    fn test_context(key: VimKey, vim_state: &VimState) -> KeyContext<'_> {
        KeyContext::new(key, vim_state, Arc::new(MappingStore::new()), &[], false)
    }

    /// Test consumer that always consumes
    struct AlwaysConsume;

    impl KeyConsumer for AlwaysConsume {
        fn name(&self) -> &'static str {
            "AlwaysConsume"
        }

        fn is_applicable(&self, _ctx: &KeyContext) -> bool {
            true
        }

        fn consume(&self, ctx: &mut KeyContext) -> ConsumeResult {
            ctx.mark_handled();
            ConsumeResult::Consumed
        }
    }

    /// Test consumer that never consumes
    struct NeverConsume;

    impl KeyConsumer for NeverConsume {
        fn name(&self) -> &'static str {
            "NeverConsume"
        }

        fn is_applicable(&self, _ctx: &KeyContext) -> bool {
            true
        }

        fn consume(&self, _ctx: &mut KeyContext) -> ConsumeResult {
            ConsumeResult::NotConsumed
        }
    }

    #[test]
    fn test_empty_pipeline() {
        let pipeline = KeyConsumerPipeline::new();
        assert_eq!(pipeline.len(), 0);

        let vim_state = VimState::default();
        let key = VimKey::new(KeyCode::Char('j'), VimModifiers::NONE);
        let mut ctx = test_context(key, &vim_state);

        let result = pipeline.process(&mut ctx);
        assert_eq!(result, ConsumeResult::NotConsumed);
    }

    #[test]
    fn test_consumer_order() {
        let mut pipeline = KeyConsumerPipeline::new();
        pipeline.add(NeverConsume);
        pipeline.add(AlwaysConsume);
        pipeline.add(NeverConsume);

        let vim_state = VimState::default();
        let key = VimKey::new(KeyCode::Char('j'), VimModifiers::NONE);
        let mut ctx = test_context(key, &vim_state);

        let result = pipeline.process(&mut ctx);
        assert_eq!(result, ConsumeResult::Consumed);
        assert!(ctx.input_handled);
    }

    #[test]
    fn test_all_not_consumed() {
        let mut pipeline = KeyConsumerPipeline::new();
        pipeline.add(NeverConsume);
        pipeline.add(NeverConsume);

        let vim_state = VimState::default();
        let key = VimKey::new(KeyCode::Char('j'), VimModifiers::NONE);
        let mut ctx = test_context(key, &vim_state);

        let result = pipeline.process(&mut ctx);
        assert_eq!(result, ConsumeResult::NotConsumed);
    }

    #[test]
    fn test_build_pipeline_has_all_consumers() {
        let pipeline = build_pipeline();
        // Should have 6 consumers: Passthrough, CmdLine, Completion, Mapping, Recording, VimCommand
        assert_eq!(pipeline.len(), 6);
    }
}
