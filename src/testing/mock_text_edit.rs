//! Pure-Rust mock of Godot's TextEdit/CodeEdit for testing the bridge layer
//! without the Godot runtime.
//!
//! Replicates behavioral semantics from Godot's `text_edit.cpp`: text storage
//! (`Vec<String>`), caret model with `last_fit_x`, origin-based selection,
//! multi-caret, and group-based undo with caret snapshots. This lets bridge
//! effect handlers run against a real text model in `cargo test` without
//! requiring a Godot editor instance or GDExtension linkage.
//!
//! ## Undo model
//!
//! Godot's native undo uses chain-linked operations (`chain_forward`/
//! `chain_backward`) designed for its ACTION system where consecutive
//! same-action edits auto-chain. The vim plugin instead uses bare
//! `begin/end_complex_operation` calls where each outermost pair must
//! form one atomic undo step.
//!
//! This mock simplifies to a group-based model: each outermost `begin/end`
//! pair creates one `UndoGroup` containing all text operations within.
//! `undo()` reverts the most recent group, `redo()` replays it. This is
//! semantically equivalent for the vim plugin's usage pattern, though it
//! skips Godot's chain-linking internals that are irrelevant to us.
//!
//! Reference: `/reference/godot/scene/gui/text_edit.{h,cpp}`

use crate::bridge::port::TextEditorPort;

// ─── Data structures ────────────────────────────────────────────────────────
// These mirror Godot's internal structs (Caret, Selection) at the field level.
// Using separate Rust structs rather than tuple aliases preserves the same
// field semantics that Godot's C++ code relies on (e.g., origin-based selection
// where origin != caret position).

/// Mirrors Godot's `Selection` struct — origin-based, not anchor/head.
/// The origin is where the selection started; the caret may be before or after it.
#[derive(Clone, Default, Debug)]
struct Selection {
    active: bool,
    origin_line: i32,
    origin_column: i32,
}

/// Mirrors Godot's `Caret` struct. `last_fit_x` tracks the column the user
/// "intended" when moving vertically through lines of varying width.
#[derive(Clone, Default, Debug)]
struct Caret {
    line: i32,
    column: i32,
    last_fit_x: i32,
    selection: Selection,
}

#[derive(Clone, Debug)]
struct TextOperation {
    op_type: OpType,
    from_line: i32,
    from_column: i32,
    to_line: i32,
    to_column: i32,
    text: String,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum OpType {
    Insert,
    Remove,
}

/// One atomic undo step. Corresponds to one outermost `begin/end_complex_operation`
/// pair. Captures caret snapshots at both boundaries so undo restores the caret
/// to its pre-edit position (matching Godot's behavior).
#[derive(Clone, Debug)]
struct UndoGroup {
    ops: Vec<TextOperation>,
    start_carets: Vec<Caret>,
    end_carets: Vec<Caret>,
}

// ─── MockTextEdit ───────────────────────────────────────────────────────────

/// Pure-Rust mock of Godot's TextEdit with faithful behavioral semantics.
///
/// Key invariants (matching Godot's C++ implementation):
/// - `lines` always has >= 1 entry (empty document = `vec![""]`)
/// - `carets` always has >= 1 entry (primary caret at index 0)
/// - Column range: `[0, line.len()]` inclusive (cursor can sit past last char)
/// - Line range: `[0, lines.len() - 1]`
#[derive(Debug)]
pub(crate) struct MockTextEdit {
    lines: Vec<String>,

    /// Index 0 is the primary caret, never removed by `remove_secondary_carets`.
    carets: Vec<Caret>,

    // ── Undo state ──
    undo_stack: Vec<UndoGroup>,
    /// Points past the last undone group. Equal to `undo_stack.len()` when
    /// there is nothing to redo.
    undo_pos: usize,
    /// The group being accumulated inside the current `begin/end` pair.
    current_group: Option<UndoGroup>,
    /// Nesting depth — only the outermost `end` finalizes the group.
    complex_operation_count: u32,

    // ── Scroll state (simplified: no wrap, no minimap) ──
    v_scroll: f64,
    h_scroll: i32,
    visible_line_count: i32,
}

impl MockTextEdit {
    /// Lines are split on `\n`. An empty string produces a single empty line
    /// (matching Godot's invariant that a document always has at least one line).
    pub(crate) fn new(text: &str) -> Self {
        let lines: Vec<String> = if text.is_empty() {
            vec![String::new()]
        } else {
            text.split('\n').map(String::from).collect()
        };

        Self {
            lines,
            carets: vec![Caret::default()],
            undo_stack: Vec::new(),
            undo_pos: 0,
            current_group: None,
            complex_operation_count: 0,
            v_scroll: 0.0,
            h_scroll: 0,
            visible_line_count: 25,
        }
    }

    pub(crate) fn caret_count(&self) -> usize {
        self.carets.len()
    }

    pub(crate) fn set_visible_line_count(&mut self, count: i32) {
        self.visible_line_count = count;
    }

    // ── Core text operations ─────────────────────────────────────────────
    // Named after Godot's `_base_insert_text` / `_base_remove_text` which
    // perform raw text manipulation without undo recording.

    /// Returns `(end_line, end_col)` — the position just past the inserted text.
    fn base_insert_text(&mut self, p_line: i32, p_col: i32, p_text: &str) -> (i32, i32) {
        debug_assert!(p_line >= 0, "negative line: {}", p_line);
        debug_assert!(p_col >= 0, "negative col: {}", p_col);
        let line_idx = p_line as usize;
        let col = p_col as usize;

        // Strip \r to normalize CRLF — Godot stores lines without line endings.
        let clean_text: String = p_text.chars().filter(|&c| c != '\r').collect();
        let substrings: Vec<&str> = clean_text.split('\n').collect();

        // Godot pads with spaces when inserting past the end of a line.
        let current_line = &self.lines[line_idx];
        let current_len = current_line.len();
        let padded_line = if col > current_len {
            let mut s = current_line.clone();
            s.extend(std::iter::repeat_n(' ', col - current_len));
            s
        } else {
            current_line.clone()
        };

        let pre = &padded_line[..col];
        let post = &padded_line[col..];

        let mut new_lines: Vec<String> = Vec::with_capacity(substrings.len());
        for (i, &substr) in substrings.iter().enumerate() {
            let mut line_text = String::new();
            if i == 0 {
                line_text.push_str(pre);
            }
            line_text.push_str(substr);
            if i == substrings.len() - 1 {
                line_text.push_str(post);
            }
            new_lines.push(line_text);
        }

        self.lines[line_idx] = new_lines[0].clone();
        for (i, new_line) in new_lines.into_iter().enumerate().skip(1) {
            self.lines.insert(line_idx + i, new_line);
        }

        let end_line = p_line + (substrings.len() as i32 - 1);
        let end_col = self.lines[end_line as usize].len() as i32 - post.len() as i32;

        (end_line, end_col)
    }

    fn base_remove_text(
        &mut self,
        from_line: i32,
        from_col: i32,
        to_line: i32,
        to_col: i32,
    ) -> String {
        debug_assert!(from_line >= 0, "negative from_line: {}", from_line);
        debug_assert!(from_col >= 0, "negative from_col: {}", from_col);
        debug_assert!(to_line >= 0, "negative to_line: {}", to_line);
        debug_assert!(to_col >= 0, "negative to_col: {}", to_col);
        let removed = self.base_get_text(from_line, from_col, to_line, to_col);

        let pre = self.lines[from_line as usize][..from_col as usize].to_string();
        let post = self.lines[to_line as usize][to_col as usize..].to_string();

        if to_line > from_line {
            self.lines
                .drain((from_line as usize + 1)..=(to_line as usize));
        }

        self.lines[from_line as usize] = pre + &post;

        removed
    }

    fn base_get_text(&self, from_line: i32, from_col: i32, to_line: i32, to_col: i32) -> String {
        debug_assert!(from_line >= 0, "negative from_line: {}", from_line);
        debug_assert!(from_col >= 0, "negative from_col: {}", from_col);
        debug_assert!(to_line >= 0, "negative to_line: {}", to_line);
        debug_assert!(to_col >= 0, "negative to_col: {}", to_col);
        if from_line == to_line {
            return self.lines[from_line as usize][from_col as usize..to_col as usize].to_string();
        }

        let mut result = String::new();
        for line in from_line..=to_line {
            let line_text = &self.lines[line as usize];
            if line == from_line {
                result.push_str(&line_text[from_col as usize..]);
            } else if line == to_line {
                result.push('\n');
                result.push_str(&line_text[..to_col as usize]);
            } else {
                result.push('\n');
                result.push_str(line_text);
            }
        }
        result
    }

    // ── Undo-recording wrappers ─────────────────────────────────────────
    // These pair raw text mutation with undo recording, matching how Godot's
    // `insert_text` and `remove_text` interact with the undo system.

    fn insert_text_record(&mut self, p_line: i32, p_col: i32, p_text: &str) -> (i32, i32) {
        self.clear_redo();

        let (end_line, end_col) = self.base_insert_text(p_line, p_col, p_text);

        let op = TextOperation {
            op_type: OpType::Insert,
            from_line: p_line,
            from_column: p_col,
            to_line: end_line,
            to_column: end_col,
            text: p_text.to_string(),
        };

        self.record_op(op);
        (end_line, end_col)
    }

    fn remove_text_record(&mut self, from_line: i32, from_col: i32, to_line: i32, to_col: i32) {
        self.clear_redo();

        let removed_text = self.base_get_text(from_line, from_col, to_line, to_col);
        self.base_remove_text(from_line, from_col, to_line, to_col);

        let op = TextOperation {
            op_type: OpType::Remove,
            from_line,
            from_column: from_col,
            to_line,
            to_column: to_col,
            text: removed_text,
        };

        self.record_op(op);
    }

    fn record_op(&mut self, op: TextOperation) {
        if let Some(group) = &mut self.current_group {
            group.ops.push(op);
        }
        // Ops outside a begin/end pair are silently dropped from undo history.
        // Godot would record them in `current_op` and push on next `begin`, but
        // the vim plugin never mutates text outside an undo group, so the
        // simplified behavior is equivalent for our use case.
    }

    /// Standard undo semantics: any new edit after an undo discards the redo future.
    fn clear_redo(&mut self) {
        if self.undo_pos < self.undo_stack.len() {
            self.undo_stack.truncate(self.undo_pos);
        }
    }

    /// Replay or reverse a recorded operation. Undo passes `reverse=true`,
    /// which flips Insert<->Remove so the same `TextOperation` struct serves
    /// both directions without storing separate forward/backward data.
    fn do_text_op(&mut self, op: &TextOperation, reverse: bool) {
        let effective_type = if reverse {
            match op.op_type {
                OpType::Insert => OpType::Remove,
                OpType::Remove => OpType::Insert,
            }
        } else {
            op.op_type
        };

        match effective_type {
            OpType::Insert => {
                self.base_insert_text(op.from_line, op.from_column, &op.text);
            }
            OpType::Remove => {
                self.base_remove_text(op.from_line, op.from_column, op.to_line, op.to_column);
            }
        }
    }

    // ── Caret offset after edit ──────────────────────────────────────────
    // When text is inserted or removed, all carets positioned after the edit
    // point must be shifted to stay at the "same" logical position. This
    // mirrors Godot's `_offset_carets_after` which adjusts both caret
    // positions and selection origins.

    fn offset_carets_after(&mut self, old_line: i32, old_col: i32, new_line: i32, new_col: i32) {
        let edit_height = new_line - old_line;
        let edit_size = new_col - old_col;

        for caret in &mut self.carets {
            Self::offset_point(
                &mut caret.line,
                &mut caret.column,
                old_line,
                old_col,
                edit_height,
                edit_size,
                new_line,
            );

            if caret.selection.active {
                Self::offset_point(
                    &mut caret.selection.origin_line,
                    &mut caret.selection.origin_column,
                    old_line,
                    old_col,
                    edit_height,
                    edit_size,
                    new_line,
                );
            }
        }
    }

    fn offset_point(
        line: &mut i32,
        col: &mut i32,
        old_line: i32,
        old_col: i32,
        edit_height: i32,
        edit_size: i32,
        new_line: i32,
    ) {
        if *line > old_line || (*line == old_line && *col > old_col) {
            *line += edit_height;
            if *line == new_line {
                *col += edit_size;
            }
        }
    }

    // ── Caret clamping ───────────────────────────────────────────────────

    fn clamp_line(&self, line: i32) -> i32 {
        line.clamp(0, self.lines.len() as i32 - 1)
    }

    fn clamp_column(&self, line: i32, col: i32) -> i32 {
        let line_len = self.lines[line as usize].len() as i32;
        col.clamp(0, line_len)
    }
}

// ─── TextEditorPort implementation ───────────────────────────────────────────
// This is the core value of MockTextEdit: implementing the same trait that
// the production CodeEditPort does, so effect handlers can be tested against
// a real text model without Godot.

impl TextEditorPort for MockTextEdit {
    // ── Text content ────────────────────────────────────────────────────

    fn get_text(&self) -> String {
        self.lines.join("\n")
    }

    fn get_line(&self, line: i32) -> String {
        if line < 0 || line >= self.lines.len() as i32 {
            return String::new();
        }
        self.lines[line as usize].clone()
    }

    /// Matches Godot's multi-caret insert: iterates all carets, deletes any
    /// active selection first, inserts text, then offsets subsequent carets.
    fn insert_text_at_caret(&mut self, text: &str) {
        self.begin_complex_operation();

        let caret_count = self.carets.len();
        for i in 0..caret_count {
            if self.carets[i].selection.active {
                let from_line = self.selection_from_line(i);
                let from_col = self.selection_from_col(i);
                let to_line = self.selection_to_line(i);
                let to_col = self.selection_to_col(i);

                self.remove_text_record(from_line, from_col, to_line, to_col);
                self.offset_carets_after(to_line, to_col, from_line, from_col);

                self.carets[i].selection.active = false;
                self.carets[i].line = from_line;
                self.carets[i].column = from_col;
            }

            let from_line = self.carets[i].line;
            let from_col = self.carets[i].column;

            let (new_line, new_col) = self.insert_text_record(from_line, from_col, text);

            self.offset_carets_after(from_line, from_col, new_line, new_col);

            self.carets[i].line = new_line;
            self.carets[i].column = new_col;
        }

        self.end_complex_operation();
    }

    fn delete_selection(&mut self) {
        let caret = &self.carets[0];
        if !caret.selection.active {
            return;
        }

        let from_line = self.selection_from_line(0);
        let from_col = self.selection_from_col(0);
        let to_line = self.selection_to_line(0);
        let to_col = self.selection_to_col(0);

        self.begin_complex_operation();

        self.remove_text_record(from_line, from_col, to_line, to_col);
        self.offset_carets_after(to_line, to_col, from_line, from_col);

        self.carets[0].selection.active = false;
        self.carets[0].line = from_line;
        self.carets[0].column = from_col;

        self.end_complex_operation();
    }

    // ── Cursor ──────────────────────────────────────────────────────────

    fn set_caret_line(&mut self, line: i32) {
        let clamped = self.clamp_line(line);
        self.carets[0].line = clamped;
        let col = self.carets[0].column;
        self.carets[0].column = self.clamp_column(clamped, col);
        self.maybe_deselect(0);
    }

    fn set_caret_column(&mut self, col: i32) {
        let line = self.carets[0].line;
        let clamped = self.clamp_column(line, col);
        self.carets[0].column = clamped;
        self.carets[0].last_fit_x = clamped;
        self.maybe_deselect(0);
    }

    fn get_caret_line(&self) -> i32 {
        self.carets[0].line
    }

    fn get_caret_column(&self) -> i32 {
        self.carets[0].column
    }

    /// No folds to unfold in the mock — delegates straight to `set_caret_line`.
    fn set_caret_line_unfold(&mut self, line: i32, _viewport: crate::bridge::port::ViewportAdjust) {
        self.set_caret_line(line);
    }

    fn adjust_viewport_to_caret(&mut self) {
        let caret_line = self.carets[0].line;
        let first_visible = self.v_scroll as i32;
        let last_visible = first_visible + self.visible_line_count - 1;

        if caret_line < first_visible {
            self.v_scroll = caret_line as f64;
        } else if caret_line > last_visible {
            self.v_scroll = (caret_line - self.visible_line_count + 1).max(0) as f64;
        }
    }

    // ── Selection ───────────────────────────────────────────────────────

    fn select(&mut self, from: crate::types::CharLineCol, to: crate::types::CharLineCol) {
        let caret_line = self.clamp_line(to.line);
        let caret_col = self.clamp_column(caret_line, to.col);
        self.carets[0].line = caret_line;
        self.carets[0].column = caret_col;

        let origin_line = self.clamp_line(from.line);
        let origin_col = self.clamp_column(origin_line, from.col);
        self.carets[0].selection.origin_line = origin_line;
        self.carets[0].selection.origin_column = origin_col;

        self.carets[0].selection.active = origin_line != caret_line || origin_col != caret_col;
    }

    fn deselect(&mut self) {
        for caret in &mut self.carets {
            caret.selection.active = false;
        }
    }

    fn select_for_caret(
        &mut self,
        from: crate::types::CharLineCol,
        to: crate::types::CharLineCol,
        caret_index: i32,
    ) {
        debug_assert!(caret_index >= 0, "negative caret_index: {}", caret_index);
        let idx = caret_index as usize;
        if idx >= self.carets.len() {
            return;
        }

        let caret_line = self.clamp_line(to.line);
        let caret_col = self.clamp_column(caret_line, to.col);
        self.carets[idx].line = caret_line;
        self.carets[idx].column = caret_col;

        let origin_line = self.clamp_line(from.line);
        let origin_col = self.clamp_column(origin_line, from.col);
        self.carets[idx].selection.origin_line = origin_line;
        self.carets[idx].selection.origin_column = origin_col;

        self.carets[idx].selection.active = origin_line != caret_line || origin_col != caret_col;
    }

    // ── Multi-caret ─────────────────────────────────────────────────────

    fn add_caret(&mut self, line: i32, col: i32) -> i32 {
        let clamped_line = self.clamp_line(line);
        let clamped_col = self.clamp_column(clamped_line, col);

        self.carets.push(Caret {
            line: clamped_line,
            column: clamped_col,
            last_fit_x: clamped_col,
            selection: Selection::default(),
        });

        (self.carets.len() - 1) as i32
    }

    fn remove_secondary_carets(&mut self) {
        self.carets.truncate(1);
    }

    // ── Undo ────────────────────────────────────────────────────────────

    fn begin_complex_operation(&mut self) {
        if self.complex_operation_count == 0 {
            // Snapshot carets now — undo will restore to this state.
            self.current_group = Some(UndoGroup {
                ops: Vec::new(),
                start_carets: self.carets.clone(),
                end_carets: Vec::new(),
            });
        }
        self.complex_operation_count += 1;
    }

    fn end_complex_operation(&mut self) {
        if self.complex_operation_count == 0 {
            return;
        }
        self.complex_operation_count -= 1;

        if self.complex_operation_count == 0 {
            if let Some(mut group) = self.current_group.take() {
                // Only push groups that actually modified text — empty groups
                // (e.g., a begin/end pair around a no-op command) are discarded.
                if !group.ops.is_empty() {
                    group.end_carets = self.carets.clone();
                    self.undo_stack.truncate(self.undo_pos);
                    self.undo_stack.push(group);
                    self.undo_pos = self.undo_stack.len();
                }
            }
        }
    }

    fn undo(&mut self) {
        // Force-close any open group so partial operations don't leak.
        while self.complex_operation_count > 0 {
            self.end_complex_operation();
        }

        if self.undo_pos == 0 {
            return;
        }

        self.undo_pos -= 1;
        let group = self.undo_stack[self.undo_pos].clone();

        for caret in &mut self.carets {
            caret.selection.active = false;
        }

        // Reverse order: last-applied op is undone first.
        for op in group.ops.iter().rev() {
            self.do_text_op(op, true);
        }

        self.carets = group.start_carets;
        if self.carets.is_empty() {
            self.carets = vec![Caret::default()];
        }
    }

    fn redo(&mut self) {
        if self.undo_pos >= self.undo_stack.len() {
            return;
        }

        let group = self.undo_stack[self.undo_pos].clone();

        for caret in &mut self.carets {
            caret.selection.active = false;
        }

        for op in &group.ops {
            self.do_text_op(op, false);
        }

        self.carets = group.end_carets;
        if self.carets.is_empty() {
            self.carets = vec![Caret::default()];
        }

        self.undo_pos += 1;
    }

    // ── Scroll ──────────────────────────────────────────────────────────

    fn set_v_scroll(&mut self, value: f64) {
        self.v_scroll = value.max(0.0);
    }

    fn get_first_visible_line(&self) -> i32 {
        self.v_scroll as i32
    }

    fn get_visible_line_count(&self) -> i32 {
        self.visible_line_count
    }

    fn set_h_scroll(&mut self, value: i32) {
        self.h_scroll = value.max(0);
    }

    fn get_h_scroll(&self) -> i32 {
        self.h_scroll
    }

    /// Without code folding, every line is visible — so the visible offset
    /// from any line is just the absolute count requested.
    fn get_next_visible_line_offset_from(&self, _line: i32, visible_amount: i32) -> i32 {
        visible_amount.abs()
    }
}

// ─── Extension trait implementations ────────────────────────────────────────
// All default no-ops. The mock has no real fold/IDE/navigation behavior, but
// dispatch tests exercise these trait paths to verify the bridge doesn't panic
// when the engine emits fold or IDE effects.

use crate::bridge::port::{FoldCapable, IdeCapable, NavigationCapable};

impl FoldCapable for MockTextEdit {}
impl IdeCapable for MockTextEdit {}
impl NavigationCapable for MockTextEdit {}

// ─── Inherent methods for test assertions ───────────────────────────────────
// These are NOT on the TextEditorPort trait because the production CodeEditPort
// calls Godot's built-in `has_selection()` / `get_line_count()` directly via
// the Godot API rather than through the trait. The assert_editor! macro uses
// these inherent methods on MockTextEdit.

impl MockTextEdit {
    pub(crate) fn get_line_count(&self) -> i32 {
        self.lines.len() as i32
    }

    pub(crate) fn has_selection(&self) -> bool {
        self.carets[0].selection.active
    }

    /// Selection endpoints are sorted (from <= to) regardless of origin/caret order.
    pub(crate) fn get_selection_from_line(&self) -> i32 {
        self.selection_from_line(0)
    }

    pub(crate) fn get_selection_from_column(&self) -> i32 {
        self.selection_from_col(0)
    }

    pub(crate) fn get_selection_to_line(&self) -> i32 {
        self.selection_to_line(0)
    }

    pub(crate) fn get_selection_to_column(&self) -> i32 {
        self.selection_to_col(0)
    }
}

// ─── Private selection helpers ──────────────────────────────────────────────
// Godot's selection model uses origin + caret, NOT anchor + head. The "from"
// and "to" methods sort these into document order (from <= to) for deletion
// and assertion purposes.

impl MockTextEdit {
    fn selection_from_line(&self, idx: usize) -> i32 {
        let c = &self.carets[idx];
        if !c.selection.active {
            return c.line;
        }
        c.selection.origin_line.min(c.line)
    }

    fn selection_from_col(&self, idx: usize) -> i32 {
        let c = &self.carets[idx];
        if !c.selection.active {
            return c.column;
        }
        if c.selection.origin_line < c.line {
            c.selection.origin_column
        } else if c.selection.origin_line > c.line {
            c.column
        } else {
            c.selection.origin_column.min(c.column)
        }
    }

    fn selection_to_line(&self, idx: usize) -> i32 {
        let c = &self.carets[idx];
        if !c.selection.active {
            return c.line;
        }
        c.selection.origin_line.max(c.line)
    }

    fn selection_to_col(&self, idx: usize) -> i32 {
        let c = &self.carets[idx];
        if !c.selection.active {
            return c.column;
        }
        if c.selection.origin_line < c.line {
            c.column
        } else if c.selection.origin_line > c.line {
            c.selection.origin_column
        } else {
            c.selection.origin_column.max(c.column)
        }
    }

    /// Godot auto-deactivates selections that collapse to zero width (origin == caret).
    /// Called after set_caret_line/column to mirror this behavior.
    fn maybe_deselect(&mut self, idx: usize) {
        let c = &self.carets[idx];
        if c.selection.active
            && c.selection.origin_line == c.line
            && c.selection.origin_column == c.column
        {
            self.carets[idx].selection.active = false;
        }
    }
}

// ─── Internal unit tests ────────────────────────────────────────────────────
// Tests for private methods (base_insert_text, base_remove_text, caret offset)
// that cannot be reached from bridge_tests/ which only sees pub(crate) API.

#[cfg(test)]
mod tests {
    use super::*;

    // ── base_insert_text ──────────────────────────────────────────────────

    #[test]
    fn base_insert_at_beginning() {
        let mut mock = MockTextEdit::new("hello");
        let (end_line, end_col) = mock.base_insert_text(0, 0, "abc");
        assert_eq!(mock.get_text(), "abchello");
        assert_eq!(end_line, 0);
        assert_eq!(end_col, 3);
    }

    #[test]
    fn base_insert_at_end() {
        let mut mock = MockTextEdit::new("hello");
        let (end_line, end_col) = mock.base_insert_text(0, 5, " world");
        assert_eq!(mock.get_text(), "hello world");
        assert_eq!(end_line, 0);
        assert_eq!(end_col, 11);
    }

    #[test]
    fn base_insert_newline() {
        let mut mock = MockTextEdit::new("helloworld");
        let (end_line, end_col) = mock.base_insert_text(0, 5, "\n");
        assert_eq!(mock.get_text(), "hello\nworld");
        assert_eq!(end_line, 1);
        assert_eq!(end_col, 0);
    }

    #[test]
    fn base_insert_multi_line() {
        let mut mock = MockTextEdit::new("ac");
        let (end_line, end_col) = mock.base_insert_text(0, 1, "1\n2\n3");
        assert_eq!(mock.get_text(), "a1\n2\n3c");
        assert_eq!(end_line, 2);
        assert_eq!(end_col, 1);
    }

    // ── base_remove_text ──────────────────────────────────────────────────

    #[test]
    fn base_remove_single_line() {
        let mut mock = MockTextEdit::new("hello world");
        let removed = mock.base_remove_text(0, 5, 0, 11);
        assert_eq!(mock.get_text(), "hello");
        assert_eq!(removed, " world");
    }

    #[test]
    fn base_remove_across_lines() {
        let mut mock = MockTextEdit::new("hello\nworld");
        let removed = mock.base_remove_text(0, 3, 1, 2);
        assert_eq!(mock.get_text(), "helrld");
        assert_eq!(removed, "lo\nwo");
    }

    #[test]
    fn base_remove_entire_line() {
        let mut mock = MockTextEdit::new("aaa\nbbb\nccc");
        let removed = mock.base_remove_text(0, 3, 1, 3);
        assert_eq!(mock.get_text(), "aaa\nccc");
        assert_eq!(removed, "\nbbb");
    }

    // ── select_for_caret (accesses private caret fields) ──────────────────

    #[test]
    fn select_for_secondary_caret() {
        let mut mock = MockTextEdit::new("abcd\nefgh");
        let idx = mock.add_caret(1, 0);
        mock.select_for_caret(
            crate::types::CharLineCol::new(1, 1),
            crate::types::CharLineCol::new(1, 3),
            idx,
        );
        assert_eq!(mock.get_caret_line(), 0);
        assert!(mock.carets[idx as usize].selection.active);
    }

    // ── caret offset (accesses private insert_text_record + caret fields) ─

    #[test]
    fn caret_offset_after_multiline_insert() {
        let mut mock = MockTextEdit::new("aaa\nbbb");
        mock.set_caret_line(0);
        mock.set_caret_column(3);
        let idx = mock.add_caret(1, 3);
        assert_eq!(idx, 1);

        // Insert newline at end of line 0 — secondary caret on line 1 must shift down.
        mock.set_caret_column(3);
        mock.begin_complex_operation();
        mock.insert_text_record(0, 3, "\nXXX");
        mock.offset_carets_after(0, 3, 1, 3);
        mock.end_complex_operation();

        assert_eq!(mock.get_text(), "aaa\nXXX\nbbb");
        assert_eq!(mock.carets[1].line, 2);
    }
}
