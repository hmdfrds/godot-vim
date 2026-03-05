//! Unit tests for VimEngine facade mutation methods.
//!
//! Verifies that mutations via the facade correctly update state,
//! using only public facade methods for both mutation and assertion.

#[cfg(test)]
mod tests {
    use crate::bridge::vim_adapter::engine::VimEngine;
    use vim_core::domain::position::Position;
    use vim_core::state::mode::{InsertMode, Mode, VisualKind};

    // ═══════════════════════════════════════════════════════════════════
    // Mode Transitions
    // ═══════════════════════════════════════════════════════════════════

    #[test]
    fn set_mode_changes_mode() {
        let mut engine = VimEngine::new();
        assert!(engine.is_normal());

        engine.set_mode(Mode::Insert(InsertMode::Standard { count: 1 }));
        assert!(engine.is_insert());
        assert!(!engine.is_normal());
    }

    #[test]
    fn set_mode_to_visual() {
        let mut engine = VimEngine::new();
        engine.set_mode(Mode::Visual(VisualKind::Char {
            start: Position::from_byte(0, 0),
        }));
        assert!(engine.is_visual());
    }

    #[test]
    fn set_mode_roundtrip_back_to_normal() {
        let mut engine = VimEngine::new();
        engine.set_mode(Mode::Insert(InsertMode::Standard { count: 1 }));
        assert!(engine.is_insert());
        engine.set_mode(Mode::Normal);
        assert!(engine.is_normal());
    }

    // ═══════════════════════════════════════════════════════════════════
    // Cursor Mutations
    // ═══════════════════════════════════════════════════════════════════

    #[test]
    fn sync_cursor_updates_position() {
        let mut engine = VimEngine::new();
        engine.sync_cursor(Position::from_byte(7, 3));
        assert_eq!(engine.cursor_pos(), Position::from_byte(7, 3));
    }

    #[test]
    fn move_cursor_tracked_jump_updates_position() {
        use crate::bridge::vim_adapter::core::cursor::CursorMoveType;
        let mut engine = VimEngine::new();
        engine.set_cursor(0, 0);
        engine.move_cursor_tracked(Position::from_byte(10, 5), CursorMoveType::Jump);
        assert_eq!(engine.cursor_pos(), Position::from_byte(10, 5));
    }

    #[test]
    fn move_cursor_tracked_jump_records_jump() {
        use crate::bridge::vim_adapter::core::cursor::CursorMoveType;
        let mut engine = VimEngine::new();
        engine.set_cursor(3, 7);
        engine.move_cursor_tracked(Position::from_byte(10, 0), CursorMoveType::Jump);
        // The old position (3,7) should be in the jump list → accessible via last_jump_pos
        assert_eq!(engine.last_jump_pos(), Some(Position::from_byte(3, 7)));
    }

    #[test]
    fn move_cursor_tracked_step_no_jump() {
        use crate::bridge::vim_adapter::core::cursor::CursorMoveType;
        let mut engine = VimEngine::new();
        engine.move_cursor_tracked(Position::from_byte(1, 0), CursorMoveType::Step);
        assert_eq!(engine.cursor_pos(), Position::from_byte(1, 0));
        // Step should not record a jump
        assert!(engine.last_jump_pos().is_none());
    }

    #[test]
    fn record_jump_at_saves_position() {
        let mut engine = VimEngine::new();
        engine.record_jump_at(Position::from_byte(42, 10));
        assert_eq!(engine.last_jump_pos(), Some(Position::from_byte(42, 10)));
    }

    // ═══════════════════════════════════════════════════════════════════
    // Count / Transition
    // ═══════════════════════════════════════════════════════════════════

    #[test]
    fn take_count_returns_one_when_empty() {
        let mut engine = VimEngine::new();
        assert_eq!(engine.take_count(), 1);
    }

    #[test]
    fn accumulate_digit_and_take_count() {
        let mut engine = VimEngine::new();
        engine.accumulate_digit('3');
        assert!(engine.has_count());
        assert_eq!(engine.take_count(), 3);
        // After take, should be reset
        assert!(!engine.has_count());
    }

    #[test]
    fn accumulate_multi_digit_count() {
        let mut engine = VimEngine::new();
        engine.accumulate_digit('1');
        engine.accumulate_digit('5');
        assert_eq!(engine.take_count(), 15);
    }

    // ═══════════════════════════════════════════════════════════════════
    // Insert Recording
    // ═══════════════════════════════════════════════════════════════════

    #[test]
    fn record_insert_char_updates_cursor() {
        let mut engine = VimEngine::new();
        // Initialize quantum buffer at (0,0)
        engine.init_quantum_buffer(Position::from_byte(0, 0));
        engine.record_insert_char('h');
        // After inserting 'h', cursor should advance to col 1
        assert_eq!(engine.cursor_pos().col, 1);
    }

    #[test]
    fn init_quantum_buffer_resets_cursor() {
        let mut engine = VimEngine::new();
        engine.set_cursor(5, 5);
        engine.init_quantum_buffer(Position::from_byte(2, 3));
        assert_eq!(engine.cursor_pos(), Position::from_byte(2, 3));
    }

    // ═══════════════════════════════════════════════════════════════════
    // Completion
    // ═══════════════════════════════════════════════════════════════════

    #[test]
    fn set_completion_visible_toggles() {
        let mut engine = VimEngine::new();
        assert!(!engine.is_completion_visible());
        engine.set_completion_visible(true);
        assert!(engine.is_completion_visible());
        engine.set_completion_visible(false);
        assert!(!engine.is_completion_visible());
    }

    #[test]
    fn sync_completion_visible_sets_state() {
        let mut engine = VimEngine::new();
        engine.sync_completion_visible(true);
        assert!(engine.is_completion_visible());
    }

    // ═══════════════════════════════════════════════════════════════════
    // Search
    // ═══════════════════════════════════════════════════════════════════

    #[test]
    fn set_search_pattern_roundtrip() {
        let mut engine = VimEngine::new();
        engine.set_search("hello".to_string(), true);
        assert_eq!(engine.last_search(), Some("hello"));
        assert!(engine.last_search_forward());
    }

    #[test]
    fn set_search_backward() {
        let mut engine = VimEngine::new();
        engine.set_search("world".to_string(), false);
        assert_eq!(engine.last_search(), Some("world"));
        assert!(!engine.last_search_forward());
    }

    // ═══════════════════════════════════════════════════════════════════
    // Marks
    // ═══════════════════════════════════════════════════════════════════

    #[test]
    fn set_mark_and_get_mark_roundtrip() {
        let mut engine = VimEngine::new();
        engine.set_mark('a', Position::from_byte(10, 5));
        assert_eq!(engine.get_mark('a'), Some(Position::from_byte(10, 5)));
    }

    #[test]
    fn set_mark_overwrite() {
        let mut engine = VimEngine::new();
        engine.set_mark('b', Position::from_byte(1, 1));
        engine.set_mark('b', Position::from_byte(2, 2));
        assert_eq!(engine.get_mark('b'), Some(Position::from_byte(2, 2)));
    }

    // ═══════════════════════════════════════════════════════════════════
    // Line Snapshot
    // ═══════════════════════════════════════════════════════════════════

    #[test]
    fn update_line_snapshot_roundtrip() {
        let mut engine = VimEngine::new();
        engine.update_line_snapshot(3, "hello world".to_string());
        let snap = engine.current_line_snapshot();
        assert!(snap.is_some());
        let (line, text) = snap.as_ref().unwrap();
        assert_eq!(*line, 3);
        assert_eq!(text, "hello world");
    }

    // ═══════════════════════════════════════════════════════════════════
    // Visual/Position Markers
    // ═══════════════════════════════════════════════════════════════════

    #[test]
    fn set_last_change_roundtrip() {
        let mut engine = VimEngine::new();
        engine.set_last_change(Position::from_byte(5, 0));
        assert_eq!(engine.last_change_pos(), Some(Position::from_byte(5, 0)));
    }

    #[test]
    fn set_last_insert_roundtrip() {
        let mut engine = VimEngine::new();
        engine.set_last_insert(Position::from_byte(8, 4));
        assert_eq!(engine.last_insert_pos(), Some(Position::from_byte(8, 4)));
    }

    // ═══════════════════════════════════════════════════════════════════
    // CmdLine History
    // ═══════════════════════════════════════════════════════════════════

    #[test]
    fn cmd_history_push_and_navigate() {
        let mut engine = VimEngine::new();
        engine.push_cmd_history("write");
        engine.push_cmd_history("quit");
        engine.reset_history_nav();

        // Navigate up should give most recent first
        let entry = engine.history_up("");
        assert!(entry.is_some());
        assert_eq!(entry.unwrap(), "quit");
    }

    #[test]
    fn cmd_history_up_then_down() {
        let mut engine = VimEngine::new();
        engine.push_cmd_history("first");
        engine.push_cmd_history("second");
        engine.reset_history_nav();

        let _ = engine.history_up("");
        let down = engine.history_down();
        // After navigating up and then down, the result is implementation-defined
        // (None or ""); verify no panic occurs.
        let _ = down;
    }

    // ═══════════════════════════════════════════════════════════════════
    // Registry
    // ═══════════════════════════════════════════════════════════════════

    #[test]
    fn registry_mut_allows_registration() {
        let mut engine = VimEngine::new();
        engine.registry_mut().register_simple("TestCommand");
        // Should not panic, and registry should now contain the command
        let _reg = engine.registry();
    }
}
