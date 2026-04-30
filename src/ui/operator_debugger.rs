//! Visual operator debugger overlay — highlights the affected range of the
//! last operation when `:vimdebug` is active.
//!
//! Renders a semi-transparent blue rectangle over the text range that was
//! affected by the most recent Delete or Replace operation. The highlight
//! auto-clears after 500 ms via a one-shot Timer.
//!
//! Architecture follows the same pattern as [`super::inccommand`]: a
//! `Control` node layered on top of `CodeEdit` that draws colored rectangles
//! using pixel positions from `get_rect_at_line_column`.

use godot::classes::{CodeEdit, Control, IControl, Timer};
use godot::prelude::*;

use crate::safety::panic_guard;
use crate::types::CharLineCol;

struct RangeHighlight {
    rect: Rect2,
}

const CLEAR_DELAY_SECS: f64 = 0.5;

/// Cap on pixel rectangles per debug range to prevent runaway draw cost
/// when an operation spans thousands of lines.
const MAX_HIGHLIGHT_RECTS: usize = 500;

#[derive(GodotClass)]
#[class(base=Control)]
pub(crate) struct DebugRangeOverlay {
    base: Base<Control>,
    highlights: Vec<RangeHighlight>,
    clear_timer: Option<Gd<Timer>>,
}

#[godot_api]
impl IControl for DebugRangeOverlay {
    fn init(base: Base<Control>) -> Self {
        Self {
            base,
            highlights: Vec::new(),
            clear_timer: None,
        }
    }

    fn ready(&mut self) {
        panic_guard(
            "operator_debugger::ready",
            || {
                let mut timer = Timer::new_alloc();
                timer.set_one_shot(true);
                timer.set_wait_time(CLEAR_DELAY_SECS);
                let callable = self.base().callable("on_clear_timer");
                timer.connect("timeout", &callable);
                self.base_mut().add_child(&timer);
                self.clear_timer = Some(timer);
            },
            (),
        );
    }

    fn draw(&mut self) {
        panic_guard(
            "operator_debugger::draw",
            || {
                if self.highlights.is_empty() {
                    return;
                }
                // Semi-transparent blue -- distinct from search highlights (yellow)
                // and inccommand previews (green) so overlapping debug sessions are
                // visually unambiguous.
                let color = Color::from_rgba(0.3, 0.5, 1.0, 0.25);
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
impl DebugRangeOverlay {
    #[func]
    fn on_clear_timer(&mut self) {
        panic_guard(
            "operator_debugger::on_clear_timer",
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

impl DebugRangeOverlay {
    /// Highlight the text range affected by the last operation. Pixel
    /// positions come from `get_rect_at_line_column` (shared with the
    /// inccommand overlay via `geometry::compute_highlight_rects`).
    pub(crate) fn show_range(
        &mut self,
        start: CharLineCol,
        end: CharLineCol,
        editor: &Gd<CodeEdit>,
    ) {
        self.highlights.clear();

        let rects =
            super::geometry::compute_highlight_rects(editor, &start, &end, MAX_HIGHLIGHT_RECTS);
        self.highlights
            .extend(rects.into_iter().map(|rect| RangeHighlight { rect }));

        self.base_mut().queue_redraw();

        if let Some(ref mut timer) = self.clear_timer {
            timer.start();
        }
    }

    pub(crate) fn clear_highlights(&mut self) {
        if self.highlights.is_empty() {
            return;
        }
        self.highlights.clear();
        self.base_mut().queue_redraw();
    }
}
