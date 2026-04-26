//! Builds a vim-core [`InputContext`] snapshot from the live CodeEdit state.
//!
//! Each keystroke dispatches through `build_context`, which reads the cursor,
//! selection, viewport, and fold/indent providers into an immutable snapshot
//! the engine can process without further FFI calls.

use godot::classes::CodeEdit;
use godot::prelude::*;
use vim_core::execution::ViewportInfo;
use vim_core::document::{FoldProvider, IndentProvider, Providers};
use vim_core::execution::{InputContext, Validated};
use vim_core::primitives::{Direction, LineNumber, Offset, SelectionRange};

use super::code_edit_ext::CodeEditExt;
use super::codec::{self, DocumentView};
use super::document::GodotDocument;

/// Adapts Godot's CodeEdit fold queries into vim-core's `FoldProvider` trait.
///
/// Fold state lives entirely in Godot's scene tree, so this provider must
/// make FFI calls on every query. The queries are cheap (O(1) per line in
/// Godot's internal data structure), so no caching is needed.
pub(crate) struct GodotFoldProvider<'a> {
    editor: &'a Gd<CodeEdit>,
}

impl FoldProvider for GodotFoldProvider<'_> {
    fn next_visible_line(&self, line: LineNumber, direction: Direction) -> LineNumber {
        if !self.is_folded(line) {
            return line;
        }
        let line_i32 = codec::usize_to_i32(usize::from(line));
        let result = match direction {
            Direction::Forward => self.editor.move_down_visible(line_i32),
            Direction::Backward => self.editor.move_up_visible(line_i32),
            // Direction is non-exhaustive; default forward to avoid panicking
            // on new variants added to vim-core.
            _ => {
                log::warn!("Unknown Direction variant {:?} in next_visible_line — treating as Forward", direction);
                self.editor.move_down_visible(line_i32)
            }
        };
        LineNumber::from(codec::i32_to_usize(result))
    }

    fn is_folded(&self, line: LineNumber) -> bool {
        let line_i32 = codec::usize_to_i32(usize::from(line));
        let line_count = self.editor.get_line_count();
        if line_i32 >= line_count {
            log::trace!("is_folded: line {} >= count {}", line_i32, line_count);
            return false;
        }
        // Godot's offset_from(line, 1) returns how many document lines you must
        // advance past `line` to reach 1 visible line. A visible line is "itself"
        // the first hit, so offset = 1. A line hidden inside a fold body requires
        // skipping further, so offset > 1. At EOF Godot may return 0 — treat as visible.
        let offset = self.editor.get_next_visible_line_offset_from(line_i32, 1);
        offset > 1
    }
}

impl<'a> GodotFoldProvider<'a> {
    #[must_use]
    pub(crate) fn new(editor: &'a Gd<CodeEdit>) -> Self {
        Self { editor }
    }
}

/// Adapts Godot's CodeEdit into vim-core's `IndentProvider` trait.
///
/// Implements a simple heuristic for `o`/`O` commands: if the current line
/// ends with a block-opening character (`:`, `{`, `(`, `[`), the new line
/// gets one extra indent level. The trailing character is checked against
/// Godot's syntax highlighter to avoid false positives inside string literals.
pub(crate) struct GodotIndentProvider<'a> {
    editor: &'a Gd<CodeEdit>,
}

// SAFETY: GodotIndentProvider is only constructed and used on the main thread
// (Godot's scene tree callback path). The `Send` bound on `IndentProvider`
// exists for hosts that may transfer providers across threads; Godot's
// single-threaded model ensures no actual cross-thread transfer occurs.
unsafe impl Send for GodotIndentProvider<'_> {}

impl IndentProvider for GodotIndentProvider<'_> {
    fn indent_for_new_line(&self, line: LineNumber) -> compact_str::CompactString {
        use compact_str::CompactString;

        let line_i32 = codec::usize_to_i32(usize::from(line));
        let line_count = self.editor.get_line_count();
        if line_i32 >= line_count {
            return CompactString::default();
        }

        let line_text = self.editor.get_line(line_i32).to_string();

        let indent: CompactString = line_text.chars().take_while(|&c| c == ' ' || c == '\t').collect();

        // Only auto-indent if the last non-whitespace character is a block opener
        // AND it is not inside a string literal (Godot's `is_in_string_ex` returns
        // -1 when the column is outside any string region).
        let trimmed = line_text.trim_end();
        let last_char_col = codec::usize_to_i32(trimmed.chars().count().saturating_sub(1));
        let should_indent = !trimmed.is_empty()
            && (trimmed.ends_with(':')
                || trimmed.ends_with('{')
                || trimmed.ends_with('(')
                || trimmed.ends_with('['))
            && self.editor.is_in_string_ex(line_i32).column(last_char_col).done() == -1;

        if should_indent {
            let use_spaces = self.editor.is_indent_using_spaces();
            let indent_size = self.editor.safe_indent_size();
            if use_spaces {
                compact_str::format_compact!("{indent}{}", " ".repeat(indent_size))
            } else {
                compact_str::format_compact!("{indent}\t")
            }
        } else {
            indent
        }
    }
}

impl<'a> GodotIndentProvider<'a> {
    #[must_use]
    pub(crate) fn new(editor: &'a Gd<CodeEdit>) -> Self {
        Self { editor }
    }
}

/// Snapshot the live CodeEdit state into an `InputContext` for the vim engine.
///
/// This is the single FFI boundary crossing per keystroke: every Godot query
/// (cursor position, selection, viewport geometry, fold/indent state) happens
/// here. The returned `InputContext` is fully self-contained and requires no
/// further Godot calls during engine processing.
pub(crate) fn build_context<'a>(
    editor: &Gd<CodeEdit>,
    doc: &'a GodotDocument<'a>,
    providers: Providers<'a>,
) -> InputContext<'a, GodotDocument<'a>, Validated> {
    let text = doc.text();
    let line_index = doc.line_index();

    let cursor_offset = line_index.line_col_to_byte(
        text,
        editor.get_caret_line(),
        editor.get_caret_column(),
    );
    debug_assert!(
        cursor_offset <= text.len(),
        "Cursor offset {} exceeds text length {}",
        cursor_offset, text.len()
    );

    let mut ctx = InputContext::new(doc, cursor_offset).validate_clamped();
    if editor.has_selection() {
        let doc_view = DocumentView::new(text, line_index);
        ctx = ctx.with_selection(read_selection(&doc_view, editor));
    }

    let first_line = codec::i32_to_usize(editor.get_first_visible_line());
    let height = editor.safe_visible_line_count();
    let width = approximate_viewport_width(editor);
    ctx = ctx.with_viewport(ViewportInfo {
        first_line,
        height,
        width,
    });

    ctx = ctx.with_providers(providers);

    log::trace!(
        "build_context: offset={} selection={} viewport=[lines {}..{}, width={}]",
        cursor_offset,
        editor.has_selection(),
        first_line,
        first_line + height,
        width
    );

    ctx
}

const DEFAULT_VIEWPORT_WIDTH: usize = 80;

/// Estimate viewport width in columns from pixel dimensions and font metrics.
///
/// CodeEdit exposes no column-count API, so we divide pixel width by the
/// space-character advance. Exact for monospace fonts (the code editor case);
/// only approximate if a proportional font is somehow configured. Falls back
/// to 80 columns when font metrics are unavailable or degenerate.
fn approximate_viewport_width(editor: &Gd<CodeEdit>) -> usize {
    let pixel_width = editor.get_size().x;
    if pixel_width <= 0.0 {
        return DEFAULT_VIEWPORT_WIDTH;
    }

    let Some(font) = editor.get_theme_font("font") else {
        return DEFAULT_VIEWPORT_WIDTH;
    };
    let font_size = editor.get_theme_font_size("font_size");
    let char_width = font.get_char_size(' ' as u32, font_size).x;
    if char_width <= 0.0 {
        return DEFAULT_VIEWPORT_WIDTH;
    }

    let ratio = pixel_width / char_width;
    if !ratio.is_finite() {
        return DEFAULT_VIEWPORT_WIDTH;
    }
    let columns = (ratio as usize).min(10000);
    if columns == 0 { DEFAULT_VIEWPORT_WIDTH } else { columns }
}

/// Read the current Godot selection as a byte-offset `SelectionRange`.
///
/// ## Round-trip consistency with `handle_set_selection`
///
/// The engine emits `Effect::SetSelection { anchor, head }` as inclusive byte
/// offsets. `handle_set_selection` converts to Godot (line, col) and calls
/// `editor.select(...)`. Godot then sorts the endpoints internally:
/// `get_selection_from_*()` = lower, `get_selection_to_*()` = higher.
///
/// The round-trip is self-consistent despite `handle_set_selection` applying
/// a +1 column offset for Char-mode rendering, because:
/// (a) `get_selection_from/to` returns sorted positions, so anchor is the
///     un-shifted end, and
/// (b) the engine replaces the head with the new motion result each keystroke
///     rather than accumulating from the old head.
///
/// ## Directionality
///
/// The returned range is always forward (`anchor <= head`) because Godot sorts
/// endpoints. The engine recovers directionality from the caret position
/// (`get_caret_column`), which indicates which end is the active head.
fn read_selection(doc: &DocumentView, editor: &Gd<CodeEdit>) -> SelectionRange {
    let anchor_offset = doc.line_index.line_col_to_byte(
        doc.text,
        editor.get_selection_from_line(),
        editor.get_selection_from_column(),
    );

    let head_offset = doc.line_index.line_col_to_byte(
        doc.text,
        editor.get_selection_to_line(),
        editor.get_selection_to_column(),
    );

    SelectionRange::new(
        Offset::new(anchor_offset),
        Offset::new(head_offset),
    )
}
