//! Highlight yank overlay — brief fade-out animation on yanked text.
//!
//! Computes pixel rectangles on the first `_draw()` call and caches them
//! for subsequent frames within the same animation.  The cache is
//! invalidated when a new animation starts or the current one ends,
//! keeping the highlight aligned with the gutter layout at animation
//! start while avoiding repeated FFI calls on every frame.

use godot::classes::{CodeEdit, Control, IControl};
use godot::prelude::*;

use crate::safety::panic_guard;
use crate::types::CharLineCol;

const HIGHLIGHT_ALPHA: f32 = 0.4;

/// Bounds draw cost for yanks spanning thousands of lines (e.g. `yG` at top of file).
const MAX_HIGHLIGHT_RECTS: usize = 500;

#[derive(GodotClass)]
#[class(base=Control)]
pub(crate) struct HighlightYankOverlay {
    base: Base<Control>,
    start: CharLineCol,
    end: CharLineCol,
    alpha: f32,
    fade_duration: f32,
    elapsed: f32,
    active: bool,
    /// Pixel rectangles computed once on the first `_draw()` of each animation
    /// cycle, then reused for every subsequent frame. This avoids per-frame FFI
    /// calls to `get_rect_at_line_column` -- the gutter layout doesn't change
    /// within a single 150ms fade-out.
    cached_rects: Vec<Rect2>,
    /// Separate from `cached_rects.is_empty()` because offscreen ranges produce
    /// zero rects legitimately; without this flag we'd re-attempt FFI every frame.
    rects_computed: bool,
}

#[godot_api]
impl IControl for HighlightYankOverlay {
    fn init(base: Base<Control>) -> Self {
        Self {
            base,
            start: CharLineCol::new(0, 0),
            end: CharLineCol::new(0, 0),
            alpha: 0.0,
            fade_duration: 0.15,
            elapsed: 0.0,
            active: false,
            cached_rects: Vec::new(),
            rects_computed: false,
        }
    }

    fn process(&mut self, delta: f64) {
        panic_guard(|| {
            if !self.active {
                return;
            }
            self.elapsed += delta as f32;
            if self.elapsed >= self.fade_duration {
                self.active = false;
                self.alpha = 0.0;
                self.cached_rects.clear();
                self.rects_computed = false;
                self.base_mut().queue_redraw();
                self.base_mut().set_process(false);
                return;
            }
            self.alpha = HIGHLIGHT_ALPHA * (1.0 - self.elapsed / self.fade_duration);
            self.base_mut().queue_redraw();
        }, ());
    }

    fn draw(&mut self) {
        panic_guard(|| {
            if !self.active {
                return;
            }

            let color = Color::from_rgba(1.0, 1.0, 0.0, self.alpha);

            // Lazy-compute on first draw frame of this animation cycle.
            if !self.rects_computed {
                self.rects_computed = true;
                let Some(parent) = self.base().get_parent() else { return };
                let Ok(editor) = parent.try_cast::<CodeEdit>() else { return };

                self.cached_rects = super::geometry::compute_highlight_rects(
                    &editor, &self.start, &self.end, MAX_HIGHLIGHT_RECTS,
                );
            }

            // Index loop avoids iterator borrow conflict with base_mut().
            for i in 0..self.cached_rects.len() {
                let rect = self.cached_rects[i];
                self.base_mut().draw_rect(rect, color);
            }
        }, ());
    }
}

#[godot_api]
impl HighlightYankOverlay {}

impl HighlightYankOverlay {
    /// Begin a fade-out animation highlighting the yanked range. Invalidates
    /// any cached pixel rects so they are recomputed on the next `_draw()`.
    pub(crate) fn show_yank(
        &mut self,
        start: CharLineCol,
        end: CharLineCol,
        duration_ms: u32,
        _editor: &Gd<CodeEdit>,
    ) {
        self.start = start;
        self.end = end;
        self.fade_duration = (duration_ms as f32 / 1000.0).max(0.01);
        self.elapsed = 0.0;
        self.alpha = HIGHLIGHT_ALPHA;
        self.active = true;
        self.cached_rects.clear();
        self.rects_computed = false;

        self.base_mut().set_process(true);
        self.base_mut().queue_redraw();
    }

}
