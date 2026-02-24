//! Test harness for shell handler testing.
//!
//! Provides a complete mock environment with VimState, MockEditor, and assertion helpers.
#![allow(dead_code)]

use super::{MockClipboard, MockEditor};
use vim_core::domain::position::Position;
use vim_core::domain::snapshot::DocumentSnapshot;
use vim_core::state::config::Config;
use vim_core::state::mode::Mode;
use vim_core::state::VimState;

/// Complete test environment for shell handlers.
///
/// Combines MockEditor, VimState, and assertion helpers for ergonomic testing.
pub struct TestHarness {
    /// The mock editor buffer.
    pub editor: MockEditor,
    /// The Vim state machine.
    pub vim_state: VimState,
    /// Mock clipboard.
    pub clipboard: MockClipboard,
    /// Cached configuration.
    pub config: Config,
}

impl TestHarness {
    /// Creates a new test harness with default empty state.
    #[must_use]
    pub fn new() -> Self {
        Self {
            editor: MockEditor::new(),
            vim_state: VimState::new(),
            clipboard: MockClipboard::new(),
            config: Config::default(),
        }
    }

    /// Sets the editor content.
    #[must_use]
    pub fn with_content(mut self, content: &str) -> Self {
        self.editor = MockEditor::with_content(content);
        self
    }

    /// Sets the cursor position.
    #[must_use]
    pub fn with_cursor(mut self, line: usize, col: usize) -> Self {
        self.editor.set_cursor(line, col);
        self
    }

    /// Sets the mode.
    #[must_use]
    pub fn with_mode(mut self, mode: Mode) -> Self {
        self.vim_state.set_mode(mode);
        self
    }

    /// Sets indent configuration.
    #[must_use]
    pub fn with_indent_size(mut self, size: usize) -> Self {
        self.config.indent_size = size;
        self
    }

    // ─────────────────────────────────────────────────────────────────
    // Cursor Movement (Direct)
    // ─────────────────────────────────────────────────────────────────

    /// Moves cursor down by N lines (simulates j motion).
    pub fn move_down(&mut self, count: usize) {
        let current = self.editor.cursor();
        let new_line = (current.line + count).min(self.editor.line_count().saturating_sub(1));
        let new_col = current.col.min(self.editor.line_len(new_line));
        self.editor.set_cursor(new_line, new_col);
    }

    /// Moves cursor up by N lines (simulates k motion).
    pub fn move_up(&mut self, count: usize) {
        let current = self.editor.cursor();
        let new_line = current.line.saturating_sub(count);
        let new_col = current.col.min(self.editor.line_len(new_line));
        self.editor.set_cursor(new_line, new_col);
    }

    /// Moves cursor right by N columns (simulates l motion).
    pub fn move_right(&mut self, count: usize) {
        let current = self.editor.cursor();
        let line_len = self.editor.line_len(current.line);
        let new_col = (current.col + count).min(line_len.saturating_sub(1));
        self.editor.set_cursor(current.line, new_col);
    }

    /// Moves cursor left by N columns (simulates h motion).
    pub fn move_left(&mut self, count: usize) {
        let current = self.editor.cursor();
        let new_col = current.col.saturating_sub(count);
        self.editor.set_cursor(current.line, new_col);
    }

    /// Moves cursor to end of line (simulates $ motion).
    pub fn move_to_eol(&mut self) {
        let current = self.editor.cursor();
        let line_len = self.editor.line_len(current.line);
        // $ in normal mode goes to last char, not past it
        let new_col = line_len.saturating_sub(1);
        self.editor.set_cursor(current.line, new_col);
    }

    /// Moves cursor to start of line (simulates 0 motion).
    pub fn move_to_sol(&mut self) {
        let current = self.editor.cursor();
        self.editor.set_cursor(current.line, 0);
    }

    // ─────────────────────────────────────────────────────────────────
    // Assertions
    // ─────────────────────────────────────────────────────────────────

    /// Asserts cursor is at expected position.
    #[track_caller]
    pub fn assert_cursor(&self, line: usize, col: usize) {
        let actual = self.editor.cursor();
        assert_eq!(
            actual,
            Position::new(line, col),
            "Expected cursor at ({}, {}), got ({}, {})",
            line,
            col,
            actual.line,
            actual.col
        );
    }

    /// Asserts editor content matches expected.
    #[track_caller]
    pub fn assert_content(&self, expected: &str) {
        let actual = self.editor.content();
        assert_eq!(actual, expected, "Content mismatch");
    }

    /// Asserts line content matches expected.
    #[track_caller]
    pub fn assert_line(&self, line: usize, expected: &str) {
        let actual = self.editor.line(line);
        assert_eq!(
            actual.as_ref(),
            expected,
            "Line {} mismatch: expected '{}', got '{}'",
            line,
            expected,
            actual.as_ref()
        );
    }

    /// Asserts mode matches expected.
    #[track_caller]
    pub fn assert_mode(&self, expected: &Mode) {
        let actual = self.vim_state.mode();
        assert_eq!(
            &actual, expected,
            "Mode mismatch: expected {:?}, got {:?}",
            expected, actual
        );
    }

    // ─────────────────────────────────────────────────────────────────
    // Getters
    // ─────────────────────────────────────────────────────────────────

    /// Returns current cursor position as (line, col) tuple.
    #[must_use]
    pub fn cursor(&self) -> (usize, usize) {
        let pos = self.editor.cursor();
        (pos.line, pos.col)
    }

    /// Returns current line content.
    #[must_use]
    pub fn current_line(&self) -> String {
        self.editor.line(self.editor.cursor().line).to_string()
    }
}

impl Default for TestHarness {
    fn default() -> Self {
        Self::new()
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Tests
// ═══════════════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_harness_basic_motion_down() {
        let mut harness = TestHarness::new()
            .with_content("line 1\nline 2\nline 3")
            .with_cursor(0, 0);

        harness.move_down(1);

        harness.assert_cursor(1, 0);
    }

    #[test]
    fn test_harness_basic_motion_right() {
        let mut harness = TestHarness::new()
            .with_content("hello world")
            .with_cursor(0, 0);

        harness.move_right(1);

        harness.assert_cursor(0, 1);
    }

    #[test]
    fn test_harness_motion_end_of_line() {
        let mut harness = TestHarness::new().with_content("hello").with_cursor(0, 0);

        harness.move_to_eol();

        harness.assert_cursor(0, 4); // Last char, not past it
    }

    #[test]
    fn test_harness_motion_with_count() {
        let mut harness = TestHarness::new()
            .with_content("a\nb\nc\nd\ne")
            .with_cursor(0, 0);

        harness.move_down(3);

        harness.assert_cursor(3, 0);
    }

    #[test]
    fn test_harness_line_assertion() {
        let harness = TestHarness::new().with_content("first\nsecond\nthird");

        harness.assert_line(0, "first");
        harness.assert_line(1, "second");
        harness.assert_line(2, "third");
    }

    #[test]
    fn test_harness_content_assertion() {
        let harness = TestHarness::new().with_content("hello\nworld");

        harness.assert_content("hello\nworld");
    }

    #[test]
    fn test_harness_move_up() {
        let mut harness = TestHarness::new().with_content("a\nb\nc").with_cursor(2, 0);

        harness.move_up(1);

        harness.assert_cursor(1, 0);
    }

    #[test]
    fn test_harness_move_left() {
        let mut harness = TestHarness::new().with_content("hello").with_cursor(0, 3);

        harness.move_left(2);

        harness.assert_cursor(0, 1);
    }

    #[test]
    fn test_harness_move_to_sol() {
        let mut harness = TestHarness::new().with_content("hello").with_cursor(0, 3);

        harness.move_to_sol();

        harness.assert_cursor(0, 0);
    }

    #[test]
    fn test_harness_cursor_clamps_to_line_length() {
        let mut harness = TestHarness::new()
            .with_content("short\nlonger line")
            .with_cursor(1, 10);

        harness.move_up(1);

        // Column should clamp to "short".len() = 5
        harness.assert_cursor(0, 5);
    }
}
