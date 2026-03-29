//! `TextEditorPort` implementation: `CodeEditPort` newtype over `Gd<CodeEdit>`.
//!
//! The newtype is necessary because implementing a trait directly on
//! `Gd<CodeEdit>` causes infinite recursion: trait method names shadow the
//! identically-named Godot-generated inherent methods, so `self.get_text()`
//! calls the trait method instead of the Godot FFI method. The newtype
//! breaks this ambiguity — `self.0.get_text()` always resolves to Godot.

use std::cell::RefCell;
use std::rc::Rc;

use godot::classes::CodeEdit;
use godot::prelude::*;

use super::port::{FoldCapable, IdeCapable, NavigationCapable, TextEditorPort, ViewportAdjust};

// Brace-pair cache: thread-local rather than a VimController field because
// TextEditorPort is intentionally stateless (enabling MockTextEdit in tests).
//
// Keyed by `InstanceId` (never recycled by Godot within a session).
// Invalidated on editor detach via `invalidate_brace_pair_cache`.
// Only two access paths: `AutoBraceSnapshot::from_editor` and `invalidate_brace_pair_cache`.
thread_local! {
    #[allow(clippy::type_complexity)]
    static BRACE_PAIR_CACHE: RefCell<Option<(InstanceId, Rc<Vec<(String, String)>>)>> =
        const { RefCell::new(None) };
}

pub(crate) fn invalidate_brace_pair_cache() {
    BRACE_PAIR_CACHE.with(|c| *c.borrow_mut() = None);
}

// ── Pre-dispatch snapshots ───────────────────────────────────────────────
//
// Captured once before effect dispatch to avoid repeated FFI round-trips.

/// Snapshot of auto-brace completion state captured from a `CodeEdit`.
///
/// Captured once per keystroke via `from_editor`. During effect dispatch,
/// auto-brace logic queries this snapshot instead of making FFI calls per
/// inserted character.
#[derive(Debug, Clone)]
pub(crate) struct AutoBraceSnapshot {
    pub(crate) enabled: bool,
    /// Sorted by open-key length descending (longest match wins). Shared via
    /// `Rc` to avoid cloning the vec on every insert-mode keystroke.
    pub(crate) pairs: Rc<Vec<(String, String)>>,
    /// String delimiter start keys (e.g. `"`, `'`), extracted from Godot's
    /// space-separated `"start_key end_key"` format. Used to suppress
    /// auto-brace insertion inside string literals.
    pub(crate) string_delimiters: Vec<String>,
}

impl AutoBraceSnapshot {
    /// Capture auto-brace state from the live editor (3 FFI calls, cached pairs).
    pub(crate) fn from_editor(editor: &Gd<CodeEdit>) -> Self {
        let enabled = editor.is_auto_brace_completion_enabled();

        let pairs = {
            let editor_id = editor.instance_id();
            BRACE_PAIR_CACHE.with(|cache| {
                let mut cache = cache.borrow_mut();
                if let Some((id, ref pairs)) = *cache {
                    if id == editor_id {
                        return Rc::clone(pairs);
                    }
                }
                let dict = editor.get_auto_brace_completion_pairs();
                let mut pairs: Vec<(String, String)> = dict
                    .iter_shared()
                    .map(|(k, v)| (k.to_string(), v.to_string()))
                    .collect();
                // Longest-match-first: e.g. `/*` must match before `*`.
                pairs.sort_by(|a, b| b.0.len().cmp(&a.0.len()));
                let rc = Rc::new(pairs);
                *cache = Some((editor_id, Rc::clone(&rc)));
                rc
            })
        };

        // Godot returns delimiters as "start_key end_key" (space-separated).
        // We only need the start key for `has_string_delimiter` checks.
        let string_delimiters = editor
            .get_string_delimiters()
            .iter_shared()
            .map(|entry| {
                let s = entry.to_string();
                match s.find(' ') {
                    Some(idx) => s[..idx].to_string(),
                    None => s,
                }
            })
            .collect();

        Self {
            enabled,
            pairs,
            string_delimiters,
        }
    }

    /// Empty snapshot for contexts without auto-brace (`:norm` execution, tests).
    pub(crate) fn disabled() -> Self {
        Self {
            enabled: false,
            pairs: Rc::new(Vec::new()),
            string_delimiters: Vec::new(),
        }
    }

    /// Check if `key` is a string delimiter start key (no FFI — answered from snapshot).
    pub(crate) fn has_string_delimiter(&self, key: &str) -> bool {
        self.string_delimiters.iter().any(|d| d == key)
    }
}

/// Syntax context (string/comment) at a cursor position, captured in 2 FFI calls.
///
/// Used to suppress auto-brace and other syntax-aware behaviors when the
/// cursor is inside a string literal or comment.
#[derive(Debug, Clone, Copy)]
pub(crate) struct SyntaxContext {
    pub(crate) is_in_string: bool,
    pub(crate) is_in_comment: bool,
}

impl SyntaxContext {
    /// Capture syntax context at `(line, col)` via Godot FFI.
    ///
    /// Godot's `is_in_string_ex`/`is_in_comment_ex` return the delimiter index
    /// (>= 0) when inside a region, or -1 when outside.
    pub(crate) fn from_editor(editor: &Gd<CodeEdit>, line: i32, col: i32) -> Self {
        Self {
            is_in_string: editor.is_in_string_ex(line).column(col).done() != -1,
            is_in_comment: editor.is_in_comment_ex(line).column(col).done() != -1,
        }
    }

    #[allow(dead_code)] // Used by test infrastructure (bridge_tests::macros::DispatchCtx)
    pub(crate) fn default_context() -> Self {
        Self {
            is_in_string: false,
            is_in_comment: false,
        }
    }
}

pub(crate) struct CodeEditPort<'a>(pub(crate) &'a mut Gd<CodeEdit>);

// All TextEditorPort methods are 1:1 delegations to `self.0` (the Godot FFI).
// No comments on individual methods — see `port.rs` for the trait-level docs.

impl TextEditorPort for CodeEditPort<'_> {
    fn get_text(&self) -> String {
        self.0.get_text().to_string()
    }

    fn get_line(&self, line: i32) -> String {
        self.0.get_line(line).to_string()
    }

    fn insert_text_at_caret(&mut self, text: &str) {
        self.0.insert_text_at_caret(&GString::from(text));
    }

    fn delete_selection(&mut self) {
        self.0.delete_selection();
    }

    fn set_caret_line(&mut self, line: i32) {
        self.0.set_caret_line(line);
    }

    fn set_caret_column(&mut self, col: i32) {
        self.0.set_caret_column(col);
    }

    fn get_caret_line(&self) -> i32 {
        self.0.get_caret_line()
    }

    fn get_caret_column(&self) -> i32 {
        self.0.get_caret_column()
    }

    fn set_caret_line_unfold(&mut self, line: i32, viewport: ViewportAdjust) {
        self.0
            .set_caret_line_ex(line)
            .can_be_hidden(false)
            .adjust_viewport(matches!(viewport, ViewportAdjust::Adjust))
            .done();
    }

    fn adjust_viewport_to_caret(&mut self) {
        self.0.adjust_viewport_to_caret();
    }

    fn select(&mut self, from: crate::types::CharLineCol, to: crate::types::CharLineCol) {
        self.0.select(from.line, from.col, to.line, to.col);
    }

    fn deselect(&mut self) {
        self.0.deselect();
    }

    fn select_for_caret(&mut self, from: crate::types::CharLineCol, to: crate::types::CharLineCol, caret_index: i32) {
        self.0
            .select_ex(from.line, from.col, to.line, to.col)
            .caret_index(caret_index)
            .done();
    }

    fn add_caret(&mut self, line: i32, col: i32) -> i32 {
        self.0.add_caret(line, col)
    }

    fn remove_secondary_carets(&mut self) {
        self.0.remove_secondary_carets();
    }

    fn begin_complex_operation(&mut self) {
        self.0.begin_complex_operation();
    }

    fn end_complex_operation(&mut self) {
        self.0.end_complex_operation();
    }

    fn undo(&mut self) {
        self.0.undo();
    }

    fn redo(&mut self) {
        self.0.redo();
    }

    fn set_v_scroll(&mut self, value: f64) {
        self.0.set_v_scroll(value);
    }

    fn get_first_visible_line(&self) -> i32 {
        self.0.get_first_visible_line()
    }

    fn get_visible_line_count(&self) -> i32 {
        self.0.get_visible_line_count()
    }

    fn set_h_scroll(&mut self, value: i32) {
        self.0.set_h_scroll(value);
    }

    fn get_h_scroll(&self) -> i32 {
        self.0.get_h_scroll()
    }

    fn get_next_visible_line_offset_from(&self, line: i32, visible_amount: i32) -> i32 {
        self.0.get_next_visible_line_offset_from(line, visible_amount)
    }
}

impl FoldCapable for CodeEditPort<'_> {
    fn fold_line(&mut self, line: i32) {
        self.0.fold_line(line);
    }

    fn unfold_line(&mut self, line: i32) {
        self.0.unfold_line(line);
    }

    fn toggle_foldable_line(&mut self, line: i32) {
        self.0.toggle_foldable_line(line);
    }

    fn fold_all_lines(&mut self) {
        self.0.fold_all_lines();
    }

    fn unfold_all_lines(&mut self) {
        self.0.unfold_all_lines();
    }
}

impl IdeCapable for CodeEditPort<'_> {
    fn cancel_code_completion(&mut self) {
        self.0.cancel_code_completion();
    }

    fn dismiss_code_hint(&mut self) {
        // `set_code_hint` is not exposed in gdext's typed API — must use dynamic call.
        self.0.call("set_code_hint", &["".to_variant()]);
    }
}

impl NavigationCapable for CodeEditPort<'_> {
    fn emit_symbol_lookup(&mut self, symbol: &str, line: i32, col: i32) {
        self.0.emit_signal(
            "symbol_lookup",
            &[
                symbol.to_variant(),
                line.to_variant(),
                col.to_variant(),
            ],
        );
    }

    fn emit_symbol_hovered_with_mouse_warp(&mut self, symbol: &str, line: i32, col: i32) {
        // Warp mouse to the symbol position so Godot's tooltip system shows
        // the hover documentation at the cursor. Uses canvas-space coordinates
        // from `get_global_transform()`, suitable for `DisplayServer::warp_mouse`.
        let rect_local = self.0.get_rect_at_line_column(line, col);

        // Godot returns (-1, -1) for off-screen or not-yet-laid-out positions.
        if rect_local.position.x == -1 && rect_local.position.y == -1 {
            log::trace!("emit_symbol_hovered: skipping mouse warp (off-screen sentinel)");
            self.0.emit_signal(
                "symbol_hovered",
                &[
                    symbol.to_variant(),
                    line.to_variant(),
                    col.to_variant(),
                ],
            );
            return;
        }
        let pos_local = Vector2::new(rect_local.position.x as f32, rect_local.position.y as f32);
        let transform = self.0.get_global_transform();
        let pos_global = transform * pos_local;

        // NaN guard: a degenerate transform or uninitialized layout can produce
        // NaN, and f32->i32 cast of NaN/infinity is UB in Rust (saturates in
        // release, may panic in debug).
        if pos_global.x.is_nan() || pos_global.y.is_nan() {
            log::warn!("emit_symbol_hovered: NaN position, skipping mouse warp");
            self.0.emit_signal(
                "symbol_hovered",
                &[
                    symbol.to_variant(),
                    line.to_variant(),
                    col.to_variant(),
                ],
            );
            return;
        }

        // Clamp before cast: f32->i32 is UB for out-of-range values.
        let warp_x = pos_global.x.clamp(i32::MIN as f32, i32::MAX as f32) as i32;
        // Vertically center the warp point within the line's glyph rectangle.
        let warp_y = (pos_global.y + rect_local.size.y as f32 / 2.0)
            .clamp(i32::MIN as f32, i32::MAX as f32) as i32;

        let mut display_server = godot::classes::DisplayServer::singleton();
        display_server.warp_mouse(Vector2i::new(warp_x, warp_y));

        self.0.emit_signal(
            "symbol_hovered",
            &[
                symbol.to_variant(),
                line.to_variant(),
                col.to_variant(),
            ],
        );
    }
}
