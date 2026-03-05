//! Integration tests for VimEngine facade using TestHarness.
//!
//! Tests end-to-end workflows: mode round-trips, facade+harness interaction,
//! and composite operations.

#[cfg(test)]
mod tests {
    use crate::bridge::vim_adapter::engine::VimEngine;
    use crate::bridge::vim_adapter::mock::TestHarness;
    use vim_core::domain::column::ByteCol;
    use vim_core::domain::position::Position;
    use vim_core::state::mode::{InsertMode, Mode, VisualKind};

    // ═══════════════════════════════════════════════════════════════════
    // Mode Round-Trips
    // ═══════════════════════════════════════════════════════════════════

    #[test]
    fn facade_normal_insert_normal_roundtrip() {
        let mut engine = VimEngine::new();
        assert!(engine.is_normal());

        engine.set_mode(Mode::Insert(InsertMode::Standard { count: 1 }));
        assert!(engine.is_insert());
        assert!(!engine.is_normal());
        assert!(!engine.is_visual());

        engine.set_mode(Mode::Normal);
        assert!(engine.is_normal());
        assert!(!engine.is_insert());
    }

    #[test]
    fn facade_visual_modes_exclusive() {
        let mut engine = VimEngine::new();

        engine.set_mode(Mode::Visual(VisualKind::Char {
            start: Position::from_byte(0, 0),
        }));
        assert!(engine.is_visual());
        assert!(!engine.is_visual_block());

        engine.set_mode(Mode::Visual(VisualKind::Block {
            start: Position::from_byte(0, 0),
            cursor: Position::from_byte(0, 0),
        }));
        assert!(engine.is_visual());
        assert!(engine.is_visual_block());

        engine.set_mode(Mode::Normal);
        assert!(!engine.is_visual());
        assert!(!engine.is_visual_block());
    }

    // ═══════════════════════════════════════════════════════════════════
    // Cursor + Position Tracking
    // ═══════════════════════════════════════════════════════════════════

    #[test]
    fn facade_cursor_movement_preserves_state() {
        let mut engine = VimEngine::new();
        engine.sync_cursor(Position::from_byte(0, 0));
        assert_eq!(engine.cursor_pos(), Position::from_byte(0, 0));

        engine.sync_cursor(Position::from_byte(5, 10));
        assert_eq!(engine.cursor_pos(), Position::from_byte(5, 10));

        engine.set_preferred_column(ByteCol::new(10));
        assert_eq!(engine.preferred_column(), Some(ByteCol::new(10)));

        // sync_cursor should also update
        engine.sync_cursor(Position::from_byte(3, 7));
        assert_eq!(engine.cursor_pos(), Position::from_byte(3, 7));
    }

    #[test]
    fn facade_jump_tracking_chain() {
        use crate::bridge::vim_adapter::core::cursor::CursorMoveType;
        let mut engine = VimEngine::new();

        // Start at (0,0) → jump to (10,0) → jump to (20,0)
        engine.sync_cursor(Position::from_byte(0, 0));
        engine.move_cursor_tracked(Position::from_byte(10, 0), CursorMoveType::Jump);
        assert_eq!(engine.last_jump_pos(), Some(Position::from_byte(0, 0)));

        engine.move_cursor_tracked(Position::from_byte(20, 0), CursorMoveType::Jump);
        assert_eq!(engine.last_jump_pos(), Some(Position::from_byte(10, 0)));
        assert_eq!(engine.cursor_pos(), Position::from_byte(20, 0));
    }

    // ═══════════════════════════════════════════════════════════════════
    // Search Pattern Persistence
    // ═══════════════════════════════════════════════════════════════════

    #[test]
    fn facade_search_pattern_persistence() {
        let mut engine = VimEngine::new();
        assert!(engine.last_search().is_none());

        engine.set_search("test_pattern".to_string(), true);
        assert_eq!(engine.last_search(), Some("test_pattern"));
        assert!(engine.last_search_forward());

        // Overwrite with backward search
        engine.set_search("other".to_string(), false);
        assert_eq!(engine.last_search(), Some("other"));
        assert!(!engine.last_search_forward());
    }

    // ═══════════════════════════════════════════════════════════════════
    // Count Prefix Accumulation
    // ═══════════════════════════════════════════════════════════════════

    #[test]
    fn facade_count_prefix_accumulation() {
        let mut engine = VimEngine::new();
        assert!(!engine.has_count());

        engine.accumulate_digit('5');
        assert!(engine.has_count());

        engine.accumulate_digit('2');
        let count = engine.take_count();
        assert_eq!(count, 52);

        // After take, reset
        assert!(!engine.has_count());
        assert_eq!(engine.take_count(), 1); // default is 1
    }

    // ═══════════════════════════════════════════════════════════════════
    // Insert Recording Session
    // ═══════════════════════════════════════════════════════════════════

    #[test]
    fn facade_insert_recording_full_session() {
        let mut engine = VimEngine::new();

        // Enter insert mode
        engine.set_mode(Mode::Insert(InsertMode::Standard { count: 1 }));
        assert!(engine.is_insert());

        // Initialize quantum buffer at cursor
        engine.init_quantum_buffer(Position::from_byte(0, 0));
        assert_eq!(engine.cursor_pos(), Position::from_byte(0, 0));

        // Type "Hi"
        engine.record_insert_char('H');
        assert_eq!(engine.cursor_pos().col.as_usize(), 1);
        engine.record_insert_char('i');
        assert_eq!(engine.cursor_pos().col.as_usize(), 2);

        // Exit insert mode
        engine.set_mode(Mode::Normal);
        assert!(engine.is_normal());
    }

    // ═══════════════════════════════════════════════════════════════════
    // Harness Integration
    // ═══════════════════════════════════════════════════════════════════

    #[test]
    fn harness_basic_setup() {
        let harness = TestHarness::new()
            .with_content("hello world\nsecond line")
            .with_cursor(0, 5);

        harness.assert_cursor(0, 5);
        harness.assert_line(0, "hello world");
    }

    #[test]
    fn harness_motion_sequence() {
        let mut harness = TestHarness::new().with_content("line one\nline two\nline three");

        harness.assert_cursor(0, 0);
        harness.move_down(1);
        harness.assert_cursor(1, 0);
        harness.move_down(1);
        harness.assert_cursor(2, 0);
        harness.move_up(2);
        harness.assert_cursor(0, 0);
    }
}
