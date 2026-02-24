//! Key consumer trait - defines the contract for key handling stages.

use strum::Display;

/// Result of a `KeyConsumer` processing a key.
#[derive(Debug, Clone, PartialEq, Eq, Display)]
pub enum ConsumeResult {
    /// Key was fully consumed - stop pipeline processing
    Consumed,
    /// Key was not consumed - continue to next consumer
    NotConsumed,
    /// Key should be passed through to Godot - stop pipeline, no input handling
    Passthrough,
}

/// A consumer in the key handling pipeline.
///
/// Each consumer handles a specific concern (e.g., mappings, completion, macros).
/// Consumers are called in order until one returns `Consumed` or `Passthrough`.
pub trait KeyConsumer {
    /// Returns the name of this consumer for debugging.
    fn name(&self) -> &'static str;

    /// Checks if this consumer is applicable to the current context.
    ///
    /// Return `false` to skip this consumer entirely (optimization).
    fn is_applicable(&self, ctx: &super::KeyContext) -> bool;

    /// Attempts to consume the key.
    ///
    /// Called only if `is_applicable` returns `true`.
    fn consume(&self, ctx: &mut super::KeyContext) -> ConsumeResult;
}
