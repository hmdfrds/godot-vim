//! Mock editor for testing without Godot runtime.
//!
//! `MockEditor` provides an in-memory simulation of Godot's CodeEdit,
//! implementing the same operations used by shell handlers.
#![allow(dead_code)]

use vim_core::domain::column::ByteCol;
use vim_core::domain::position::Position;
use vim_core::domain::selection::Selection;
use vim_core::domain::shared_str::SharedStr;
use vim_core::domain::snapshot::DocumentSnapshot;

/// In-memory editor buffer for testing.
///
/// Simulates the essential operations of Godot's `CodeEdit` without
/// requiring the Godot runtime. Lines are stored as `Vec<String>`.
#[derive(Debug, Clone)]
pub struct MockEditor {
    /// Lines of text (without trailing newlines).
    lines: Vec<String>,
    /// Current cursor position (line, column).
    cursor: Position,
    /// Selection anchor (same as cursor if no selection).
    anchor: Position,
    /// Simulated fold state: set of folded line indices.
    folded_lines: std::collections::HashSet<usize>,
}

impl MockEditor {
    /// Creates a new empty editor.
    #[must_use]
    pub fn new() -> Self {
        Self {
            lines: vec![String::new()],
            cursor: Position::from_byte(0, 0),
            anchor: Position::from_byte(0, 0),
            folded_lines: std::collections::HashSet::new(),
        }
    }

    /// Creates an editor with initial content.
    #[must_use]
    pub fn with_content(content: &str) -> Self {
        let lines: Vec<String> = if content.is_empty() {
            vec![String::new()]
        } else {
            content.lines().map(String::from).collect()
        };
        Self {
            lines,
            cursor: Position::from_byte(0, 0),
            anchor: Position::from_byte(0, 0),
            folded_lines: std::collections::HashSet::new(),
        }
    }

    /// Sets the cursor position.
    pub fn set_cursor(&mut self, line: usize, col: usize) {
        let line = line.min(self.lines.len().saturating_sub(1));
        let col = col.min(self.line_len(line));
        self.cursor = Position::from_byte(line, col);
        self.anchor = self.cursor;
    }

    /// Sets a visual selection from anchor to cursor.
    pub fn set_selection(&mut self, anchor: Position, cursor: Position) {
        self.anchor = anchor;
        self.cursor = cursor;
    }

    /// Returns current cursor position.
    #[must_use]
    pub fn cursor(&self) -> Position {
        self.cursor
    }

    /// Returns current anchor position.
    #[must_use]
    pub fn anchor(&self) -> Position {
        self.anchor
    }

    /// Returns true if there's an active selection.
    #[must_use]
    pub fn has_selection(&self) -> bool {
        self.cursor != self.anchor
    }

    /// Returns the current selection.
    #[must_use]
    pub fn selection(&self) -> Selection {
        Selection::new(self.anchor, self.cursor)
    }

    /// Returns the length of a line.
    #[must_use]
    pub fn line_len(&self, line: usize) -> usize {
        self.lines.get(line).map_or(0, |s| s.len())
    }

    /// Simulates folding a line.
    pub fn fold_line(&mut self, line: usize) {
        self.folded_lines.insert(line);
    }

    /// Simulates unfolding a line.
    pub fn unfold_line(&mut self, line: usize) {
        self.folded_lines.remove(&line);
    }

    /// Returns true if a line is folded.
    #[must_use]
    pub fn is_line_folded(&self, line: usize) -> bool {
        self.folded_lines.contains(&line)
    }

    // ─────────────────────────────────────────────────────────────────
    // Text Mutation Operations
    // ─────────────────────────────────────────────────────────────────

    /// Inserts text at the current cursor position.
    pub fn insert_text(&mut self, text: &str) {
        let line = self.cursor.line;
        let col = self.cursor.col.as_usize();

        // Handle multi-line insertion
        let insert_lines: Vec<&str> = text.split('\n').collect();

        if insert_lines.len() == 1 {
            // Simple single-line insert
            if let Some(current_line) = self.lines.get_mut(line) {
                current_line.insert_str(col, text);
                self.cursor.col = self.cursor.col.saturating_add_bytes(text.len());
            }
        } else {
            // Multi-line insert
            let current_line = self.lines.get(line).cloned().unwrap_or_default();
            let before = &current_line[..col.min(current_line.len())];
            let after = &current_line[col.min(current_line.len())..];

            // First line: before + first insert segment
            self.lines[line] = format!("{}{}", before, insert_lines[0]);

            // Middle lines: insert as-is
            for (i, insert_line) in insert_lines[1..insert_lines.len() - 1].iter().enumerate() {
                self.lines.insert(line + 1 + i, insert_line.to_string());
            }

            // Last line: last insert segment + after
            let last_idx = insert_lines.len() - 1;
            let last_line = format!("{}{}", insert_lines[last_idx], after);
            self.lines.insert(line + last_idx, last_line);

            // Update cursor
            self.cursor.line = line + last_idx;
            self.cursor.col = ByteCol::new(insert_lines[last_idx].len());
        }

        self.anchor = self.cursor;
    }

    /// Deletes the selection (or character at cursor if no selection).
    pub fn delete_selection(&mut self) {
        if !self.has_selection() {
            // Delete single character at cursor
            if let Some(line) = self.lines.get_mut(self.cursor.line) {
                if self.cursor.col.as_usize() < line.len() {
                    line.remove(self.cursor.col.as_usize());
                }
            }
            return;
        }

        // Delete selection
        let sel = self.selection();
        let (start, end) = sel.range();

        let start_line_content = self.lines.get(start.line).cloned().unwrap_or_default();
        let end_line_content = self.lines.get(end.line).cloned().unwrap_or_default();

        let before = &start_line_content[..start.col.min(start_line_content.len())];
        let after = &end_line_content[end.col.min(end_line_content.len())..];

        // Replace with merged line
        self.lines[start.line] = format!("{}{}", before, after);

        // Remove lines in between
        for _ in start.line + 1..=end.line {
            if start.line + 1 < self.lines.len() {
                self.lines.remove(start.line + 1);
            }
        }

        // Ensure at least one line
        if self.lines.is_empty() {
            self.lines.push(String::new());
        }

        // Move cursor to start of selection
        self.cursor = start;
        self.anchor = start;
    }

    /// Returns the entire content as a single string.
    #[must_use]
    pub fn content(&self) -> String {
        self.lines.join("\n")
    }

    /// Returns the lines as a slice.
    #[must_use]
    pub fn lines(&self) -> &[String] {
        &self.lines
    }
}

impl Default for MockEditor {
    fn default() -> Self {
        Self::new()
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// DocumentSnapshot Implementation
// ═══════════════════════════════════════════════════════════════════════════

impl DocumentSnapshot for MockEditor {
    fn line_count(&self) -> usize {
        self.lines.len()
    }

    fn line(&self, idx: usize) -> SharedStr {
        self.lines
            .get(idx)
            .map(|s| SharedStr::from(s.as_str()))
            .unwrap_or_else(|| SharedStr::from(""))
    }

    fn selection(&self) -> Selection {
        Selection::new(self.anchor, self.cursor)
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Tests
// ═══════════════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_editor_has_one_empty_line() {
        let editor = MockEditor::new();
        assert_eq!(editor.line_count(), 1);
        assert_eq!(editor.line(0).as_ref(), "");
    }

    #[test]
    fn test_with_content_splits_lines() {
        let editor = MockEditor::with_content("hello\nworld\n!");
        assert_eq!(editor.line_count(), 3);
        assert_eq!(editor.line(0).as_ref(), "hello");
        assert_eq!(editor.line(1).as_ref(), "world");
        assert_eq!(editor.line(2).as_ref(), "!");
    }

    #[test]
    fn test_set_cursor_clamps_to_bounds() {
        let mut editor = MockEditor::with_content("short");
        editor.set_cursor(100, 100);
        assert_eq!(editor.cursor(), Position::from_byte(0, 5));
    }

    #[test]
    fn test_insert_text_single_line() {
        let mut editor = MockEditor::with_content("hello");
        editor.set_cursor(0, 5);
        editor.insert_text(" world");
        assert_eq!(editor.line(0).as_ref(), "hello world");
        assert_eq!(editor.cursor().col.as_usize(), 11);
    }

    #[test]
    fn test_delete_selection() {
        let mut editor = MockEditor::with_content("hello world");
        editor.set_selection(Position::from_byte(0, 0), Position::from_byte(0, 6));
        editor.delete_selection();
        assert_eq!(editor.line(0).as_ref(), "world");
    }

    #[test]
    fn test_fold_unfold() {
        let mut editor = MockEditor::with_content("a\nb\nc");
        assert!(!editor.is_line_folded(1));
        editor.fold_line(1);
        assert!(editor.is_line_folded(1));
        editor.unfold_line(1);
        assert!(!editor.is_line_folded(1));
    }
}
