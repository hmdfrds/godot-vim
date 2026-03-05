//! Godot `CodeEdit` adapter implementing `DocumentSnapshot`.
//!
//! This is the thin shell that bridges the pure core to Godot.
//! It wraps a `CodeEdit` reference and provides read-only document access.
//!
//! ## Performance
//! Lines are fetched on-demand from `CodeEdit` with LRU caching instead of cloning
//! the entire document upfront. This makes snapshot creation O(1) instead of O(N),
//! and repeated line access benefits from the cache.

use std::cell::RefCell;
use std::num::NonZeroUsize;

use crate::bridge::godot::code_edit_ext::CodeEditExt;
use godot::classes::text_edit::SearchFlags;
use godot::classes::CodeEdit;
use godot::obj::Gd;
use lru::LruCache;

use crate::bridge::vim_adapter::core::cast::{i32_to_usize, usize_to_i32};
use crate::bridge::vim_adapter::core::column_codec;
use vim_core::domain::capabilities::viewport::ViewportCapability;
use vim_core::domain::fold::{FoldProvider, VerticalDirection};
use vim_core::domain::position::Position;
use vim_core::domain::search_provider::SearchProvider;
use vim_core::domain::selection::Selection;
use vim_core::domain::shared_str::SharedStr;
use vim_core::domain::snapshot::DocumentSnapshot;

/// LRU cache capacity for line caching (compile-time verified non-zero).
const LINE_CACHE_CAP: NonZeroUsize = {
    // Compile-time verified: 32 is non-zero
    match NonZeroUsize::new(32) {
        Some(v) => v,
        None => panic!("LINE_CACHE_CAP must be non-zero"),
    }
};

/// Lazy snapshot of a Godot `CodeEdit` for pure function access.
///
/// This struct provides on-demand access to editor state for pure processing.
/// Lines are fetched lazily from `CodeEdit` when accessed, with an LRU cache
/// for frequently accessed lines. Snapshot creation is O(1) instead of O(N).
pub struct GodotSnapshot {
    /// Reference to the `CodeEdit` for lazy line access.
    editor: Gd<CodeEdit>,
    /// Cached line count (single API call on creation).
    line_count: usize,
    /// Current selection (cursor position + anchor).
    selection: Selection,
    /// LRU cache for line content (interior mutability for caching in &self methods).
    line_cache: RefCell<LruCache<usize, SharedStr>>,
}

impl GodotSnapshot {
    /// Creates a lazy snapshot from a `CodeEdit`.
    ///
    /// Only captures line count and cursor position - O(1) operation.
    /// Lines are fetched on-demand when `line()` is called, with LRU caching.
    #[must_use]
    pub fn from_editor(editor: &Gd<CodeEdit>) -> Self {
        let editor_clone = editor.clone();

        // O(1) metadata capture
        let line_count = i32_to_usize(editor_clone.get_line_count());
        let cursor_line = i32_to_usize(editor_clone.get_caret_line());
        let cursor_editor_col = i32_to_usize(editor_clone.get_caret_column());
        let cursor_col =
            column_codec::editor_col_to_byte_in_editor(&editor_clone, cursor_line, cursor_editor_col);

        // Capture the visual selection if present.
        // Godot's selection end is exclusive (points past the last selected character).
        // `update_visual_selection` adds +1 to the cursor column for display purposes,
        // so 1 is subtracted from the "to" column here to recover the logical cursor position.
        // The pure core adds +1 back via is_inclusive() when computing operation ranges.
        let (anchor, head) = if editor_clone.has_selection() {
            let sel_from_line = i32_to_usize(editor_clone.get_selection_from_line());
            let sel_from_editor_col = i32_to_usize(editor_clone.get_selection_from_column());
            let sel_to_line = i32_to_usize(editor_clone.get_selection_to_line());
            let sel_to_editor_col = i32_to_usize(editor_clone.get_selection_to_column());

            let sel_from_col = column_codec::editor_col_to_byte_in_editor(
                &editor_clone,
                sel_from_line,
                sel_from_editor_col,
            );

            // Convert from Godot display format (exclusive end) to Vim logical format
            // by subtracting 1 from the end column (saturating to prevent underflow)
            let logical_to_col = if sel_to_editor_col > 0 {
                column_codec::editor_col_to_byte_in_editor(
                    &editor_clone,
                    sel_to_line,
                    sel_to_editor_col - 1,
                )
            } else {
                0
            };

            // Determine which end is anchor vs head based on cursor position
            if (cursor_line, cursor_editor_col) == (sel_to_line, sel_to_editor_col) {
                // Cursor at end = forward selection
                (
                    Position::from_byte(sel_from_line, sel_from_col),
                    Position::from_byte(sel_to_line, logical_to_col),
                )
            } else {
                // Cursor at start = backward selection
                // For a backward selection, the "from" position is numerically higher.
                (
                    Position::from_byte(sel_to_line, logical_to_col),
                    Position::from_byte(sel_from_line, sel_from_col),
                )
            }
        } else {
            // No selection = cursor only
            let pos = Position::from_byte(cursor_line, cursor_col);
            (pos, pos)
        };

        // Use compile-time verified constant (no runtime expect needed)
        let cache_capacity = LINE_CACHE_CAP;

        Self {
            editor: editor_clone,
            line_count,
            selection: Selection::new(anchor, head),
            line_cache: RefCell::new(LruCache::new(cache_capacity)),
        }
    }

    /// Creates a lazy snapshot from a `CodeEdit` with explicit selection override.
    #[must_use]
    pub fn from_editor_with_selection(editor: &Gd<CodeEdit>, selection: Selection) -> Self {
        let editor_clone = editor.clone();
        let line_count = i32_to_usize(editor_clone.get_line_count());

        // Use compile-time verified constant (no runtime expect needed)
        let cache_capacity = LINE_CACHE_CAP;

        Self {
            editor: editor_clone,
            line_count,
            selection,
            line_cache: RefCell::new(LruCache::new(cache_capacity)),
        }
    }
}

/// JIT (Just-In-Time) snapshot that implements `DocumentSnapshot`.
///
/// This proxy delays capturing Godot metadata (line counts, selection)
/// until vim-core attempts to access the document.
/// This allows "passive" inputs (like typing in Insert mode) to avoid
/// the snapshot tax entirely.
pub struct LazyGodotSnapshot {
    editor: Gd<CodeEdit>,
    inner: RefCell<Option<GodotSnapshot>>,
}

impl LazyGodotSnapshot {
    /// Creates a new LazyGodotSnapshot. Preparation is O(0).
    pub fn new(editor: &Gd<CodeEdit>) -> Self {
        Self {
            editor: editor.clone(),
            inner: RefCell::new(None),
        }
    }

    /// Creates a new LazyGodotSnapshot with a selection override.
    pub fn with_selection(editor: &Gd<CodeEdit>, selection: Selection) -> Self {
        Self {
            editor: editor.clone(),
            inner: RefCell::new(Some(GodotSnapshot::from_editor_with_selection(
                editor, selection,
            ))),
        }
    }

    /// Internal helper to ensure the real snapshot is captured.
    fn ensure_init(&self) -> std::cell::Ref<'_, GodotSnapshot> {
        let mut inner = self.inner.borrow_mut();
        if inner.is_none() {
            *inner = Some(GodotSnapshot::from_editor(&self.editor));
        }
        drop(inner);
        // inner is guaranteed Some after the initialization above.
        // Ref::map requires an infallible closure, so expect() is the only option here.
        #[allow(clippy::expect_used)]
        std::cell::Ref::map(self.inner.borrow(), |o| {
            o.as_ref()
                .expect("inner initialized above")
        })
    }
}

impl DocumentSnapshot for LazyGodotSnapshot {
    fn line_count(&self) -> usize {
        self.ensure_init().line_count()
    }

    fn line(&self, idx: usize) -> SharedStr {
        self.ensure_init().line(idx)
    }

    fn selection(&self) -> Selection {
        self.ensure_init().selection()
    }
}

impl DocumentSnapshot for GodotSnapshot {
    fn line_count(&self) -> usize {
        self.line_count
    }

    /// Returns the content of a line.
    ///
    /// Uses `try_borrow_mut()` to avoid panics on reentrant access.
    /// Falls back to direct fetch (uncached) if cache is already borrowed.
    fn line(&self, idx: usize) -> SharedStr {
        if idx >= self.line_count {
            return SharedStr::from("");
        }

        // Use try_borrow_mut to prevent panic on reentrant access.
        // If the cache is already borrowed, fall back to an uncached fetch.
        let Ok(mut cache) = self.line_cache.try_borrow_mut() else {
            return SharedStr::from(self.editor.get_line(usize_to_i32(idx)).to_string());
        };

        if let Some(cached_line) = cache.get(&idx) {
            // SharedStr is Arc<str>; clone only increments the reference count.
            return cached_line.clone();
        }

        // Fetch from editor and cache it
        let line_content = SharedStr::from(self.editor.get_line(usize_to_i32(idx)).to_string());
        cache.put(idx, line_content.clone());
        line_content
    }

    fn selection(&self) -> Selection {
        self.selection
    }
}

impl ViewportCapability for GodotSnapshot {
    fn get_visual_line_count(&self, line: usize) -> usize {
        // Bounds check: return 1 for lines that don't exist yet
        // This can happen when cursor points to a new line before editor is updated
        if line >= self.line_count {
            return 1;
        }
        // Godot returns wrap count (splits - 1), so add 1 for segment count.
        // If line is not wrapped, returns 0, so 0+1 = 1 segment.
        let wrap_count = self.editor.get_line_wrap_count(usize_to_i32(line));
        (wrap_count + 1) as usize
    }

    fn get_wrapped_segments(&self, line: usize) -> Vec<String> {
        // Bounds check: return empty for lines that don't exist yet
        // This can happen when cursor points to a new line before editor is updated
        if line >= self.line_count {
            return vec![];
        }
        let segments = self.editor.get_line_wrapped_text(usize_to_i32(line));
        segments.as_slice().iter().map(|s| s.to_string()).collect()
    }
}

impl ViewportCapability for LazyGodotSnapshot {
    fn get_visual_line_count(&self, line: usize) -> usize {
        self.ensure_init().get_visual_line_count(line)
    }

    fn get_wrapped_segments(&self, line: usize) -> Vec<String> {
        self.ensure_init().get_wrapped_segments(line)
    }
}

impl FoldProvider for GodotSnapshot {
    fn next_visible_line(&self, line: usize, direction: VerticalDirection) -> usize {
        let line_i32 = usize_to_i32(line);
        let result = match direction {
            VerticalDirection::Down => self.editor.move_down_visible(line_i32),
            VerticalDirection::Up => self.editor.move_up_visible(line_i32),
        };
        i32_to_usize(result)
    }
}

impl FoldProvider for LazyGodotSnapshot {
    fn next_visible_line(&self, line: usize, direction: VerticalDirection) -> usize {
        self.ensure_init().next_visible_line(line, direction)
    }
}

impl SearchProvider for GodotSnapshot {
    fn find_match(
        &self,
        pattern: &str,
        from: Position,
        forward: bool,
        wrap: bool,
    ) -> Option<(Position, Position)> {
        let flags = if forward {
            SearchFlags::MATCH_CASE
        } else {
            SearchFlags::MATCH_CASE | SearchFlags::BACKWARDS
        };

        let mut result = self.editor.search(
            &godot::prelude::GString::from(pattern),
            flags,
            usize_to_i32(from.line),
            usize_to_i32(column_codec::byte_to_editor_col_in_editor(
                &self.editor,
                from.line,
                usize::from(from.col),
            )),
        );

        if result.x == -1 && wrap {
            let (wrap_line, wrap_col) = if forward {
                (0, 0)
            } else {
                let last_line = self.editor.get_line_count() - 1;
                let last_col = self.editor.get_line(last_line).to_string().chars().count() as i32;
                (last_line, last_col)
            };
            result = self.editor.search(
                &godot::prelude::GString::from(pattern),
                flags,
                wrap_line,
                wrap_col,
            );
        }

        if result.x == -1 {
            return None;
        }

        let match_line = i32_to_usize(result.y);
        let match_start_col =
            column_codec::editor_col_to_byte_in_editor(&self.editor, match_line, i32_to_usize(result.x));
        let match_start = Position::from_byte(match_line, match_start_col);
        let pattern_len = pattern.chars().count();
        let end_editor_col = i32_to_usize(result.x) + pattern_len.saturating_sub(1);
        let end_col = column_codec::editor_col_to_byte_in_editor(&self.editor, match_line, end_editor_col);
        let match_end = Position::from_byte(match_line, end_col);

        Some((match_start, match_end))
    }
}

impl SearchProvider for LazyGodotSnapshot {
    fn find_match(
        &self,
        pattern: &str,
        from: Position,
        forward: bool,
        wrap: bool,
    ) -> Option<(Position, Position)> {
        self.ensure_init().find_match(pattern, from, forward, wrap)
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Testing Support: EagerSnapshot for tests (no Godot dependency)
// ═══════════════════════════════════════════════════════════════════════════

/// Eager snapshot for testing without Godot.
///
/// This holds pre-allocated lines and is used in unit tests.
#[cfg(test)]
pub struct EagerSnapshot {
    lines: Vec<String>,
    selection: Selection,
}

#[cfg(test)]
impl EagerSnapshot {
    /// Creates a snapshot with explicit lines and selection (for testing).
    #[must_use]
    pub fn new(lines: Vec<String>, selection: Selection) -> Self {
        Self { lines, selection }
    }
}

#[cfg(test)]
impl DocumentSnapshot for EagerSnapshot {
    fn line_count(&self) -> usize {
        self.lines.len()
    }

    fn line(&self, idx: usize) -> SharedStr {
        self.lines
            .get(idx)
            .map(|s| SharedStr::from(s.as_str()))
            .unwrap_or(SharedStr::from(""))
    }

    fn selection(&self) -> Selection {
        self.selection
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_eager_snapshot_new() {
        let snap = EagerSnapshot::new(
            vec!["Hello".to_string(), "World".to_string()],
            Selection::cursor(0, 0),
        );
        assert_eq!(snap.line_count(), 2);
        assert_eq!(snap.line(0).as_ref(), "Hello");
    }

    #[test]
    fn test_eager_snapshot_line_len() {
        let snap = EagerSnapshot::new(vec!["Hello".to_string()], Selection::cursor(0, 0));
        assert_eq!(snap.line_len(0), 5);
    }

    #[test]
    fn test_eager_snapshot_cursor() {
        let snap = EagerSnapshot::new(vec!["Hello".to_string()], Selection::cursor(0, 3));
        assert_eq!(snap.cursor(), Position::from_byte(0, 3));
    }

    #[test]
    fn test_eager_snapshot_selection() {
        let sel = Selection::new(Position::from_byte(0, 0), Position::from_byte(0, 5));
        let snap = EagerSnapshot::new(vec!["Hello World".to_string()], sel);
        assert_eq!(snap.selection(), sel);
    }
}
