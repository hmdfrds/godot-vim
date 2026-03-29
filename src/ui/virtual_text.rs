//! Virtual text overlay — renders inline text annotations alongside buffer
//! content without modifying the document.
//!
//! Not yet wired into the effect pipeline — reserved for future use.
//!
//! Architecture follows the same pattern as [`super::inccommand`]: a
//! `Control` node layered on top of `CodeEdit` that draws text annotations
//! at pixel positions from `get_rect_at_line_column`.

use std::collections::HashMap;

use compact_str::CompactString;
use godot::classes::{CodeEdit, Control, IControl};
use godot::prelude::*;

use crate::safety::panic_guard;

// ── Constants ──────────────────────────────────────────────────────────────

/// Maximum total entries across all namespaces.
const MAX_ENTRIES: usize = 200;

// ── Data types ─────────────────────────────────────────────────────────────

/// Rendering position style for a virtual text entry.
#[derive(Debug, Clone, Copy, Default)]
#[allow(dead_code)] // Variants reserved for when engine emits virtual text effects
pub(crate) enum VirtualTextStyle {
    /// After the end of the line.
    #[default]
    Eol,
    /// Overlaid on top of existing text at the anchor position.
    Overlay,
    /// Right-aligned to the right edge of the editor.
    RightAlign,
}

/// A single virtual text annotation (logical coordinates).
#[derive(Debug, Clone)]
pub(crate) struct VirtualTextEntry {
    pub(crate) line: i32,
    pub(crate) col: i32,
    pub(crate) text: CompactString,
    pub(crate) namespace: CompactString,
    pub(crate) style: VirtualTextStyle,
}

/// Pre-computed draw command produced by `rebuild_draw_list`.
struct DrawCmd {
    rect: Rect2,
}

// ── GodotClass ─────────────────────────────────────────────────────────────

/// Overlay Control that renders virtual text annotations above `CodeEdit`.
///
/// Godot's `_draw()` callback has no access to the parent editor reference,
/// so text glyphs cannot be drawn at arbitrary positions. Instead, annotations
/// are rendered as subtle colored rectangles (matching the inccommand approach).
/// Pixel positions are pre-computed via `get_rect_at_line_column()` when
/// entries change and cached in `draw_list` for `_draw()` to consume.
#[derive(GodotClass)]
#[class(base=Control)]
pub(crate) struct VirtualTextOverlay {
    base: Base<Control>,
    /// Logical entries keyed by namespace for independent clear/update.
    entries: HashMap<CompactString, Vec<VirtualTextEntry>>,
    /// Pre-computed pixel rectangles consumed by `_draw()`.
    draw_list: Vec<DrawCmd>,
}

#[godot_api]
impl IControl for VirtualTextOverlay {
    fn init(base: Base<Control>) -> Self {
        Self {
            base,
            entries: HashMap::new(),
            draw_list: Vec::new(),
        }
    }

    fn draw(&mut self) {
        panic_guard(|| {
            if self.draw_list.is_empty() {
                return;
            }
            let color = Color::from_rgba(0.6, 0.5, 1.0, 0.15);
            // Index loop avoids iterator borrow conflict with base_mut().
            for i in 0..self.draw_list.len() {
                let rect = self.draw_list[i].rect;
                self.base_mut().draw_rect(rect, color);
            }
        }, ());
    }
}

#[godot_api]
impl VirtualTextOverlay {}

impl VirtualTextOverlay {
    /// Upsert a single entry (keyed by namespace + line + col) and rebuild draw list.
    #[allow(dead_code)] // API reserved for when engine emits virtual text effects
    pub(crate) fn set_entry(
        &mut self,
        entry: VirtualTextEntry,
        editor: &Gd<CodeEdit>,
    ) {
        self.enforce_capacity();

        let ns = entry.namespace.clone();
        let bucket = self.entries.entry(ns).or_default();

        let existing = bucket
            .iter()
            .position(|e| e.line == entry.line && e.col == entry.col);
        if let Some(idx) = existing {
            bucket[idx] = entry;
        } else {
            bucket.push(entry);
        }

        self.rebuild_draw_list(editor);
        self.base_mut().queue_redraw();
    }

    #[allow(dead_code)] // API reserved for when engine emits virtual text effects
    pub(crate) fn clear_namespace(&mut self, namespace: &str, editor: &Gd<CodeEdit>) {
        self.entries.remove(namespace);
        self.rebuild_draw_list(editor);
        self.base_mut().queue_redraw();
    }

    #[allow(dead_code)] // API reserved for when engine emits virtual text effects
    pub(crate) fn clear_all_entries(&mut self) {
        if self.entries.is_empty() {
            return;
        }
        self.entries.clear();
        self.draw_list.clear();
        self.base_mut().queue_redraw();
    }

    /// Convert logical entries to pixel-positioned draw commands.
    ///
    /// `char_width` is approximated as 60% of line_height since CodeEdit
    /// does not expose per-glyph metrics at this API level.
    fn rebuild_draw_list(&mut self, editor: &Gd<CodeEdit>) {
        self.draw_list.clear();

        let line_height = editor.get_line_height().max(1);
        let editor_width = editor.get_size().x;
        let char_width = (line_height as f32 * 0.6).max(1.0);

        for bucket in self.entries.values() {
            for entry in bucket {
                let Some(col_pos) = super::geometry::corrected_col_x(editor, entry.line, entry.col) else {
                    continue;
                };

                let text_width = entry.text.chars().count() as f32 * char_width;
                let x = match entry.style {
                    VirtualTextStyle::Eol => col_pos.x as f32 + 4.0,
                    VirtualTextStyle::Overlay => col_pos.x as f32,
                    VirtualTextStyle::RightAlign => {
                        (editor_width - text_width - 8.0).max(0.0)
                    }
                };

                self.draw_list.push(DrawCmd {
                    rect: Rect2::new(
                        Vector2::new(x, col_pos.y as f32),
                        Vector2::new(text_width, line_height as f32),
                    ),
                });
            }
        }
    }

    /// Evict the oldest entry from the largest namespace when at capacity.
    /// FIFO within the largest bucket prevents any single namespace from
    /// monopolizing the entry pool.
    fn enforce_capacity(&mut self) {
        let total: usize = self.entries.values().map(|v| v.len()).sum();
        if total < MAX_ENTRIES {
            return;
        }
        let largest = self
            .entries
            .iter()
            .max_by_key(|(_, v)| v.len())
            .map(|(k, _)| k.clone());
        if let Some(ns) = largest {
            if let Some(bucket) = self.entries.get_mut(&ns) {
                if !bucket.is_empty() {
                    bucket.remove(0);
                }
                if bucket.is_empty() {
                    self.entries.remove(&ns);
                }
            }
        }
    }
}
