//! Unit tests for VimEngine facade read-only accessors.
//!
//! Verifies that each accessor correctly exposes the underlying state
//! via the facade API (no direct VimState access).

#[cfg(test)]
mod tests {
    use crate::bridge::vim_adapter::engine::VimEngine;
    use vim_core::domain::column::ByteCol;
    use vim_core::domain::position::Position;
    use vim_core::state::mode::Mode;

    // ═══════════════════════════════════════════════════════════════════
    // Mode Accessors
    // ═══════════════════════════════════════════════════════════════════

    #[test]
    fn mode_returns_normal_by_default() {
        let engine = VimEngine::new();
        assert!(matches!(engine.mode(), Mode::Normal));
    }

    #[test]
    fn is_normal_true_by_default() {
        let engine = VimEngine::new();
        assert!(engine.is_normal());
    }

    #[test]
    fn is_visual_false_in_normal() {
        let engine = VimEngine::new();
        assert!(!engine.is_visual());
    }

    #[test]
    fn is_insert_false_in_normal() {
        let engine = VimEngine::new();
        assert!(!engine.is_insert());
    }

    #[test]
    fn is_replace_false_in_normal() {
        let engine = VimEngine::new();
        assert!(!engine.is_replace());
    }

    #[test]
    fn is_cmdline_false_in_normal() {
        let engine = VimEngine::new();
        assert!(!engine.is_cmdline());
    }

    #[test]
    fn is_visual_block_false_in_normal() {
        let engine = VimEngine::new();
        assert!(!engine.is_visual_block());
    }

    // ═══════════════════════════════════════════════════════════════════
    // Cursor / Position
    // ═══════════════════════════════════════════════════════════════════

    #[test]
    fn cursor_pos_default_is_origin() {
        let engine = VimEngine::new();
        let pos = engine.cursor_pos();
        assert_eq!(pos.line, 0);
        assert_eq!(pos.col.as_usize(), 0);
    }

    #[test]
    fn set_cursor_updates_position() {
        let mut engine = VimEngine::new();
        engine.sync_cursor(Position::from_byte(5, 10));
        let pos = engine.cursor_pos();
        assert_eq!(pos.line, 5);
        assert_eq!(pos.col, ByteCol::new(10));
    }

    #[test]
    fn preferred_column_none_by_default() {
        let engine = VimEngine::new();
        assert_eq!(engine.preferred_column(), None);
    }

    #[test]
    fn set_preferred_column_roundtrip() {
        let mut engine = VimEngine::new();
        engine.set_preferred_column(ByteCol::new(42));
        assert_eq!(engine.preferred_column(), Some(ByteCol::new(42)));
    }

    // ═══════════════════════════════════════════════════════════════════
    // Macros
    // ═══════════════════════════════════════════════════════════════════

    #[test]
    fn is_recording_false_by_default() {
        let engine = VimEngine::new();
        assert!(!engine.is_recording());
    }

    #[test]
    fn recording_register_none_by_default() {
        let engine = VimEngine::new();
        assert_eq!(engine.recording_register(), None);
    }

    // ═══════════════════════════════════════════════════════════════════
    // Search
    // ═══════════════════════════════════════════════════════════════════

    #[test]
    fn last_search_none_by_default() {
        let engine = VimEngine::new();
        assert_eq!(engine.last_search(), None);
    }

    #[test]
    fn last_search_forward_false_by_default() {
        let engine = VimEngine::new();
        // Default search direction is unset (false)
        assert!(!engine.last_search_forward());
    }

    #[test]
    fn last_substitute_none_by_default() {
        let engine = VimEngine::new();
        let (pat, repl, flags) = engine.last_substitute();
        assert!(pat.is_none());
        assert!(repl.is_none());
        assert!(flags.is_none());
    }

    // ═══════════════════════════════════════════════════════════════════
    // Registers
    // ═══════════════════════════════════════════════════════════════════

    #[test]
    fn register_get_none_for_empty() {
        let engine = VimEngine::new();
        assert!(engine.register_get('a').is_none());
    }

    #[test]
    fn register_entries_empty_by_default() {
        let engine = VimEngine::new();
        assert_eq!(engine.register_entries().count(), 0);
    }

    // ═══════════════════════════════════════════════════════════════════
    // Insert / Completion
    // ═══════════════════════════════════════════════════════════════════

    #[test]
    fn is_completion_visible_false_by_default() {
        let engine = VimEngine::new();
        assert!(!engine.is_completion_visible());
    }

    // ═══════════════════════════════════════════════════════════════════
    // Line Snapshot
    // ═══════════════════════════════════════════════════════════════════

    #[test]
    fn current_line_snapshot_none_by_default() {
        let engine = VimEngine::new();
        assert!(engine.current_line_snapshot().is_none());
    }

    // ═══════════════════════════════════════════════════════════════════
    // Count
    // ═══════════════════════════════════════════════════════════════════

    #[test]
    fn has_count_false_by_default() {
        let engine = VimEngine::new();
        assert!(!engine.has_count());
    }

    // ═══════════════════════════════════════════════════════════════════
    // Visual/Position Markers
    // ═══════════════════════════════════════════════════════════════════

    #[test]
    fn last_insert_pos_none_by_default() {
        let engine = VimEngine::new();
        assert!(engine.last_insert_pos().is_none());
    }

    #[test]
    fn last_change_pos_none_by_default() {
        let engine = VimEngine::new();
        assert!(engine.last_change_pos().is_none());
    }

    #[test]
    fn last_jump_pos_none_by_default() {
        let engine = VimEngine::new();
        assert!(engine.last_jump_pos().is_none());
    }

    #[test]
    fn last_visual_selection_none_by_default() {
        let engine = VimEngine::new();
        assert!(engine.last_visual_selection().is_none());
    }

    // ═══════════════════════════════════════════════════════════════════
    // Marks
    // ═══════════════════════════════════════════════════════════════════

    #[test]
    fn get_mark_none_by_default() {
        let engine = VimEngine::new();
        assert!(engine.get_mark('a').is_none());
    }

    // ═══════════════════════════════════════════════════════════════════
    // Registry
    // ═══════════════════════════════════════════════════════════════════

    #[test]
    fn registry_exists() {
        let engine = VimEngine::new();
        // Registry should be accessible (non-panicking)
        let _reg = engine.registry();
    }

    #[test]
    fn last_change_none_by_default() {
        let engine = VimEngine::new();
        assert!(engine.last_change().is_none());
    }

    // ═══════════════════════════════════════════════════════════════════
    // Default trait
    // ═══════════════════════════════════════════════════════════════════

    #[test]
    fn default_matches_new() {
        let from_new = VimEngine::new();
        let from_default = VimEngine::default();
        // Both should start in Normal mode at (0,0)
        assert_eq!(from_new.mode(), from_default.mode());
        assert_eq!(from_new.cursor_pos(), from_default.cursor_pos());
    }
}
