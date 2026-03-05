//! Mapping consumer - handles custom key mappings.
//!
//! Checks if the key (combined with pending keys) matches a custom mapping.
//! Uses the KeyTrie-based `MappingStore` for O(k) lookups via `KeyContext`.

use crate::bridge::vim_adapter::key::consumer::{ConsumeResult, KeyConsumer};
use crate::bridge::vim_adapter::key::context::KeyContext;
use vim_core::inputs::mapping::MappingLookup;

/// Consumer for custom key mappings.
///
/// This consumer:
/// 1. Checks if current key could start/continue a mapping
/// 2. Returns Consumed if a complete mapping is found
/// 3. Returns Consumed if waiting for more keys (pending state)
/// 4. Returns `NotConsumed` if no mapping possible
///
/// Mapping state (pending keys, timer) is managed by `VimWrapper`;
/// this consumer provides the lookup logic via `KeyContext`.
pub struct MappingConsumer;

impl KeyConsumer for MappingConsumer {
    fn name(&self) -> &'static str {
        "Mapping"
    }

    fn is_applicable(&self, _ctx: &KeyContext) -> bool {
        true
    }

    fn consume(&self, ctx: &mut KeyContext) -> ConsumeResult {
        // Skip if no pending keys and this key cannot start any mapping.
        if ctx.pending_keys.is_empty() && !ctx.could_start_mapping() {
            return ConsumeResult::NotConsumed;
        }

        let mapping_store = ctx.mapping_store.clone();

        let Some(mode) = ctx.mapping_mode() else {
            return ConsumeResult::NotConsumed;
        };

        // Build full sequence including new key
        let mut full_sequence = ctx.pending_keys.clone();
        full_sequence.push(ctx.key);

        // Lookup the full sequence
        let lookup_result = mapping_store.lookup(&full_sequence, mode);

        // Process the result now that the immutable borrow of ctx is released.
        match lookup_result {
            MappingLookup::Match(mapping) => {
                ctx.mark_handled();
                ctx.trace(format!("Mapping: matched → {:?}", mapping.to));
                ConsumeResult::Consumed
            }
            MappingLookup::MatchAndPrefix(mapping) => {
                ctx.mark_handled();
                ctx.trace(format!("Mapping: matched+prefix → {:?}", mapping.to));
                ConsumeResult::Consumed
            }
            MappingLookup::Prefix => {
                ctx.mark_handled();
                ctx.trace("Mapping: prefix, waiting");
                ConsumeResult::Consumed
            }
            MappingLookup::None => ConsumeResult::NotConsumed,
        }
    }
}
