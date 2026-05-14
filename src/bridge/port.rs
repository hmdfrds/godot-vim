//! `TextEditorPort` — abstraction over Godot's CodeEdit for testability.
//!
//! Trait hierarchy:
//!
//! - **`TextEditorPort`** (26 methods) — core text editing (required).
//! - **`FoldCapable`** (5 methods, default no-ops) — code folding.
//! - **`IdeCapable`** (2 methods, default no-ops) — autocomplete/hints.
//! - **`NavigationCapable`** (2 methods, default no-ops) — go-to-definition, hover docs.
//!
//! Production: `CodeEditPort` (in `port_impl.rs`) implements all four.
//! Tests: `MockTextEdit` implements `TextEditorPort`; extension traits
//! use their default no-op bodies.

use crate::types::CharLineCol;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ViewportAdjust {
    Adjust,
    NoAdjust,
}

/// Core text editor abstraction used by the effect dispatch layer.
///
/// Mirrors Godot's CodeEdit API with Rust-native types (`String`/`&str`
/// instead of `GString`). Only methods called through the trait abstraction
/// during effect dispatch are included — methods called directly on
/// `Gd<CodeEdit>` (e.g. `get_line_count`, `has_selection`) are intentionally
/// omitted to keep the mock surface small.
pub(crate) trait TextEditorPort {
    fn get_text(&self) -> String;
    fn get_line(&self, line: i32) -> String;

    /// Coordinate-addressed insert — no caret/selection involvement.
    fn insert_text(&mut self, text: &str, line: i32, col: i32);
    /// Coordinate-addressed removal — no caret/selection involvement.
    fn remove_text(&mut self, from_line: i32, from_col: i32, to_line: i32, to_col: i32);

    /// Caret-relative insert. Test-only; production uses `insert_text`.
    #[allow(dead_code)]
    fn insert_text_at_caret(&mut self, text: &str);
    #[allow(dead_code)]
    fn delete_selection(&mut self);

    fn set_caret_line(&mut self, line: i32);
    fn set_caret_column(&mut self, col: i32);
    fn get_caret_line(&self) -> i32;
    fn get_caret_column(&self) -> i32;

    /// Set caret line with fold-awareness: always unfolds the target line
    /// (`can_be_hidden=false`) and optionally scrolls the viewport.
    fn set_caret_line_unfold(&mut self, line: i32, viewport: ViewportAdjust);
    fn adjust_viewport_to_caret(&mut self);

    /// `from` = selection origin (anchor), `to` = caret position (head).
    fn select(&mut self, from: CharLineCol, to: CharLineCol);
    fn deselect(&mut self);
    /// Like `select`, but targets a specific caret index (for multi-caret).
    fn select_for_caret(&mut self, from: CharLineCol, to: CharLineCol, caret_index: i32);

    /// Returns the new caret index, or -1 on failure.
    fn add_caret(&mut self, line: i32, col: i32) -> i32;
    fn remove_caret(&mut self, caret_idx: i32);
    fn remove_secondary_carets(&mut self);
    fn get_caret_count(&self) -> i32;

    /// Get caret line for a specific caret index (multi-cursor import).
    fn get_caret_line_for(&self, caret_idx: i32) -> i32;
    /// Get caret column for a specific caret index (multi-cursor import).
    fn get_caret_column_for(&self, caret_idx: i32) -> i32;

    /// Set caret line for a specific caret index (multi-cursor sync).
    fn set_caret_line_for(&mut self, line: i32, caret_idx: i32);
    /// Set caret column for a specific caret index (multi-cursor sync).
    fn set_caret_column_for(&mut self, col: i32, caret_idx: i32);

    /// Groups subsequent edits into a single undo step. Nesting is supported.
    fn begin_complex_operation(&mut self);
    fn end_complex_operation(&mut self);

    fn begin_multicaret_edit(&mut self);
    fn end_multicaret_edit(&mut self);
    fn undo(&mut self);
    fn redo(&mut self);

    fn set_v_scroll(&mut self, value: f64);
    fn get_first_visible_line(&self) -> i32;
    fn get_visible_line_count(&self) -> i32;
    fn set_h_scroll(&mut self, value: i32);
    fn get_h_scroll(&self) -> i32;

    /// How many document lines to skip to reach `visible_amount` visible lines
    /// from `line`. For editors without folds, returns `visible_amount.abs()`.
    fn get_next_visible_line_offset_from(&self, line: i32, visible_amount: i32) -> i32;
}

// Extension traits: optional capabilities with default no-op bodies so that
// test mocks can implement them trivially (`impl FoldCapable for MockTextEdit {}`).

/// Code folding. Default no-ops for editors without fold support.
pub(crate) trait FoldCapable: TextEditorPort {
    fn fold_line(&mut self, _line: i32) {}
    fn unfold_line(&mut self, _line: i32) {}
    fn toggle_foldable_line(&mut self, _line: i32) {}
    fn fold_all_lines(&mut self) {}
    fn unfold_all_lines(&mut self) {}
}

/// IDE autocomplete/hint integration. Default no-ops for test mocks.
pub(crate) trait IdeCapable: TextEditorPort {
    fn cancel_code_completion(&mut self) {}
    fn dismiss_code_hint(&mut self) {}
}

/// LSP navigation: go-to-definition and hover documentation.
///
/// The production implementation (`CodeEditPort`) emits Godot signals
/// (`symbol_lookup`, `symbol_hovered`) that `ScriptTextEditor` listens on
/// to trigger the actual language server actions.
pub(crate) trait NavigationCapable: TextEditorPort {
    fn emit_symbol_lookup(&mut self, _symbol: &str, _line: i32, _col: i32) {}

    /// Show documentation tooltip for the symbol at the given position.
    fn show_documentation_tooltip(&mut self, _symbol: &str, _line: i32, _col: i32) {}
}
