//! Shared test infrastructure for bridge effect tests.
//!
//! - `assert_editor!` — multi-field assertion macro for MockTextEdit state.
//!   Accepts any combination of text, cursor, selection, scroll, etc. checks
//!   in a single call, producing clear mismatch messages per field.
//! - `effects!` — DSL macro that builds `Vec<Effect>` from a concise
//!   semicolon-separated list, hiding the verbose enum constructors.
//! - `DispatchCtx` — owns the `ShellState` + `InstanceId` + `UndoDepth` that
//!   the full dispatch pipeline requires, isolating each test from global state.
//! - `apply_*` helpers — thin wrappers that create a `DocumentView` from the
//!   mock's current text and call a single effect handler. These bypass the
//!   full dispatch pipeline to test individual handlers in isolation.

use crate::bridge::port::TextEditorPort;
use crate::testing::MockTextEdit;

// ── assert_editor! ──────────────────────────────────────────────────────────

/// Multi-field assertion macro for `MockTextEdit`. Verifies any combination of
/// editor state in a single call with clear per-field mismatch messages.
///
/// # Supported fields
///
/// | Syntax | Asserts |
/// |--------|---------|
/// | `text: expr` | `get_text() == expr` |
/// | `cursor: (line, col)` | caret line and column |
/// | `selection: (fl, fc) => (tl, tc)` | selection endpoints (implies has_selection) |
/// | `selection_cols: (fc, tc)` | selection columns only (implies has_selection) |
/// | `has_selection` | `has_selection() == true` |
/// | `no_selection` | `has_selection() == false` |
/// | `carets: n` | `caret_count() == n` |
/// | `line_count: n` | `get_line_count() == n` |
/// | `line(n): expr` | `get_line(n) == expr` |
/// | `scroll: n` | `get_first_visible_line() == n` |
/// | `h_scroll: n` | `get_h_scroll() == n` |
macro_rules! assert_editor {
    // Entry: append trailing comma to normalize, then dispatch through @step arms.
    ($mock:expr, $($fields:tt)+) => {
        assert_editor!(@step $mock, $($fields)+,);
    };
    ($mock:expr $(,)?) => {};

    (@step $mock:expr $(,)*) => {};

    // ── text ─────────────────────────────────────────────────────
    (@step $mock:expr, text: $expected:expr, $($rest:tt)*) => {
        assert_eq!($mock.get_text(), $expected, "text mismatch");
        assert_editor!(@step $mock, $($rest)*);
    };

    // ── cursor ───────────────────────────────────────────────────
    (@step $mock:expr, cursor: ($line:expr, $col:expr), $($rest:tt)*) => {
        assert_eq!($mock.get_caret_line(), $line, "caret line mismatch");
        assert_eq!($mock.get_caret_column(), $col, "caret column mismatch");
        assert_editor!(@step $mock, $($rest)*);
    };

    // ── selection (full: from => to) ─────────────────────────────
    (@step $mock:expr, selection: ($fl:expr, $fc:expr) => ($tl:expr, $tc:expr), $($rest:tt)*) => {
        assert!($mock.has_selection(), "expected active selection");
        assert_eq!($mock.get_selection_from_line(), $fl, "selection from_line mismatch");
        assert_eq!($mock.get_selection_from_column(), $fc, "selection from_col mismatch");
        assert_eq!($mock.get_selection_to_line(), $tl, "selection to_line mismatch");
        assert_eq!($mock.get_selection_to_column(), $tc, "selection to_col mismatch");
        assert_editor!(@step $mock, $($rest)*);
    };

    // ── selection_cols (columns only, ignores lines) ─────────────
    (@step $mock:expr, selection_cols: ($fc:expr, $tc:expr), $($rest:tt)*) => {
        assert!($mock.has_selection(), "expected active selection");
        assert_eq!($mock.get_selection_from_column(), $fc, "selection from_col mismatch");
        assert_eq!($mock.get_selection_to_column(), $tc, "selection to_col mismatch");
        assert_editor!(@step $mock, $($rest)*);
    };

    // ── has_selection / no_selection ──────────────────────────────
    (@step $mock:expr, has_selection, $($rest:tt)*) => {
        assert!($mock.has_selection(), "expected active selection");
        assert_editor!(@step $mock, $($rest)*);
    };
    (@step $mock:expr, no_selection, $($rest:tt)*) => {
        assert!(!$mock.has_selection(), "expected no selection");
        assert_editor!(@step $mock, $($rest)*);
    };

    // ── carets ───────────────────────────────────────────────────
    (@step $mock:expr, carets: $n:expr, $($rest:tt)*) => {
        assert_eq!($mock.caret_count(), $n, "caret count mismatch");
        assert_editor!(@step $mock, $($rest)*);
    };

    // ── line_count ───────────────────────────────────────────────
    (@step $mock:expr, line_count: $n:expr, $($rest:tt)*) => {
        assert_eq!($mock.get_line_count(), $n, "line count mismatch");
        assert_editor!(@step $mock, $($rest)*);
    };

    // ── line(n) ──────────────────────────────────────────────────
    (@step $mock:expr, line($n:expr): $expected:expr, $($rest:tt)*) => {
        assert_eq!($mock.get_line($n), $expected, "line {} mismatch", $n);
        assert_editor!(@step $mock, $($rest)*);
    };

    // ── scroll ───────────────────────────────────────────────────
    (@step $mock:expr, scroll: $line:expr, $($rest:tt)*) => {
        assert_eq!($mock.get_first_visible_line(), $line, "first visible line mismatch");
        assert_editor!(@step $mock, $($rest)*);
    };

    // ── h_scroll ─────────────────────────────────────────────────
    (@step $mock:expr, h_scroll: $val:expr, $($rest:tt)*) => {
        assert_eq!($mock.get_h_scroll(), $val, "h_scroll mismatch");
        assert_editor!(@step $mock, $($rest)*);
    };
}

// ── effects! ────────────────────────────────────────────────────────────────

/// DSL for building `Vec<Effect>` without verbose enum constructors.
/// Uses fully-qualified paths so call sites need no imports beyond `#[macro_use]`.
///
/// ```ignore
/// let effects = effects![
///     begin_undo;
///     delete(5, 6);
///     insert(5, "_");
///     set_cursor(6);
///     end_undo
/// ];
/// ```
macro_rules! effects {
    ($($name:ident $(($($args:tt)*))? );+ $(;)?) => {{
        let mut __v: Vec<vim_core::effects::Effect> = Vec::new();
        $( effects!(@one __v, $name $(($($args)*))?); )+
        __v
    }};

    // ── Undo group ───────────────────────────────────────────────
    (@one $v:ident, begin_undo) => {
        $v.push(vim_core::effects::Effect::BeginUndoGroup {
            cursor_strategy: vim_core::primitives::UndoCursorStrategy::FirstEdit,
        });
    };
    (@one $v:ident, begin_undo_force) => {
        $v.push(vim_core::effects::Effect::BeginUndoGroup {
            cursor_strategy: vim_core::primitives::UndoCursorStrategy::EntryPosition,
        });
    };
    (@one $v:ident, end_undo) => {
        $v.push(vim_core::effects::Effect::EndUndoGroup { node_id: None });
    };

    // ── Text mutations ───────────────────────────────────────────
    (@one $v:ident, insert($offset:expr, $text:expr)) => {
        $v.push(vim_core::effects::Effect::Insert {
            offset: vim_core::primitives::Offset::new($offset),
            text: ($text).into(),
        });
    };
    (@one $v:ident, delete($start:expr, $end:expr)) => {
        $v.push(vim_core::effects::Effect::Delete {
            range: vim_core::primitives::Range::from_raw($start, $end),
        });
    };
    (@one $v:ident, replace($start:expr, $end:expr, $text:expr)) => {
        $v.push(vim_core::effects::Effect::Replace {
            range: vim_core::primitives::Range::from_raw($start, $end),
            text: ($text).into(),
        });
    };

    // ── Cursor / selection ───────────────────────────────────────
    (@one $v:ident, set_cursor($offset:expr)) => {
        $v.push(vim_core::effects::Effect::SetCursor {
            offset: vim_core::primitives::Offset::new($offset),
        });
    };
    (@one $v:ident, set_selection($anchor:expr, $head:expr, $shape:expr)) => {
        $v.push(vim_core::effects::Effect::SetSelection {
            anchor: vim_core::primitives::Offset::new($anchor),
            head: vim_core::primitives::Offset::new($head),
            shape: $shape,
        });
    };
    (@one $v:ident, clear_selection) => {
        $v.push(vim_core::effects::Effect::ClearSelection);
    };

    // ── Undo / redo ──────────────────────────────────────────────
    (@one $v:ident, undo($count:expr)) => {
        $v.push(vim_core::effects::Effect::Undo { count: $count, steps: vec![] });
    };
    (@one $v:ident, redo($count:expr)) => {
        $v.push(vim_core::effects::Effect::Redo { count: $count, steps: vec![] });
    };

    // ── Scroll ───────────────────────────────────────────────────
    (@one $v:ident, center_cursor) => {
        $v.push(vim_core::effects::Effect::CenterCursor);
    };
    (@one $v:ident, cursor_to_top) => {
        $v.push(vim_core::effects::Effect::CursorToTop);
    };
    (@one $v:ident, cursor_to_bottom) => {
        $v.push(vim_core::effects::Effect::CursorToBottom);
    };
    (@one $v:ident, scroll_left($count:expr)) => {
        $v.push(vim_core::effects::Effect::ScrollLeft { count: $count });
    };
    (@one $v:ident, scroll_right($count:expr)) => {
        $v.push(vim_core::effects::Effect::ScrollRight { count: $count });
    };
    (@one $v:ident, scroll_to($offset:expr)) => {
        $v.push(vim_core::effects::Effect::ScrollTo {
            offset: vim_core::primitives::Offset::new($offset),
        });
    };

    // ── Folds ────────────────────────────────────────────────────
    (@one $v:ident, fold_line($line:expr)) => {
        $v.push(vim_core::effects::Effect::FoldLine {
            line: vim_core::primitives::LineNumber::new($line),
        });
    };
    (@one $v:ident, unfold_line($line:expr)) => {
        $v.push(vim_core::effects::Effect::UnfoldLine {
            line: vim_core::primitives::LineNumber::new($line),
        });
    };
    (@one $v:ident, toggle_fold($line:expr)) => {
        $v.push(vim_core::effects::Effect::ToggleFold {
            line: vim_core::primitives::LineNumber::new($line),
        });
    };
    (@one $v:ident, fold_all) => {
        $v.push(vim_core::effects::Effect::FoldAll);
    };
    (@one $v:ident, unfold_all) => {
        $v.push(vim_core::effects::Effect::UnfoldAll);
    };
}

// ── DispatchCtx ─────────────────────────────────────────────────────────────

/// Owns the per-test dispatch state so each test is fully isolated. In production,
/// ShellState and UndoDepth live in the controller; here they're scoped to one test.
pub(super) struct DispatchCtx {
    state: crate::state::ShellState,
    editor_id: godot::prelude::InstanceId,
    undo_depth: crate::effects::UndoDepth,
    clipboard: crate::bridge::clipboard::MockClipboard,
}

impl DispatchCtx {
    pub(super) fn new() -> Self {
        Self {
            state: crate::state::ShellState::default(),
            editor_id: godot::prelude::InstanceId::from_i64(1),
            undo_depth: crate::effects::UndoDepth::new(),
            clipboard: crate::bridge::clipboard::MockClipboard::new(),
        }
    }

    /// Run effects through the full bridge dispatch pipeline, including text
    /// cache invalidation, undo depth tracking, and selection/cursor interplay.
    /// This is the closest to production behavior achievable without Godot.
    pub(super) fn dispatch(
        &mut self,
        mock: &mut MockTextEdit,
        effects: Vec<vim_core::effects::Effect>,
    ) {
        let text = mock.get_text();
        crate::effects::dispatch(
            effects,
            mock,
            crate::effects::DispatchContext {
                state: &mut self.state,
                editor_id: self.editor_id,
                undo_depth: &mut self.undo_depth,
                auto_brace: crate::effects::dispatch::AutoBraceMode::Ineligible,
                auto_brace_snapshot: crate::bridge::AutoBraceSnapshot::disabled(),
                line_index_hint: None,
                scrolloff: 0,
                highlight_yank_duration_ms: 150,
                syntax_query: Box::new(|_, _| crate::bridge::SyntaxRegion::code()),
                clipboard: &mut self.clipboard,
            },
            &text,
        );
    }
}

// ── apply_* helpers ─────────────────────────────────────────────────────────
// These call individual effect handlers in isolation (bypassing full dispatch)
// to test the byte-offset-to-line/col codec and Godot API translation without
// the complexity of undo depth tracking, text cache invalidation, etc.
// Each helper snapshots the mock's current text into a DocumentView, which is
// the same pattern the production dispatch loop uses per-batch.

pub(super) fn apply_insert(mock: &mut MockTextEdit, offset: usize, content: &str) {
    let text = mock.get_text();
    let li = crate::bridge::codec::LineIndex::new(&text);
    let doc = crate::bridge::codec::DocumentView::new(&text, &li);
    crate::effects::text::handle_insert(mock, &doc, offset, content);
}

pub(super) fn apply_delete(mock: &mut MockTextEdit, start: usize, end: usize) {
    let text = mock.get_text();
    let li = crate::bridge::codec::LineIndex::new(&text);
    let doc = crate::bridge::codec::DocumentView::new(&text, &li);
    crate::effects::text::handle_delete(mock, &doc, start, end);
}

pub(super) fn apply_replace(mock: &mut MockTextEdit, start: usize, end: usize, content: &str) {
    let text = mock.get_text();
    let li = crate::bridge::codec::LineIndex::new(&text);
    let doc = crate::bridge::codec::DocumentView::new(&text, &li);
    crate::effects::text::handle_replace(mock, &doc, start, end, content);
}

pub(super) fn apply_set_cursor(mock: &mut MockTextEdit, offset: usize) {
    let text = mock.get_text();
    let li = crate::bridge::codec::LineIndex::new(&text);
    let doc = crate::bridge::codec::DocumentView::new(&text, &li);
    crate::effects::cursor::handle_set_cursor(mock, &doc, offset, 0);
}

pub(super) fn apply_set_selection(
    mock: &mut MockTextEdit,
    anchor: usize,
    head: usize,
    shape: vim_core::primitives::SelectionShape,
) {
    let text = mock.get_text();
    let li = crate::bridge::codec::LineIndex::new(&text);
    let doc = crate::bridge::codec::DocumentView::new(&text, &li);
    crate::effects::cursor::handle_set_selection(mock, &doc, anchor, head, shape);
}
