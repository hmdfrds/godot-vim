//! Pure-Rust mock of Godot's TextEdit/CodeEdit for testing the bridge layer
//! without the Godot runtime.
//!
//! Replicates behavioral semantics from Godot's `text_edit.cpp`: text storage
//! (`Vec<String>`), caret model with `last_fit_x`, origin-based selection,
//! multi-caret, group-based undo with caret snapshots, and the viewport in
//! screen rows with folds and wrap. This lets bridge effect handlers run
//! against a real text model in `cargo test` without requiring a Godot
//! editor instance or GDExtension linkage.
//!
//! ## Undo model
//!
//! Godot's own history (`text_edit.cpp`, "Versioning"): one op per text
//! change with `chain_forward`/`chain_backward` marking complex operations,
//! the version counter, edit actions, and `undo()`/`redo()` with their caret
//! restore and unhiding. `u` and `<C-r>` never use it: they go through the
//! changeset-based `UndoStore`.
//!
//! Reference: `/reference/godot/scene/gui/text_edit.{h,cpp}`

use std::collections::{BTreeMap, BTreeSet};

use crate::bridge::port::{FoldCapable, TextEditorPort};

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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum OpKind {
    None,
    Insert,
    Remove,
}

/// Godot's `TextOperation`.
#[derive(Clone, Debug)]
struct TextOp {
    kind: OpKind,
    from: (i32, i32),
    to: (i32, i32),
    text: String,
    version: u32,
    prev_version: u32,
    chain_forward: bool,
    chain_backward: bool,
    start_carets: Vec<Caret>,
    end_carets: Vec<Caret>,
}

impl TextOp {
    fn none() -> Self {
        Self {
            kind: OpKind::None,
            from: (0, 0),
            to: (0, 0),
            text: String::new(),
            version: 0,
            prev_version: 0,
            chain_forward: false,
            chain_backward: false,
            start_carets: Vec::new(),
            end_carets: Vec::new(),
        }
    }
}

/// Godot's `TextEdit::EditAction`, the two the mock needs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum EditAction {
    None,
    Typing,
}

/// A caret's `(line, column, selection origin if active)`.
pub(crate) type CaretReading = (i32, i32, Option<(i32, i32)>);

/// `gui/common/text_edit_undo_stack_max_size`'s default.
const UNDO_STACK_MAX: usize = 1024;

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

    // ── Undo state, as Godot's fields ──
    undo_stack: Vec<TextOp>,
    /// Godot's `undo_stack_pos`: the first op of the redo area, `None` when
    /// there is nothing to redo.
    undo_pos: Option<usize>,
    /// The newest op, pushed onto the stack by the next `push_current_op`.
    current_op: TextOp,
    version: u32,
    complex_operation_count: u32,
    next_operation_is_complex: bool,
    current_action: EditAction,
    in_action: bool,
    pending_action_end: bool,
    /// `version` at the outermost `begin_complex_operation`.
    pair_start_version: u32,
    empty_complex_pairs: usize,
    godot_action_closes: usize,

    /// Multicaret edit nesting depth. `merge_overlapping_carets` is deferred
    /// to the outermost `end_multicaret_edit`.
    multicaret_edit_count: u32,

    // ── Viewport, in screen rows: a line inside a closed fold takes none, a
    // wrapped line one per wrap. No minimap, no scroll past the end. ──
    /// The row at the top, as Godot's `v_scroll`.
    v_scroll: f64,
    h_scroll: i32,
    visible_line_count: i32,
    /// Fold header -> last line it hides, for each fold a test declares.
    fold_regions: BTreeMap<i32, i32>,
    folded: BTreeSet<i32>,
    /// Characters per screen row; 0 does not wrap.
    wrap_width: i32,

    /// Completion popup calls, in order, and what the popup and caret are on.
    pub(crate) completion_log: Vec<String>,
    pub(crate) popup_open: bool,
    pub(crate) popup_paths: bool,
    pub(crate) in_string: bool,
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
            undo_pos: None,
            current_op: TextOp::none(),
            version: 0,
            complex_operation_count: 0,
            next_operation_is_complex: false,
            current_action: EditAction::None,
            in_action: false,
            pending_action_end: false,
            pair_start_version: 0,
            empty_complex_pairs: 0,
            godot_action_closes: 0,
            multicaret_edit_count: 0,
            v_scroll: 0.0,
            h_scroll: 0,
            visible_line_count: 25,
            fold_regions: BTreeMap::new(),
            folded: BTreeSet::new(),
            wrap_width: 0,
            completion_log: Vec::new(),
            popup_open: false,
            popup_paths: false,
            in_string: false,
        }
    }

    pub(crate) fn caret_count(&self) -> usize {
        self.carets.len()
    }

    pub(crate) fn set_visible_line_count(&mut self, count: i32) {
        self.visible_line_count = count;
    }

    /// A fold at `header` hiding `header + 1..=last`, as Godot finds from
    /// indentation or `#region`. Edits do not move it.
    pub(crate) fn add_fold_region(&mut self, header: i32, last: i32) {
        self.fold_regions.insert(header, last);
    }

    pub(crate) fn set_wrap_width(&mut self, cols: i32) {
        self.wrap_width = cols;
    }

    pub(crate) fn set_v_scroll(&mut self, value: f64) {
        let max = (self.total_rows() - self.visible_line_count).max(0);
        self.v_scroll = value.clamp(0.0, f64::from(max));
    }

    // ── Rows (text_edit.cpp) ─────────────────────────────────────────────

    /// CodeEdit enables hiding with code folding; the mock once a test
    /// declares a fold. Godot's fast paths apply without it.
    fn hiding_enabled(&self) -> bool {
        !self.fold_regions.is_empty()
    }

    fn last_line(&self) -> i32 {
        self.lines.len() as i32 - 1
    }

    fn is_line_hidden(&self, line: i32) -> bool {
        self.folded
            .iter()
            .any(|h| line > *h && line <= self.fold_regions[h])
    }

    fn line_wrap_count(&self, line: i32) -> i32 {
        if self.wrap_width <= 0 {
            return 0;
        }
        let len = self.lines[line as usize].len() as i32;
        (len.max(1) - 1) / self.wrap_width
    }

    fn caret_wrap_index(&self, idx: usize) -> i32 {
        let caret = &self.carets[idx];
        if self.wrap_width <= 0 {
            return 0;
        }
        (caret.column / self.wrap_width).min(self.line_wrap_count(caret.line))
    }

    /// `get_visible_line_count_in_range`, inclusive.
    fn rows_in(&self, from: i32, to: i32) -> i32 {
        (from..=to)
            .filter(|l| !self.is_line_hidden(*l))
            .map(|l| 1 + self.line_wrap_count(l))
            .sum()
    }

    fn total_rows(&self) -> i32 {
        self.rows_in(0, self.last_line())
    }

    fn scroll_pos_for_line(&self, line: i32, wrap: i32) -> f64 {
        debug_assert!((0..=self.line_wrap_count(line)).contains(&wrap));
        f64::from(self.rows_in(0, line - 1) + wrap)
    }

    /// `_scroll_moved`: the line and wrap row at the top.
    fn top(&self) -> (i32, i32) {
        let v = self.v_scroll.floor() as i32;
        let mut rows = 0;
        let mut line = 0;
        while line <= self.last_line() {
            if !self.is_line_hidden(line) {
                rows += 1 + self.line_wrap_count(line);
                if rows > v {
                    break;
                }
            }
            line += 1;
        }
        let line = line.min(self.last_line());
        let wraps = self.line_wrap_count(line);
        (line, (wraps - (rows - v - 1)).clamp(0, wraps))
    }

    /// `get_next_visible_line_index_offset_from`: lines walked, inclusive,
    /// and the wrap row reached.
    fn next_visible_row_offset(&self, line: i32, wrap: i32, amount: i32) -> (i32, i32) {
        if !self.hiding_enabled() && self.wrap_width <= 0 {
            return (amount.abs(), 0);
        }
        if amount == 0 {
            return (0, 0);
        }
        let count = self.lines.len() as i32;
        let mut total = 0;
        let wrap_index = if amount > 0 {
            let mut visible = -wrap;
            let mut i = line;
            while i < count {
                total += 1;
                if !self.is_line_hidden(i) {
                    visible += 1 + self.line_wrap_count(i);
                }
                if visible >= amount {
                    break;
                }
                i += 1;
            }
            let mut w = self.line_wrap_count(i.min(count - 1)) - (visible - amount).max(0);
            let end = line + total - 1;
            if self.is_line_hidden(end) {
                let back = self.next_visible_row_offset(end, 0, -1);
                total -= back.0 - 1;
                w = back.1;
            }
            w
        } else {
            let mut visible = wrap - self.line_wrap_count(line);
            let mut i = line;
            while i >= 0 {
                total += 1;
                if !self.is_line_hidden(i) {
                    visible += 1 + self.line_wrap_count(i);
                }
                if visible >= -amount {
                    break;
                }
                i -= 1;
            }
            visible + amount
        };
        (total, wrap_index.max(0))
    }

    fn last_full_visible(&self) -> (i32, i32) {
        let (first, wrap) = self.top();
        let (walked, last_wrap) =
            self.next_visible_row_offset(first, wrap, self.visible_line_count);
        ((first + walked - 1).clamp(0, self.last_line()), last_wrap)
    }

    /// As Godot 4.6+, which leaves the row above a sliver at the top (the
    /// 0.999 of `_get_visible_lines_offset`); 4.5 rounds it away.
    fn set_line_as_last_visible(&mut self, line: i32, wrap: i32) {
        let rows = self.visible_line_count;
        let (walked, first_wrap) = self.next_visible_row_offset(line, wrap, -rows - 1);
        if self.rows_in(0, line) < rows + 1 {
            self.set_v_scroll(0.0);
            return;
        }
        self.set_v_scroll(self.scroll_pos_for_line(line - walked + 1, first_wrap) + 0.999);
    }

    /// Godot keeps the top line when folds change, and a fold collapses the
    /// carets inside it onto the end of its header.
    fn refold(&mut self, change: impl FnOnce(&mut Self)) {
        let (top, wrap) = self.top();
        change(self);
        let folds: Vec<(i32, i32)> = self
            .folded
            .iter()
            .map(|h| (*h, self.fold_regions[h]))
            .collect();
        for caret in &mut self.carets {
            if let Some((header, _)) = folds
                .iter()
                .find(|(h, last)| caret.line > *h && caret.line <= *last)
            {
                caret.line = *header;
                caret.column = self.lines[*header as usize].len() as i32;
                caret.selection.active = false;
            }
        }
        self.set_line_as_first_visible(top, wrap);
    }

    fn merge_overlapping_carets(&mut self) {
        let mut i = 0;
        while i < self.carets.len() {
            let mut j = i + 1;
            while j < self.carets.len() {
                if self.carets[i].line == self.carets[j].line
                    && self.carets[i].column == self.carets[j].column
                {
                    self.carets.remove(j);
                } else {
                    j += 1;
                }
            }
            i += 1;
        }
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
    // Godot's `_insert_text` and `_remove_text`.

    fn insert_text_record(&mut self, p_line: i32, p_col: i32, p_text: &str) -> (i32, i32) {
        self.clear_redo();
        let (end_line, end_col) = self.base_insert_text(p_line, p_col, p_text);
        self.record_op(OpKind::Insert, (p_line, p_col), (end_line, end_col), p_text);
        (end_line, end_col)
    }

    fn remove_text_record(&mut self, from_line: i32, from_col: i32, to_line: i32, to_col: i32) {
        self.clear_redo();
        let removed = self.base_remove_text(from_line, from_col, to_line, to_col);
        self.record_op(
            OpKind::Remove,
            (from_line, from_col),
            (to_line, to_col),
            &removed,
        );
    }

    fn record_op(&mut self, kind: OpKind, from: (i32, i32), to: (i32, i32), text: &str) {
        self.version += 1;
        let start_carets = if self.next_operation_is_complex {
            self.current_op.start_carets.clone()
        } else {
            self.carets.clone()
        };
        let op = TextOp {
            kind,
            from,
            to,
            text: text.to_owned(),
            version: self.version,
            prev_version: self.current_op.version,
            chain_forward: false,
            chain_backward: false,
            start_carets,
            end_carets: self.carets.clone(),
        };
        self.push_current_op();
        self.current_op = op;
    }

    /// Godot's `_clear_redo`.
    fn clear_redo(&mut self) {
        let Some(pos) = self.undo_pos else {
            return;
        };
        self.push_current_op();
        self.undo_stack.truncate(pos);
        self.undo_pos = None;
    }

    /// Godot's `_push_current_op`.
    fn push_current_op(&mut self) {
        if self.pending_action_end {
            self.start_action(EditAction::None);
            return;
        }
        if self.current_op.kind == OpKind::None {
            return;
        }
        if self.next_operation_is_complex {
            self.current_op.chain_forward = true;
            self.next_operation_is_complex = false;
        }
        self.undo_stack.push(self.current_op.clone());
        self.current_op.kind = OpKind::None;
        self.current_op.text.clear();
        self.current_op.chain_forward = false;
        if self.undo_stack.len() > UNDO_STACK_MAX {
            self.undo_stack.remove(0);
        }
    }

    /// Godot's `_do_text_op`.
    fn do_text_op(&mut self, idx: usize, reverse: bool) {
        let op = &self.undo_stack[idx];
        let (from, to, text) = (op.from, op.to, op.text.clone());
        if (op.kind == OpKind::Insert) != reverse {
            self.base_insert_text(from.0, from.1, &text);
        } else {
            self.base_remove_text(from.0, from.1, to.0, to.1);
        }
    }

    /// CodeEdit's `_unhide_carets`.
    fn unhide_carets(&mut self) {
        for i in 0..self.carets.len() {
            let line = self.carets[i].line;
            if self.is_line_hidden(line) {
                FoldCapable::unfold_line(self, line);
            }
            let origin = self.carets[i].selection.origin_line;
            if self.carets[i].selection.active && self.is_line_hidden(origin) {
                FoldCapable::unfold_line(self, origin);
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

    fn insert_text(&mut self, text: &str, line: i32, col: i32) {
        self.begin_complex_operation();
        let (new_line, new_col) = self.insert_text_record(line, col, text);
        self.offset_carets_after(line, col, new_line, new_col);
        self.end_complex_operation();
    }

    fn remove_text(&mut self, from_line: i32, from_col: i32, to_line: i32, to_col: i32) {
        self.begin_complex_operation();
        self.remove_text_record(from_line, from_col, to_line, to_col);
        self.offset_carets_after(to_line, to_col, from_line, from_col);
        self.end_complex_operation();
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

    /// `set_caret_line` with `can_be_hidden = false`: a line inside a closed
    /// fold gives the first visible line below it, else the one above.
    fn set_caret_line_unfold(&mut self, line: i32, viewport: crate::bridge::port::ViewportAdjust) {
        let mut line = self.clamp_line(line);
        if self.is_line_hidden(line) {
            let down = self.get_next_visible_line_offset_from(line, 1) - 1;
            let up = self.get_next_visible_line_offset_from(line, -1) - 1;
            if line + down <= self.last_line() && !self.is_line_hidden(line + down) {
                line += down;
            } else if line - up > 0 && !self.is_line_hidden(line - up) {
                line -= up;
            }
        }
        self.set_caret_line(line);
        if viewport == crate::bridge::port::ViewportAdjust::Adjust {
            self.adjust_viewport_to_caret();
        }
    }

    fn adjust_viewport_to_caret(&mut self) {
        let line = self.carets[0].line;
        let wrap = self.caret_wrap_index(0);
        let (first, first_wrap) = self.top();
        let (last, last_wrap) = self.last_full_visible();
        if line < first || (line == first && wrap < first_wrap) {
            self.set_line_as_first_visible(line, wrap);
        } else if line > last || (line == last && wrap > last_wrap) {
            self.set_line_as_last_visible(line, wrap);
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

    fn remove_caret(&mut self, caret_idx: i32) {
        let idx = caret_idx as usize;
        if idx < self.carets.len() {
            self.carets.remove(idx);
        }
    }

    fn remove_secondary_carets(&mut self) {
        self.carets.truncate(1);
    }

    fn get_caret_count(&self) -> i32 {
        self.carets.len() as i32
    }

    fn get_caret_line_for(&self, caret_idx: i32) -> i32 {
        let idx = caret_idx as usize;
        if idx < self.carets.len() {
            self.carets[idx].line
        } else {
            0
        }
    }

    fn get_caret_column_for(&self, caret_idx: i32) -> i32 {
        let idx = caret_idx as usize;
        if idx < self.carets.len() {
            self.carets[idx].column
        } else {
            0
        }
    }

    fn set_caret_line_for(&mut self, line: i32, caret_idx: i32) {
        let idx = caret_idx as usize;
        if idx < self.carets.len() {
            let clamped = self.clamp_line(line);
            self.carets[idx].line = clamped;
            let col = self.carets[idx].column;
            self.carets[idx].column = self.clamp_column(clamped, col);
        }
    }

    fn set_caret_column_for(&mut self, col: i32, caret_idx: i32) {
        let idx = caret_idx as usize;
        if idx < self.carets.len() {
            let line = self.carets[idx].line;
            let clamped = self.clamp_column(line, col);
            self.carets[idx].column = clamped;
            self.carets[idx].last_fit_x = clamped;
        }
    }

    // ── Undo ────────────────────────────────────────────────────────────

    fn begin_complex_operation(&mut self) {
        self.push_current_op();
        if self.complex_operation_count == 0 {
            self.next_operation_is_complex = true;
            self.current_op.start_carets = self.carets.clone();
            self.pair_start_version = self.version;
        }
        self.complex_operation_count += 1;
    }

    /// An outermost end with no op since its begin marks the previous op
    /// `chain_backward`, so Godot's next undo also takes the step before it.
    fn end_complex_operation(&mut self) {
        self.push_current_op();
        let outermost = self.complex_operation_count <= 1;
        let empty = self.complex_operation_count == 0 || self.version == self.pair_start_version;
        self.complex_operation_count = self.complex_operation_count.saturating_sub(1);
        if !outermost {
            return;
        }
        if empty {
            self.empty_complex_pairs += 1;
        }
        let carets = self.carets.clone();
        let Some(last) = self.undo_stack.last_mut() else {
            return;
        };
        last.end_carets = carets;
        if last.chain_forward {
            last.chain_forward = false;
            return;
        }
        last.chain_backward = true;
    }

    fn close_godot_action(&mut self) {
        self.godot_action_closes += 1;
        self.start_action(EditAction::None);
    }

    fn begin_multicaret_edit(&mut self) {
        self.multicaret_edit_count += 1;
    }

    fn end_multicaret_edit(&mut self) {
        if self.multicaret_edit_count > 0 {
            self.multicaret_edit_count -= 1;
        }
        if self.multicaret_edit_count == 0 {
            self.merge_overlapping_carets();
        }
    }

    // ── Scroll ──────────────────────────────────────────────────────────

    fn get_v_scroll(&self) -> f64 {
        self.v_scroll
    }

    fn get_first_visible_line(&self) -> i32 {
        self.top().0
    }

    fn get_visible_line_count(&self) -> i32 {
        self.visible_line_count
    }

    fn get_caret_wrap_index(&self) -> i32 {
        self.caret_wrap_index(0)
    }

    fn get_line_wrap_count(&self, line: i32) -> i32 {
        self.line_wrap_count(line)
    }

    fn get_scroll_pos_for_line(&self, line: i32, wrap: i32) -> f64 {
        self.scroll_pos_for_line(line, wrap)
    }

    fn get_next_visible_line_index_offset_from(
        &self,
        line: i32,
        wrap: i32,
        rows: i32,
    ) -> (i32, i32) {
        self.next_visible_row_offset(line, wrap, rows)
    }

    fn set_line_as_first_visible(&mut self, line: i32, wrap: i32) {
        self.set_v_scroll(self.scroll_pos_for_line(line, wrap));
    }

    fn set_h_scroll(&mut self, value: i32) {
        self.h_scroll = value.max(0);
    }

    fn get_h_scroll(&self) -> i32 {
        self.h_scroll
    }

    fn get_next_visible_line_offset_from(&self, line: i32, visible_amount: i32) -> i32 {
        if !self.hiding_enabled() {
            return visible_amount.abs();
        }
        let step = if visible_amount >= 0 { 1 } else { -1 };
        let (mut visible, mut total, mut i) = (0, 0, line);
        while (0..=self.last_line()).contains(&i) {
            total += 1;
            if !self.is_line_hidden(i) {
                visible += 1;
            }
            if visible >= visible_amount.abs() {
                break;
            }
            i += step;
        }
        total
    }
}

// ─── Extension trait implementations ────────────────────────────────────────
// Folds act on the regions a test declares. Navigation is a default no-op.
// The completion popup is a recorder: no candidates, only the order of calls.

use crate::bridge::port::{IdeCapable, NavigationCapable};

impl FoldCapable for MockTextEdit {
    fn fold_line(&mut self, line: i32) {
        if self.fold_regions.contains_key(&line) {
            self.refold(|m| {
                m.folded.insert(line);
            });
        }
    }

    /// The fold at `line`, or the one hiding it.
    fn unfold_line(&mut self, line: i32) {
        let regions = self.fold_regions.clone();
        self.refold(|m| {
            m.folded.retain(|h| !(line >= *h && line <= regions[h]));
        });
    }

    fn is_line_folded(&self, line: i32) -> bool {
        line < self.last_line() && !self.is_line_hidden(line) && self.is_line_hidden(line + 1)
    }

    fn toggle_foldable_line(&mut self, line: i32) {
        if self.folded.contains(&line) {
            self.unfold_line(line);
        } else {
            self.fold_line(line);
        }
    }

    fn fold_all_lines(&mut self) {
        self.refold(|m| m.folded = m.fold_regions.keys().copied().collect());
    }

    fn unfold_all_lines(&mut self) {
        self.refold(|m| m.folded.clear());
    }
}

impl IdeCapable for MockTextEdit {
    fn cancel_code_completion(&mut self) {
        self.completion_log.push("cancel".into());
        self.popup_open = false;
    }

    fn completion_popup_open(&self) -> bool {
        self.popup_open
    }

    fn completion_popup_holds_paths(&self) -> bool {
        self.popup_open && self.popup_paths
    }

    fn caret_in_string(&self) -> bool {
        self.in_string
    }

    fn is_completion_prefix(&self, ch: char) -> bool {
        matches!(ch, '.' | ',' | '(' | '=' | '$' | '@' | '"' | '\'')
    }

    fn request_code_completion(&mut self, force: bool) {
        self.completion_log.push(format!("request(force={force})"));
    }

    fn arm_code_completion(&mut self) {
        self.completion_log.push("arm".into());
    }
}
impl NavigationCapable for MockTextEdit {}

// ─── Godot's undo API ──────────────────────────────────────────────────────

impl MockTextEdit {
    /// `TextEdit::start_action`.
    pub(crate) fn start_action(&mut self, action: EditAction) {
        if self.current_action != action {
            if self.current_action != EditAction::None {
                self.in_action = false;
                self.pending_action_end = false;
                self.end_complex_operation();
            }
            if action != EditAction::None {
                self.in_action = true;
                self.begin_complex_operation();
            }
        } else if self.current_action != EditAction::None {
            self.pending_action_end = false;
        }
        self.current_action = action;
    }

    /// `TextEdit::undo`, as Ctrl+Z and Edit > Undo call it.
    pub(crate) fn undo(&mut self) {
        if self.in_action {
            self.pending_action_end = true;
        }
        self.push_current_op();
        let mut pos = match self.undo_pos {
            None if self.undo_stack.is_empty() => return,
            None => self.undo_stack.len() - 1,
            Some(0) => return,
            Some(p) => p - 1,
        };
        self.deselect();
        self.do_text_op(pos, true);
        self.current_op.version = self.undo_stack[pos].prev_version;
        if self.undo_stack[pos].chain_backward {
            // Godot's ERR_BREAK at the front of the stack.
            while pos > 0 {
                pos -= 1;
                self.do_text_op(pos, true);
                self.current_op.version = self.undo_stack[pos].prev_version;
                if self.undo_stack[pos].chain_forward {
                    break;
                }
            }
        }
        self.undo_pos = Some(pos);
        self.carets = self.undo_stack[pos].start_carets.clone();
        self.unhide_carets();
        self.adjust_viewport_to_caret();
    }

    /// `TextEdit::redo`.
    pub(crate) fn redo(&mut self) {
        if self.in_action {
            self.pending_action_end = true;
        }
        self.push_current_op();
        let Some(mut pos) = self.undo_pos else {
            return;
        };
        self.deselect();
        self.do_text_op(pos, false);
        self.current_op.version = self.undo_stack[pos].version;
        if self.undo_stack[pos].chain_forward {
            while pos + 1 < self.undo_stack.len() {
                pos += 1;
                self.do_text_op(pos, false);
                self.current_op.version = self.undo_stack[pos].version;
                if self.undo_stack[pos].chain_backward {
                    break;
                }
            }
        }
        self.carets = self.undo_stack[pos].end_carets.clone();
        self.undo_pos = (pos + 1 < self.undo_stack.len()).then_some(pos + 1);
        self.unhide_carets();
        self.adjust_viewport_to_caret();
    }

    pub(crate) fn has_undo(&self) -> bool {
        match self.undo_pos {
            None => !self.undo_stack.is_empty() || self.current_op.kind != OpKind::None,
            Some(pos) => pos != 0,
        }
    }

    pub(crate) fn has_redo(&self) -> bool {
        self.undo_pos.is_some()
    }

    pub(crate) fn get_version(&self) -> u32 {
        self.current_op.version
    }

    /// Outermost complex operations that closed with no op inside.
    pub(crate) fn empty_complex_pairs(&self) -> usize {
        self.empty_complex_pairs
    }

    /// Calls to `close_godot_action`.
    pub(crate) fn godot_action_closes(&self) -> usize {
        self.godot_action_closes
    }

    /// Every caret with its selection origin, if a selection is active.
    pub(crate) fn carets_with_selection(&self) -> Vec<CaretReading> {
        self.carets
            .iter()
            .map(|c| {
                let origin = c
                    .selection
                    .active
                    .then_some((c.selection.origin_line, c.selection.origin_column));
                (c.line, c.column, origin)
            })
            .collect()
    }

    pub(crate) fn folded_lines(&self) -> Vec<i32> {
        self.folded.iter().copied().collect()
    }
}

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

    // ── Godot's undo history ──────────────────────────────────────────────

    /// One `x` as godot-vim commits it: one op in one complex operation.
    fn x_at(mock: &mut MockTextEdit, line: i32) {
        mock.begin_complex_operation();
        mock.remove_text(line, 0, line, 1);
        mock.end_complex_operation();
    }

    #[test]
    fn one_undo_takes_one_complex_operation() {
        let mut mock = MockTextEdit::new("abc\ndef");
        x_at(&mut mock, 0);
        mock.begin_complex_operation();
        mock.insert_text("1", 1, 0);
        mock.insert_text("2", 1, 0);
        mock.end_complex_operation();
        assert_eq!(mock.get_text(), "bc\n21def");
        assert_eq!(mock.get_version(), 3);

        mock.undo();
        assert_eq!(mock.get_text(), "bc\ndef");
        assert_eq!(mock.get_version(), 1);
        assert!(mock.has_undo() && mock.has_redo());
        mock.redo();
        assert_eq!(mock.get_text(), "bc\n21def");
        assert!(!mock.has_redo());
        assert_eq!(mock.empty_complex_pairs(), 0);
    }

    /// text_edit.cpp `end_complex_operation`: with no op since the begin,
    /// the previous op gets `chain_backward`, so `undo()` walks past it.
    #[test]
    fn empty_pair_chains_the_previous_op_backward() {
        let mut mock = MockTextEdit::new("abc\ndef\nghi");
        x_at(&mut mock, 0);
        x_at(&mut mock, 1);
        mock.begin_complex_operation();
        mock.end_complex_operation();
        assert_eq!(mock.empty_complex_pairs(), 1);

        mock.undo();
        assert_eq!(mock.get_text(), "abc\ndef\nghi", "both x undone");
        assert!(!mock.has_undo());
    }

    #[test]
    fn undo_restores_the_start_carets_with_their_selection() {
        let mut mock = MockTextEdit::new("abc\ndef");
        mock.select(
            crate::types::CharLineCol::new(0, 0),
            crate::types::CharLineCol::new(0, 2),
        );
        mock.begin_complex_operation();
        mock.remove_text(0, 0, 0, 2);
        mock.end_complex_operation();
        mock.deselect();
        mock.set_caret_line(1);

        mock.undo();
        assert_eq!(mock.get_text(), "abc\ndef");
        assert_eq!(mock.carets_with_selection(), vec![(0, 2, Some((0, 0)))]);
    }

    /// CodeEdit `_unhide_carets`: a restored caret inside a fold opens it.
    #[test]
    fn undo_unfolds_a_restored_caret() {
        let mut mock = MockTextEdit::new("func a():\n\tx\n\ty\nfunc b():");
        mock.add_fold_region(0, 2);
        mock.set_caret_line(1);
        x_at(&mut mock, 1);
        mock.set_caret_line(0);
        mock.fold_line(0);
        assert_eq!(mock.folded_lines(), vec![0]);

        mock.undo();
        assert_eq!(mock.folded_lines(), Vec::<i32>::new());
        assert_eq!(mock.get_caret_line(), 1);
    }

    /// Godot's typing is one action until `start_action(NONE)` or another
    /// action closes it; `undo()` inside it closes it first.
    #[test]
    fn a_typing_action_is_one_undo_step() {
        let mut mock = MockTextEdit::new("");
        mock.start_action(EditAction::Typing);
        mock.insert_text("a", 0, 0);
        mock.insert_text("b", 0, 1);
        assert!(mock.has_undo());
        mock.start_action(EditAction::None);
        mock.start_action(EditAction::Typing);
        mock.insert_text("c", 0, 2);
        mock.undo();
        assert_eq!(mock.get_text(), "ab");
        mock.undo();
        assert_eq!(mock.get_text(), "");
        assert_eq!(mock.empty_complex_pairs(), 0);
    }
}
