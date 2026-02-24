#[cfg(test)]
mod tests {
    use crate::bridge::types::cursor::CursorPos;
    use crate::bridge::vim_adapter::contracts::{ExecutionContext, InputPolicy};
    use crate::bridge::vim_adapter::effect_converter;
    use crate::bridge::vim_adapter::engine::VimEngine;
    use vim_core::domain::external_command::ExternalCommand;
    use vim_core::domain::selection::Selection;
    use vim_core::domain::snapshot::TestDoc;
    use vim_core::runtime::EffectAccumulator;
    use vim_core::inputs::commands::Action;
    use vim_core::inputs::{KeyCode, VimKey, VimModifiers};
    use vim_core::protocol::messages::{ProtocolRequest, StableShellRequest};
    use vim_core::state::VimState;

    fn no_caps_context(cursor: CursorPos) -> ExecutionContext<'static> {
        ExecutionContext::with_ports(cursor, None, None, None)
    }

    #[test]
    fn input_policy_parity_for_simple_motion() {
        let mut exclusive_engine = VimEngine::new();
        let mut passive_engine = VimEngine::new();

        let doc = TestDoc::new(vec!["hello world"], Selection::cursor(0, 0));
        let key = VimKey::new(KeyCode::Char('l'), VimModifiers::NONE);
        let cursor = CursorPos::new(0, 0);

        let exclusive = exclusive_engine
            .process_key_with_policy(&key, InputPolicy::Exclusive, &doc, no_caps_context(cursor))
            .expect("exclusive policy should parse motion");
        let passive = passive_engine
            .process_key_with_policy(&key, InputPolicy::Passive, &doc, no_caps_context(cursor))
            .expect("passive policy should parse motion");

        assert_eq!(exclusive.transaction, passive.transaction);
        assert_eq!(exclusive.pending_keys, passive.pending_keys);
        assert_eq!(exclusive.commands, passive.commands);
        assert_eq!(exclusive_engine.cursor_pos(), passive_engine.cursor_pos());
        assert_eq!(exclusive_engine.mode(), passive_engine.mode());
    }

    #[test]
    fn effect_conversion_order_is_deterministic() {
        fn build_effects() -> EffectAccumulator {
            let mut effects = EffectAccumulator::new();
            effects.add_request(ProtocolRequest::ShellOp(StableShellRequest::Undo {
                count: 1,
            }));
            effects.add_request(ProtocolRequest::ClipboardSet("clip".to_string()));
            effects.add_request(ProtocolRequest::ShowMessage {
                level: vim_core::protocol::messages::MessageLevel::Info,
                text: "msg".to_string(),
            });
            effects
        }

        let state = VimState::new();
        let prev_mode = state.mode();

        let mut effects_a = build_effects();
        let mut effects_b = build_effects();

        let output_a = effect_converter::effects_to_output(&mut effects_a, &prev_mode, &state);
        let output_b = effect_converter::effects_to_output(&mut effects_b, &prev_mode, &state);

        assert_eq!(output_a.commands, output_b.commands);
        assert_eq!(output_a.transaction, output_b.transaction);
        assert_eq!(output_a.pending_keys, output_b.pending_keys);
    }

    #[test]
    fn ex_command_runtime_path_is_deterministic() {
        let mut engine_a = VimEngine::new();
        let mut engine_b = VimEngine::new();

        let doc = TestDoc::new(vec!["hello world"], Selection::cursor(0, 0));
        let cursor = CursorPos::new(0, 0);
        let context = no_caps_context(cursor);

        let output_a = engine_a.process_ex_command_with_context("w", &doc, context);
        let output_b = engine_b.process_ex_command_with_context("w", &doc, context);

        assert_eq!(output_a.transaction, output_b.transaction);
        assert_eq!(output_a.commands, output_b.commands);
        assert_eq!(output_a.pending_keys, output_b.pending_keys);
    }

    #[test]
    fn ex_command_string_path_matches_direct_action_path() {
        let mut command_engine = VimEngine::new();
        let mut action_engine = VimEngine::new();

        let doc = TestDoc::new(vec!["hello world"], Selection::cursor(0, 0));
        let cursor = CursorPos::new(0, 0);
        let context = no_caps_context(cursor);

        let from_command = command_engine.process_ex_command_with_context("w", &doc, context);
        let from_action = action_engine.process_action_with_context(
            Action::ExtCmd(ExternalCommand::Save),
            &doc,
            context,
        );

        assert_eq!(from_command.transaction, from_action.transaction);
        assert_eq!(from_command.commands, from_action.commands);
        assert_eq!(from_command.pending_keys, from_action.pending_keys);
    }
}
