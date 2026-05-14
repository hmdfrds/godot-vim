//! Block visual selection overlay.
//!
//! Replaces the secondary-caret-based block selection rendering with a
//! transparent overlay, eliminating the dual-purpose caret problem where
//! secondary carets created for rendering were misinterpreted as user-added
//! multi-cursors by the import and sync logic.
//!
//! Renders one colored rectangle per line in the block selection range using
//! `_draw()` on a transparent Control layered above CodeEdit. Logical
//! positions (anchor_line, anchor_col, head_line, head_col) are stored so
//! that viewport-change signals (scroll, resize) can recompute pixel rects
//! without re-querying the engine.

use godot::classes::{CodeEdit, Control, IControl};
use godot::prelude::*;

use crate::safety::panic_guard;

/// Cap on highlight rectangles to bound draw cost on huge block selections.
const MAX_HIGHLIGHT_RECTS: usize = 500;

/// Logical block geometry: the four corners of the block selection.
#[derive(Debug, Clone, Copy, Default)]
struct BlockGeometry {
    anchor_line: i32,
    anchor_col: i32,
    head_line: i32,
    head_col: i32,
}

#[derive(GodotClass)]
#[class(base=Control)]
pub(crate) struct BlockVisualOverlay {
    base: Base<Control>,
    highlights: Vec<Rect2>,
    /// Stored logical positions for recomputing on scroll/resize.
    geometry: Option<BlockGeometry>,
}

#[godot_api]
impl IControl for BlockVisualOverlay {
    fn init(base: Base<Control>) -> Self {
        Self {
            base,
            highlights: Vec::new(),
            geometry: None,
        }
    }

    fn draw(&mut self) {
        panic_guard(
            "block_visual::draw",
            || {
                if self.highlights.is_empty() {
                    return;
                }
                // Use Godot's selection color convention: blue at 40% opacity.
                let color = Color::from_rgba(0.26, 0.52, 0.96, 0.4);
                for i in 0..self.highlights.len() {
                    let rect = self.highlights[i];
                    self.base_mut().draw_rect(rect, color);
                }
            },
            (),
        );
    }
}

#[godot_api]
impl BlockVisualOverlay {
    /// Exposed to GDScript for scene teardown; Rust code uses `clear_highlights`.
    #[func]
    pub fn clear(&mut self) {
        panic_guard(
            "block_visual::clear",
            || {
                if self.highlights.is_empty() && self.geometry.is_none() {
                    return;
                }
                self.highlights.clear();
                self.geometry = None;
                self.base_mut().queue_redraw();
            },
            (),
        );
    }
}

impl BlockVisualOverlay {
    /// Update the block selection highlight with new geometry.
    ///
    /// Computes one pixel rect per line in the block, from min_col to max_col+1
    /// (Vim-inclusive to Godot-exclusive conversion).
    pub(crate) fn update_block(
        &mut self,
        anchor_line: i32,
        anchor_col: i32,
        head_line: i32,
        head_col: i32,
        editor: &Gd<CodeEdit>,
    ) {
        self.geometry = Some(BlockGeometry {
            anchor_line,
            anchor_col,
            head_line,
            head_col,
        });
        self.rebuild_highlights(editor);
    }

    /// Recompute pixel rectangles from stored logical positions.
    ///
    /// Called by the coordinator when the viewport changes (scroll, resize)
    /// while a block selection is active. No-op when no geometry is stored.
    pub(crate) fn recompute_rects(&mut self, editor: &Gd<CodeEdit>) {
        if self.geometry.is_none() {
            return;
        }
        self.rebuild_highlights(editor);
    }

    /// Clear the overlay (hide the block selection highlight).
    pub(crate) fn clear_highlights(&mut self) {
        let was_empty = self.highlights.is_empty() && self.geometry.is_none();
        self.highlights.clear();
        self.geometry = None;
        if !was_empty {
            self.base_mut().queue_redraw();
        }
    }

    /// Shared implementation: compute pixel rects from stored geometry.
    fn rebuild_highlights(&mut self, editor: &Gd<CodeEdit>) {
        self.highlights.clear();

        let Some(geom) = self.geometry else {
            return;
        };

        let min_line = geom.anchor_line.min(geom.head_line);
        let max_line = geom.anchor_line.max(geom.head_line);
        let min_col = geom.anchor_col.min(geom.head_col);
        // +1 converts Vim-inclusive end to Godot-exclusive end for rendering.
        let max_col_exclusive = geom.anchor_col.max(geom.head_col) + 1;

        let line_height = editor.get_line_height().max(1);

        for line in min_line..=max_line {
            if self.highlights.len() >= MAX_HIGHLIGHT_RECTS {
                break;
            }

            let Some(start_pos) = super::geometry::corrected_col_x(editor, line, min_col) else {
                continue;
            };
            let Some(end_pos) = super::geometry::corrected_col_x(editor, line, max_col_exclusive)
            else {
                continue;
            };

            let width = (end_pos.x - start_pos.x).max(1);
            self.highlights.push(Rect2::new(
                Vector2::new(start_pos.x as f32, start_pos.y as f32),
                Vector2::new(width as f32, line_height as f32),
            ));
        }

        self.base_mut().queue_redraw();
    }
}
