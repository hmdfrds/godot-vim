//! Tests for individual key consumers and pipeline integration.
//!
//! Tests the chain-of-responsibility consumers against realistic scenarios.
//! PassthroughConsumer is excluded because it depends on Godot `VimSettings`.

#[cfg(test)]
mod tests {
    use crate::bridge::vim_adapter::key::consumer::{ConsumeResult, KeyConsumer};
    use crate::bridge::vim_adapter::key::consumers::cmdline::CmdLineConsumer;
    use crate::bridge::vim_adapter::key::consumers::completion::CompletionConsumer;
    use crate::bridge::vim_adapter::key::consumers::mapping::MappingConsumer;
    use crate::bridge::vim_adapter::key::consumers::recording::RecordingConsumer;
    use crate::bridge::vim_adapter::key::consumers::vim_command::VimCommandConsumer;
    use crate::bridge::vim_adapter::key::context::KeyContext;
    use crate::bridge::vim_adapter::key::pipeline::KeyConsumerPipeline;
    use crate::bridge::vim_adapter::mapping::MappingStore;
    use std::sync::Arc;
    use vim_core::inputs::mapping::MappingMode;
    use vim_core::inputs::{KeyCode, VimKey, VimModifiers};
    use vim_core::state::mode::{CmdType, InsertMode, Mode, VisualKind};
    use vim_core::state::VimState;

    // ═══════════════════════════════════════════════════════════════════
    // Helpers
    // ═══════════════════════════════════════════════════════════════════

    fn key(c: char) -> VimKey {
        VimKey::new(KeyCode::Char(c), VimModifiers::NONE)
    }

    fn key_code(code: KeyCode) -> VimKey {
        VimKey::new(code, VimModifiers::NONE)
    }

    fn state_with_mode(mode: Mode) -> VimState {
        let mut state = VimState::default();
        state.set_mode(mode);
        state
    }

    fn ctx<'a>(vim_key: VimKey, state: &'a VimState, store: &Arc<MappingStore>) -> KeyContext<'a> {
        KeyContext::new(vim_key, state, store.clone(), &[], false)
    }

    fn ctx_with_completion<'a>(
        vim_key: VimKey,
        state: &'a VimState,
        store: &Arc<MappingStore>,
    ) -> KeyContext<'a> {
        KeyContext::new(vim_key, state, store.clone(), &[], true)
    }

    fn ctx_with_pending<'a>(
        vim_key: VimKey,
        state: &'a VimState,
        store: &Arc<MappingStore>,
        pending: &[VimKey],
    ) -> KeyContext<'a> {
        KeyContext::new(vim_key, state, store.clone(), pending, false)
    }

    fn empty_store() -> Arc<MappingStore> {
        Arc::new(MappingStore::default())
    }

    fn store_with_mapping(from: &str, to: &str, mode: MappingMode) -> Arc<MappingStore> {
        if let Some(mapping) = MappingStore::parse_mapping(from, to, mode) {
            Arc::new(MappingStore::from_mappings(vec![mapping]))
        } else {
            empty_store()
        }
    }

    // ═══════════════════════════════════════════════════════════════════
    // CmdLineConsumer Tests
    // ═══════════════════════════════════════════════════════════════════

    #[test]
    fn cmdline_not_applicable_in_normal_mode() {
        let consumer = CmdLineConsumer;
        let state = state_with_mode(Mode::Normal);
        let store = empty_store();
        let ctx = ctx(key('j'), &state, &store);
        assert!(!consumer.is_applicable(&ctx));
    }

    #[test]
    fn cmdline_applicable_in_cmdline_mode() {
        let consumer = CmdLineConsumer;
        let state = state_with_mode(Mode::CmdLine(CmdType::Ex));
        let store = empty_store();
        let ctx = ctx(key('j'), &state, &store);
        assert!(consumer.is_applicable(&ctx));
    }

    #[test]
    fn cmdline_esc_consumed() {
        let consumer = CmdLineConsumer;
        let state = state_with_mode(Mode::CmdLine(CmdType::Ex));
        let store = empty_store();
        let mut ctx = ctx(key_code(KeyCode::Esc), &state, &store);
        assert_eq!(consumer.consume(&mut ctx), ConsumeResult::Consumed);
        assert!(ctx.input_handled);
    }

    #[test]
    fn cmdline_regular_key_passthrough() {
        let consumer = CmdLineConsumer;
        let state = state_with_mode(Mode::CmdLine(CmdType::Ex));
        let store = empty_store();
        let mut ctx = ctx(key('w'), &state, &store);
        assert_eq!(consumer.consume(&mut ctx), ConsumeResult::Passthrough);
    }

    #[test]
    fn cmdline_search_forward_mode() {
        let consumer = CmdLineConsumer;
        let state = state_with_mode(Mode::CmdLine(CmdType::SearchForward));
        let store = empty_store();
        let ctx = ctx(key('/'), &state, &store);
        assert!(consumer.is_applicable(&ctx));
    }

    // ═══════════════════════════════════════════════════════════════════
    // CompletionConsumer Tests
    // ═══════════════════════════════════════════════════════════════════

    #[test]
    fn completion_not_applicable_when_inactive() {
        let consumer = CompletionConsumer;
        let state = state_with_mode(Mode::Insert(InsertMode::Standard { count: 1 }));
        let store = empty_store();
        let ctx = ctx(key('j'), &state, &store); // completion_active = false
        assert!(!consumer.is_applicable(&ctx));
    }

    #[test]
    fn completion_applicable_when_active() {
        let consumer = CompletionConsumer;
        let state = state_with_mode(Mode::Insert(InsertMode::Standard { count: 1 }));
        let store = empty_store();
        let ctx = ctx_with_completion(key('j'), &state, &store);
        assert!(consumer.is_applicable(&ctx));
    }

    #[test]
    fn completion_up_passthrough() {
        let consumer = CompletionConsumer;
        let state = state_with_mode(Mode::Insert(InsertMode::Standard { count: 1 }));
        let store = empty_store();
        let mut ctx = ctx_with_completion(key_code(KeyCode::Up), &state, &store);
        assert_eq!(consumer.consume(&mut ctx), ConsumeResult::Passthrough);
    }

    #[test]
    fn completion_down_passthrough() {
        let consumer = CompletionConsumer;
        let state = state_with_mode(Mode::Insert(InsertMode::Standard { count: 1 }));
        let store = empty_store();
        let mut ctx = ctx_with_completion(key_code(KeyCode::Down), &state, &store);
        assert_eq!(consumer.consume(&mut ctx), ConsumeResult::Passthrough);
    }

    #[test]
    fn completion_tab_passthrough() {
        let consumer = CompletionConsumer;
        let state = state_with_mode(Mode::Insert(InsertMode::Standard { count: 1 }));
        let store = empty_store();
        let mut ctx = ctx_with_completion(key_code(KeyCode::Tab), &state, &store);
        assert_eq!(consumer.consume(&mut ctx), ConsumeResult::Passthrough);
    }

    #[test]
    fn completion_esc_consumed() {
        let consumer = CompletionConsumer;
        let state = state_with_mode(Mode::Insert(InsertMode::Standard { count: 1 }));
        let store = empty_store();
        let mut ctx = ctx_with_completion(key_code(KeyCode::Esc), &state, &store);
        assert_eq!(consumer.consume(&mut ctx), ConsumeResult::Consumed);
        assert!(ctx.input_handled);
    }

    #[test]
    fn completion_enter_not_consumed() {
        let consumer = CompletionConsumer;
        let state = state_with_mode(Mode::Insert(InsertMode::Standard { count: 1 }));
        let store = empty_store();
        let mut ctx = ctx_with_completion(key_code(KeyCode::Enter), &state, &store);
        assert_eq!(consumer.consume(&mut ctx), ConsumeResult::NotConsumed);
    }

    #[test]
    fn completion_regular_char_not_consumed() {
        let consumer = CompletionConsumer;
        let state = state_with_mode(Mode::Insert(InsertMode::Standard { count: 1 }));
        let store = empty_store();
        let mut ctx = ctx_with_completion(key('a'), &state, &store);
        assert_eq!(consumer.consume(&mut ctx), ConsumeResult::NotConsumed);
    }

    // ═══════════════════════════════════════════════════════════════════
    // RecordingConsumer Tests
    // ═══════════════════════════════════════════════════════════════════

    #[test]
    fn recording_not_applicable_when_not_recording() {
        let consumer = RecordingConsumer;
        let state = state_with_mode(Mode::Normal);
        let store = empty_store();
        let ctx = ctx(key('q'), &state, &store);
        assert!(!consumer.is_applicable(&ctx));
    }

    #[test]
    fn recording_applicable_when_recording() {
        let consumer = RecordingConsumer;
        let mut state = state_with_mode(Mode::Normal);
        state.macros.recording = Some('a');
        let store = empty_store();
        let ctx = ctx(key('q'), &state, &store);
        assert!(consumer.is_applicable(&ctx));
    }

    #[test]
    fn recording_q_stops_recording() {
        let consumer = RecordingConsumer;
        let mut state = state_with_mode(Mode::Normal);
        state.macros.recording = Some('a');
        let store = empty_store();
        let mut ctx = ctx(key('q'), &state, &store);
        assert_eq!(consumer.consume(&mut ctx), ConsumeResult::Consumed);
        assert!(ctx.input_handled);
    }

    #[test]
    fn recording_other_key_passes_through() {
        let consumer = RecordingConsumer;
        let mut state = state_with_mode(Mode::Normal);
        state.macros.recording = Some('a');
        let store = empty_store();
        let mut ctx = ctx(key('j'), &state, &store);
        assert_eq!(consumer.consume(&mut ctx), ConsumeResult::NotConsumed);
    }

    // ═══════════════════════════════════════════════════════════════════
    // MappingConsumer Tests
    // ═══════════════════════════════════════════════════════════════════

    #[test]
    fn mapping_no_match_empty_store() {
        let consumer = MappingConsumer;
        let state = state_with_mode(Mode::Normal);
        let store = empty_store();
        let mut ctx = ctx(key('j'), &state, &store);
        assert_eq!(consumer.consume(&mut ctx), ConsumeResult::NotConsumed);
    }

    #[test]
    fn mapping_exact_match() {
        let consumer = MappingConsumer;
        let state = state_with_mode(Mode::Insert(InsertMode::Standard { count: 1 }));
        let store = store_with_mapping("jk", "<Esc>", MappingMode::Insert);
        // Simulate 'j' already pending, now pressing 'k'
        let pending = vec![key('j')];
        let mut ctx = ctx_with_pending(key('k'), &state, &store, &pending);
        assert_eq!(consumer.consume(&mut ctx), ConsumeResult::Consumed);
        assert!(ctx.input_handled);
    }

    #[test]
    fn mapping_prefix_match_waits() {
        let consumer = MappingConsumer;
        let state = state_with_mode(Mode::Insert(InsertMode::Standard { count: 1 }));
        let store = store_with_mapping("jk", "<Esc>", MappingMode::Insert);
        // Pressing 'j' — prefix of 'jk'
        let mut ctx = ctx(key('j'), &state, &store);
        assert_eq!(consumer.consume(&mut ctx), ConsumeResult::Consumed);
        assert!(ctx.input_handled); // Consumed because it's a prefix
    }

    #[test]
    fn mapping_wrong_mode_no_match() {
        let consumer = MappingConsumer;
        let state = state_with_mode(Mode::Normal); // Mapping is for Insert mode
        let store = store_with_mapping("jk", "<Esc>", MappingMode::Insert);
        let mut ctx = ctx(key('j'), &state, &store);
        assert_eq!(consumer.consume(&mut ctx), ConsumeResult::NotConsumed);
    }

    #[test]
    fn mapping_normal_mode_single_key() {
        let consumer = MappingConsumer;
        let state = state_with_mode(Mode::Normal);
        // Single-key mapping: 'H' -> '0' in Normal
        let store = store_with_mapping("H", "0", MappingMode::Normal);
        let mut ctx = ctx(key('H'), &state, &store);
        assert_eq!(consumer.consume(&mut ctx), ConsumeResult::Consumed);
        assert!(ctx.input_handled);
    }

    #[test]
    fn mapping_cmdline_mode_not_supported() {
        let consumer = MappingConsumer;
        let state = state_with_mode(Mode::CmdLine(CmdType::Ex));
        let store = store_with_mapping("jk", "<Esc>", MappingMode::Insert);
        let mut ctx = ctx(key('j'), &state, &store);
        assert_eq!(consumer.consume(&mut ctx), ConsumeResult::NotConsumed);
    }

    // ═══════════════════════════════════════════════════════════════════
    // VimCommandConsumer Tests
    // ═══════════════════════════════════════════════════════════════════

    #[test]
    fn vim_command_always_applicable() {
        let consumer = VimCommandConsumer;
        let state = state_with_mode(Mode::Normal);
        let store = empty_store();
        let ctx = ctx(key('j'), &state, &store);
        assert!(consumer.is_applicable(&ctx));
    }

    #[test]
    fn vim_command_esc_in_normal_consumed() {
        let consumer = VimCommandConsumer;
        let state = state_with_mode(Mode::Normal);
        let store = empty_store();
        let mut ctx = ctx(key_code(KeyCode::Esc), &state, &store);
        assert_eq!(consumer.consume(&mut ctx), ConsumeResult::Consumed);
    }

    #[test]
    fn vim_command_regular_key_consumed() {
        let consumer = VimCommandConsumer;
        let state = state_with_mode(Mode::Normal);
        let store = empty_store();
        let mut ctx = ctx(key('d'), &state, &store);
        assert_eq!(consumer.consume(&mut ctx), ConsumeResult::Consumed);
        assert!(ctx.input_handled);
    }

    #[test]
    fn vim_command_in_insert_mode_consumed() {
        let consumer = VimCommandConsumer;
        let state = state_with_mode(Mode::Insert(InsertMode::Standard { count: 1 }));
        let store = empty_store();
        let mut ctx = ctx(key('a'), &state, &store);
        assert_eq!(consumer.consume(&mut ctx), ConsumeResult::Consumed);
    }

    // ═══════════════════════════════════════════════════════════════════
    // KeyContext Tests
    // ═══════════════════════════════════════════════════════════════════

    #[test]
    fn context_mode_detection_normal() {
        let state = state_with_mode(Mode::Normal);
        let store = empty_store();
        let ctx = ctx(key('j'), &state, &store);
        assert!(ctx.is_normal_mode());
        assert!(!ctx.is_insert_mode());
        assert!(!ctx.is_visual_mode());
        assert!(!ctx.is_cmdline_mode());
    }

    #[test]
    fn context_mode_detection_insert() {
        let state = state_with_mode(Mode::Insert(InsertMode::Standard { count: 1 }));
        let store = empty_store();
        let ctx = ctx(key('j'), &state, &store);
        assert!(ctx.is_insert_mode());
        assert!(!ctx.is_normal_mode());
    }

    #[test]
    fn context_mode_detection_visual() {
        let state = state_with_mode(Mode::Visual(VisualKind::Char {
            start: Default::default(),
        }));
        let store = empty_store();
        let ctx = ctx(key('j'), &state, &store);
        assert!(ctx.is_visual_mode());
        assert!(!ctx.is_normal_mode());
    }

    #[test]
    fn context_mode_detection_cmdline() {
        let state = state_with_mode(Mode::CmdLine(CmdType::Ex));
        let store = empty_store();
        let ctx = ctx(key('j'), &state, &store);
        assert!(ctx.is_cmdline_mode());
        assert!(!ctx.is_normal_mode());
    }

    #[test]
    fn context_mapping_mode_normal() {
        let state = state_with_mode(Mode::Normal);
        let store = empty_store();
        let ctx = ctx(key('j'), &state, &store);
        assert_eq!(ctx.mapping_mode(), Some(MappingMode::Normal));
    }

    #[test]
    fn context_mapping_mode_insert() {
        let state = state_with_mode(Mode::Insert(InsertMode::Standard { count: 1 }));
        let store = empty_store();
        let ctx = ctx(key('j'), &state, &store);
        assert_eq!(ctx.mapping_mode(), Some(MappingMode::Insert));
    }

    #[test]
    fn context_mapping_mode_visual() {
        let state = state_with_mode(Mode::Visual(VisualKind::Char {
            start: Default::default(),
        }));
        let store = empty_store();
        let ctx = ctx(key('j'), &state, &store);
        assert_eq!(ctx.mapping_mode(), Some(MappingMode::Visual));
    }

    #[test]
    fn context_mapping_mode_cmdline_none() {
        let state = state_with_mode(Mode::CmdLine(CmdType::Ex));
        let store = empty_store();
        let ctx = ctx(key('j'), &state, &store);
        assert_eq!(ctx.mapping_mode(), None);
    }

    #[test]
    fn context_could_start_mapping_true() {
        let state = state_with_mode(Mode::Insert(InsertMode::Standard { count: 1 }));
        let store = store_with_mapping("jk", "<Esc>", MappingMode::Insert);
        let ctx = ctx(key('j'), &state, &store);
        assert!(ctx.could_start_mapping());
    }

    #[test]
    fn context_could_start_mapping_false() {
        let state = state_with_mode(Mode::Insert(InsertMode::Standard { count: 1 }));
        let store = store_with_mapping("jk", "<Esc>", MappingMode::Insert);
        let ctx = ctx(key('x'), &state, &store); // 'x' doesn't start any mapping
        assert!(!ctx.could_start_mapping());
    }

    #[test]
    fn context_mark_handled() {
        let state = state_with_mode(Mode::Normal);
        let store = empty_store();
        let mut ctx = ctx(key('j'), &state, &store);
        assert!(!ctx.input_handled);
        ctx.mark_handled();
        assert!(ctx.input_handled);
    }

    #[test]
    fn context_trace_messages() {
        let state = state_with_mode(Mode::Normal);
        let store = empty_store();
        let mut ctx = ctx(key('j'), &state, &store);
        ctx.trace("hello");
        ctx.trace("world");
        assert_eq!(ctx.trace_messages.len(), 2);
        assert_eq!(ctx.trace_messages[0], "hello");
    }

    // ═══════════════════════════════════════════════════════════════════
    // Pipeline Integration Tests
    // ═══════════════════════════════════════════════════════════════════

    /// Builds a pipeline without PassthroughConsumer (Godot-coupled).
    fn testable_pipeline() -> KeyConsumerPipeline {
        let mut pipeline = KeyConsumerPipeline::new();
        // Order matches build_pipeline() minus PassthroughConsumer
        pipeline.add(CmdLineConsumer);
        pipeline.add(CompletionConsumer);
        pipeline.add(MappingConsumer);
        pipeline.add(RecordingConsumer);
        pipeline.add(VimCommandConsumer);
        pipeline
    }

    #[test]
    fn pipeline_cmdline_esc_stops_early() {
        let pipeline = testable_pipeline();
        let state = state_with_mode(Mode::CmdLine(CmdType::Ex));
        let store = empty_store();
        let mut ctx = ctx(key_code(KeyCode::Esc), &state, &store);
        let result = pipeline.process(&mut ctx);
        assert_eq!(result, ConsumeResult::Consumed);
        assert!(ctx.input_handled);
        // CmdLineConsumer should handle it — VimCommand shouldn't be reached
    }

    #[test]
    fn pipeline_cmdline_char_passthrough() {
        let pipeline = testable_pipeline();
        let state = state_with_mode(Mode::CmdLine(CmdType::Ex));
        let store = empty_store();
        let mut ctx = ctx(key('w'), &state, &store);
        let result = pipeline.process(&mut ctx);
        assert_eq!(result, ConsumeResult::Passthrough);
    }

    #[test]
    fn pipeline_normal_mode_key_consumed_by_vim_command() {
        let pipeline = testable_pipeline();
        let state = state_with_mode(Mode::Normal);
        let store = empty_store();
        let mut ctx = ctx(key('d'), &state, &store);
        let result = pipeline.process(&mut ctx);
        assert_eq!(result, ConsumeResult::Consumed);
        assert!(ctx.input_handled);
    }

    #[test]
    fn pipeline_completion_active_up_passthrough() {
        let pipeline = testable_pipeline();
        let state = state_with_mode(Mode::Insert(InsertMode::Standard { count: 1 }));
        let store = empty_store();
        let mut ctx = ctx_with_completion(key_code(KeyCode::Up), &state, &store);
        let result = pipeline.process(&mut ctx);
        assert_eq!(result, ConsumeResult::Passthrough);
    }

    #[test]
    fn pipeline_mapping_takes_priority_over_vim_command() {
        let pipeline = testable_pipeline();
        let state = state_with_mode(Mode::Insert(InsertMode::Standard { count: 1 }));
        let store = store_with_mapping("jk", "<Esc>", MappingMode::Insert);
        // Press 'j' — starts a mapping prefix
        let mut ctx = ctx(key('j'), &state, &store);
        let result = pipeline.process(&mut ctx);
        // MappingConsumer should consume (prefix match), not VimCommand
        assert_eq!(result, ConsumeResult::Consumed);
    }

    #[test]
    fn pipeline_recording_q_consumed_before_vim_command() {
        let pipeline = testable_pipeline();
        let mut state = state_with_mode(Mode::Normal);
        state.macros.recording = Some('a');
        let store = empty_store();
        let mut ctx = ctx(key('q'), &state, &store);
        let result = pipeline.process(&mut ctx);
        // RecordingConsumer is before VimCommand, should handle 'q'
        assert_eq!(result, ConsumeResult::Consumed);
        assert!(ctx.input_handled);
    }

    #[test]
    fn pipeline_no_false_mapping_in_wrong_mode() {
        let pipeline = testable_pipeline();
        let state = state_with_mode(Mode::Normal); // Mapping is for Insert
        let store = store_with_mapping("jk", "<Esc>", MappingMode::Insert);
        let mut ctx = ctx(key('j'), &state, &store);
        let result = pipeline.process(&mut ctx);
        // No mapping matches in this mode; VimCommand consumes the key instead.
        assert_eq!(result, ConsumeResult::Consumed);
    }
}
