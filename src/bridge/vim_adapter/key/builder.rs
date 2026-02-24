//! Pipeline builder - constructs the consumer pipeline with appropriate consumers.

use super::consumers::{
    CmdLineConsumer, CompletionConsumer, MappingConsumer, PassthroughConsumer, RecordingConsumer,
    VimCommandConsumer,
};
use super::pipeline::KeyConsumerPipeline;

/// Builds a key consumer pipeline with all standard consumers.
///
/// This function creates the standard consumer chain in the correct order:
/// 1. Passthrough - check for bypass keys
/// 2. `CmdLine` - handle command-line mode
/// 3. Completion - handle code completion popup
/// 4. Mapping - check custom key mappings
/// 5. Recording - handle macro recording
/// 6. `VimCommand` - parse and execute Vim commands
///
/// All consumers are stateless - they use `KeyContext` for state access.
#[must_use]
pub fn build_pipeline() -> KeyConsumerPipeline {
    let mut pipeline = KeyConsumerPipeline::new();

    // 1. Passthrough (always active)
    pipeline.add(PassthroughConsumer);

    // 2. CmdLine (mode-dependent, handled by is_applicable)
    pipeline.add(CmdLineConsumer);

    // 3. Completion (only if popup active, checked via context)
    pipeline.add(CompletionConsumer);

    // 4. Mapping (always checked)
    pipeline.add(MappingConsumer);

    // 5. Recording (only if recording, checked via context)
    pipeline.add(RecordingConsumer);

    // 6. VimCommand (final fallback)
    pipeline.add(VimCommandConsumer);

    pipeline
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_pipeline() {
        let pipeline = build_pipeline();
        assert_eq!(pipeline.len(), 6);
    }
}
