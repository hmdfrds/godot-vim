//! `TextEditorPort` implementation: `CodeEditPort` newtype over `Gd<CodeEdit>`.
//!
//! The newtype is necessary because implementing a trait directly on
//! `Gd<CodeEdit>` causes infinite recursion: trait method names shadow the
//! identically-named Godot-generated inherent methods, so `self.get_text()`
//! calls the trait method instead of the Godot FFI method. The newtype
//! breaks this ambiguity — `self.0.get_text()` always resolves to Godot.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use godot::classes::{CodeEdit, EditorInterface, InputEventShortcut};
use godot::prelude::*;

use super::port::{FoldCapable, IdeCapable, NavigationCapable, TextEditorPort, ViewportAdjust};
use crate::bridge::godot_calls;

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

// ── Pending tooltip data ────────────────────────────────────────────────
//
// Thread-local store for deferred tooltip emission. `show_documentation_tooltip`
// (Tier 2 path) deposits data here; the plugin layer drains it after dispatch.

pub(crate) struct PendingTooltipData {
    pub symbol: String,
    pub line: i32,
    pub col: i32,
    pub warp_pos: Option<Vector2i>,
}

thread_local! {
    static PENDING_TOOLTIP_DATA: Cell<Option<PendingTooltipData>> = const { Cell::new(None) };
}

pub(crate) fn take_pending_tooltip_data() -> Option<PendingTooltipData> {
    PENDING_TOOLTIP_DATA.take()
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

    /// Remove pairs where both open and close are single characters.
    ///
    /// When vim-core's `auto_pairs` is active, it owns all single-char pairs
    /// (e.g. `()`, `{}`, `""`). This method filters those out so that the
    /// host-side auto-brace only handles multi-char pairs (e.g. `/* */`),
    /// preventing the two systems from conflicting on the same pair set.
    pub(crate) fn filter_engine_owned_pairs(&mut self) {
        let pairs = Rc::make_mut(&mut self.pairs);
        pairs.retain(|(open, close)| {
            open.chars().count() != 1 || close.chars().count() != 1
        });
    }

    /// Check if `key` is a string delimiter start key (no FFI — answered from snapshot).
    pub(crate) fn has_string_delimiter(&self, key: &str) -> bool {
        self.string_delimiters.iter().any(|d| d == key)
    }
}

/// Syntax region at a cursor position, replacing two booleans
/// (`is_in_string`, `is_in_comment`) whose four combinations included one
/// illegal state (simultaneously in both a string and a comment).
///
/// Used to suppress auto-brace and other syntax-aware behaviors when the
/// cursor is inside a string literal or comment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SyntaxRegion {
    /// Normal code — not inside any special syntax region.
    Code,
    /// Inside a string literal (Godot's `is_in_string_ex` returned >= 0).
    String,
    /// Inside a comment (Godot's `is_in_comment_ex` returned >= 0).
    Comment,
}

impl SyntaxRegion {
    /// Capture syntax region at `(line, col)` via Godot FFI.
    ///
    /// Godot's `is_in_string_ex`/`is_in_comment_ex` return the delimiter index
    /// (>= 0) when inside a region, or -1 when outside. Comment takes priority
    /// over string when both report true (shouldn't happen, but defensive).
    pub(crate) fn from_editor(editor: &Gd<CodeEdit>, line: i32, col: i32) -> Self {
        if editor.is_in_comment_ex(line).column(col).done() != -1 {
            Self::Comment
        } else if editor.is_in_string_ex(line).column(col).done() != -1 {
            Self::String
        } else {
            Self::Code
        }
    }

    /// Default context for tests and contexts without syntax analysis.
    #[allow(dead_code)] // Used by test infrastructure (bridge_tests::macros::DispatchCtx)
    pub(crate) fn code() -> Self {
        Self::Code
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
        super::godot_calls::dismiss_code_hint(self.0);
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

    fn show_documentation_tooltip(&mut self, symbol: &str, line: i32, col: i32) {
        // Tier 1 (Godot 4.7+): Synthesize an InputEventShortcut for
        // SHOW_TOOLTIP_AT_CARET. The shortcut "script_text_editor/show_tooltip"
        // is only registered in Godot 4.7+; its presence acts as implicit
        // version detection. When ScriptTextEditor receives this shortcut, it
        // calls _show_symbol_tooltip(p_shortcut=true), which:
        //   - Bypasses the is_anything_pressed() guard in make_tooltip
        //   - Positions the tooltip at the caret (not the mouse)
        //   - Works on Wayland/X11/macOS/Windows identically
        // Pattern proven by trigger_script_editor_close() in editor_host.rs.
        let editor_iface = EditorInterface::singleton();
        if let Some(mut settings) = editor_iface.get_editor_settings() {
            if let Some(shortcut) =
                godot_calls::get_shortcut(&mut settings, godot_calls::SHORTCUT_SHOW_TOOLTIP)
            {
                if let Some(mut viewport) = editor_iface
                    .get_base_control()
                    .and_then(|ctrl| ctrl.get_viewport())
                {
                    let mut event: Gd<InputEventShortcut> = InputEventShortcut::new_gd();
                    event.set_shortcut(&shortcut);
                    viewport.call_deferred(
                        "push_input",
                        &[event.to_variant(), false.to_variant()],
                    );
                    log::debug!("show_documentation_tooltip: shortcut synthesis for '{symbol}'");
                    return;
                }
            }
        }

        // Tier 2 (Godot ≤4.6): Deferred tooltip emission.
        // Compute the warp position but do NOT warp or emit a signal here.
        // Instead, store the data in a thread-local for the plugin layer to
        // pick up after dispatch completes.
        let rect_local = self.0.get_rect_at_line_column(line, col);

        let warp_pos = if rect_local.position.x == -1 && rect_local.position.y == -1 {
            log::trace!("show_documentation_tooltip: off-screen sentinel, no warp_pos");
            None
        } else {
            let pos_local = Vector2::new(
                rect_local.position.x as f32,
                rect_local.position.y as f32,
            );
            let transform = self.0.get_global_transform();
            let pos_global = transform * pos_local;

            if !pos_global.x.is_nan() && !pos_global.y.is_nan() {
                let warp_x = super::codec::f32_to_i32_sat(pos_global.x);
                let warp_y = super::codec::f32_to_i32_sat(
                    pos_global.y + rect_local.size.y as f32 / 2.0,
                );
                Some(Vector2i::new(warp_x, warp_y))
            } else {
                None
            }
        };

        PENDING_TOOLTIP_DATA.set(Some(PendingTooltipData {
            symbol: symbol.to_string(),
            line,
            col,
            warp_pos,
        }));

        log::debug!(
            "show_documentation_tooltip: stored pending tooltip for '{symbol}' \
             (shortcut unavailable — plugin layer will emit)"
        );
    }
}
