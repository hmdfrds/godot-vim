//! Live substitute preview overlay for `:s/pattern/replacement/flags` (inccommand).
//!
//! Renders match highlights as semi-transparent yellow rectangles via `_draw()`
//! on a transparent Control layered above CodeEdit. Match positions arrive as
//! logical `(line, col)` coordinates from the engine; pixel rectangles are
//! computed via `get_rect_at_line_column` and cached in `highlights` until the
//! next `update_matches` call. Calling `queue_redraw()` after each update
//! triggers Godot to invoke `_draw()` on the next frame.
//!
//! The overlay is capped at `MAX_HIGHLIGHTS` rectangles to bound draw cost
//! on files with thousands of matches (e.g. `:s/e/x/g` on a large file).
//!
//! **Viewport invalidation:** Cached pixel rects become stale on scroll or
//! resize because `get_rect_at_line_column` returns viewport-relative
//! coordinates. The overlay stores the last logical `MatchRange` positions so
//! that external signals (scrollbar `value_changed`, editor `draw`,
//! `minimum_size_changed`) can call [`InccommandOverlay::recompute_rects`] to
//! rebuild highlights without re-running the regex engine.

use godot::classes::{CodeEdit, Control, IControl};
use godot::prelude::*;

use crate::safety::panic_guard;

struct MatchHighlight {
    rect: Rect2,
}

/// Cap on highlight rectangles per preview update. Without this, `:s/e/x/g`
/// on a large file would generate thousands of draw_rect calls per frame.
const MAX_HIGHLIGHTS: usize = 100;

#[derive(GodotClass)]
#[class(base=Control)]
pub(crate) struct InccommandOverlay {
    base: Base<Control>,
    highlights: Vec<MatchHighlight>,
    /// Logical match positions from the last `update_matches` call.
    /// Retained so that viewport-change signals (scroll, resize) can
    /// recompute pixel rects without re-running the regex engine.
    last_positions: Vec<crate::types::MatchRange>,
}

#[godot_api]
impl IControl for InccommandOverlay {
    fn init(base: Base<Control>) -> Self {
        Self {
            base,
            highlights: Vec::new(),
            last_positions: Vec::new(),
        }
    }

    fn draw(&mut self) {
        panic_guard(
            "inccommand::draw",
            || {
                if self.highlights.is_empty() {
                    return;
                }
                // Yellow at 30% opacity -- matches Vim's inccommand highlight convention.
                let color = Color::from_rgba(1.0, 1.0, 0.0, 0.3);
                // Index loop avoids iterator borrow conflict with base_mut().
                for i in 0..self.highlights.len() {
                    let rect = self.highlights[i].rect;
                    self.base_mut().draw_rect(rect, color);
                }
            },
            (),
        );
    }
}

#[godot_api]
impl InccommandOverlay {
    /// Exposed to GDScript for scene teardown; Rust code uses `clear_highlights`.
    #[func]
    pub fn clear(&mut self) {
        panic_guard(
            "inccommand::clear",
            || {
                if self.highlights.is_empty() {
                    return;
                }
                self.highlights.clear();
                self.base_mut().queue_redraw();
            },
            (),
        );
    }
}

impl InccommandOverlay {
    /// Rebuild highlight rectangles from engine-provided match ranges.
    ///
    /// Positions arrive as logical `(line, col)` pairs so the UI layer never
    /// needs document text. Pixel geometry is computed via `get_rect_at_line_column`.
    /// The logical positions are retained in `last_positions` so that
    /// [`recompute_rects`] can rebuild pixel rects after scroll or resize
    /// without re-running the regex engine.
    pub(crate) fn update_matches(
        &mut self,
        positions: &[crate::types::MatchRange],
        editor: &Gd<CodeEdit>,
    ) {
        self.last_positions = positions.to_vec();
        self.rebuild_highlights_from_positions(editor);
    }

    /// Recompute pixel rectangles from the stored logical positions.
    ///
    /// Called by the plugin layer when the viewport changes (scroll, resize,
    /// editor redraw) while a substitute preview is active. No-op when
    /// `last_positions` is empty (no active preview).
    pub(crate) fn recompute_rects(&mut self, editor: &Gd<CodeEdit>) {
        if self.last_positions.is_empty() {
            return;
        }
        self.rebuild_highlights_from_positions(editor);
    }

    pub(crate) fn clear_highlights(&mut self) {
        let was_empty = self.highlights.is_empty() && self.last_positions.is_empty();
        self.highlights.clear();
        self.last_positions.clear();
        if !was_empty {
            self.base_mut().queue_redraw();
        }
    }

    /// Shared implementation: compute pixel rects from `self.last_positions`.
    fn rebuild_highlights_from_positions(&mut self, editor: &Gd<CodeEdit>) {
        self.highlights.clear();

        for m in self.last_positions.iter().take(MAX_HIGHLIGHTS) {
            let remaining = MAX_HIGHLIGHTS - self.highlights.len();
            let rects =
                super::geometry::compute_highlight_rects(editor, &m.start, &m.end, remaining);
            self.highlights
                .extend(rects.into_iter().map(|rect| MatchHighlight { rect }));
            if self.highlights.len() >= MAX_HIGHLIGHTS {
                break;
            }
        }

        self.base_mut().queue_redraw();
    }
}
