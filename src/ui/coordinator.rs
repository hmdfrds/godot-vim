//! Lifecycle manager for all Vim UI overlays on a `CodeEdit`.
//!
//! Each overlay (cursor, status bar, line numbers, search highlights, etc.) is
//! a separate Godot node or plain-Rust struct. This coordinator owns them all
//! behind `Option<Gd<T>>` slots so that attach/detach is atomic: either all
//! overlays exist, or none do. The `update()` method is called once per
//! keystroke with a `UiSnapshot` and fans out to each subsystem.

use compact_str::CompactString;
use godot::classes::control::{LayoutPreset, MouseFilter};
use godot::classes::text_edit::CaretType;
use godot::classes::{CodeEdit, Control, Node};
use godot::obj::NewAlloc;
use godot::prelude::*;

use vim_core::primitives::{CommandLinePrompt, Direction, Mode};

use super::cursor_shape::{compute_cursor_geometry, VimCursor};
use super::CursorColorMap;
use super::highlight_yank::HighlightYankOverlay;
use super::inccommand::InccommandOverlay;
use super::operator_debugger::DebugRangeOverlay;
use super::line_numbers::LineNumberManager;
use super::search_hl::SearchHighlighter;
use super::status_bar::{self, VimStatusBar};
use super::virtual_text::VirtualTextOverlay;
use crate::types::{CharLineCol, StatusMessage, UiSnapshot};

// ─────────────────────────────────────────────────────────────────────────────
// Sub-structs
// ─────────────────────────────────────────────────────────────────────────────

/// Editor properties we override on attach and must restore on detach.
/// `None` fields mean the editor had no explicit override (theme-inherited),
/// so detach should remove our override rather than setting a value.
#[derive(Default)]
struct SavedEditorState {
    caret_color: Option<Color>,
    highlight_line: Option<bool>,
    line_highlight_color: Option<Color>,
    caret_type: Option<CaretType>,
}

/// Per-frame dirty cache. Prevents redundant overlay repaints by comparing
/// the current snapshot against the previous one.
struct DirtyCache {
    last_mode: Option<Mode>,
    /// Stores the full value (not just is_none) so an unchanged persistent
    /// message like `Info("3 lines yanked")` doesn't trigger a repaint every
    /// keystroke.
    last_message: StatusMessage,
    last_recording: Option<char>,
    last_pending_command: Option<CompactString>,
    last_pending_keys: Option<CompactString>,
    cached_visual_head: Option<CharLineCol>,
}

impl Default for DirtyCache {
    fn default() -> Self {
        Self {
            last_mode: None,
            last_message: StatusMessage::None,
            last_recording: None,
            last_pending_command: None,
            last_pending_keys: None,
            cached_visual_head: None,
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// UiCoordinator
// ─────────────────────────────────────────────────────────────────────────────

pub(crate) struct UiCoordinator {
    status_bar: Option<Gd<VimStatusBar>>,
    cursor: Option<Gd<VimCursor>>,
    line_numbers: Option<Gd<LineNumberManager>>,
    /// Plain Rust -- not a Godot node. Drives CodeEdit's built-in search API.
    search_hl: SearchHighlighter,
    inccommand: Option<Gd<InccommandOverlay>>,
    saved: SavedEditorState,
    /// When false, the native Godot caret is used instead of VimCursor.
    /// Toggled by user settings; requires swapping caret_color overrides.
    cursor_enabled: bool,
    cache: DirtyCache,
    inccommand_enabled: bool,
    debug_overlay: Option<Gd<DebugRangeOverlay>>,
    virtual_text: Option<Gd<VirtualTextOverlay>>,
    highlight_yank: Option<Gd<HighlightYankOverlay>>,
}

impl UiCoordinator {
    #[must_use]
    pub(crate) fn new() -> Self {
        Self {
            status_bar: None,
            cursor: None,
            line_numbers: None,
            search_hl: SearchHighlighter::new(),
            inccommand: None,
            saved: SavedEditorState::default(),
            cursor_enabled: true,
            cache: DirtyCache::default(),
            inccommand_enabled: true,
            debug_overlay: None,
            virtual_text: None,
            highlight_yank: None,
        }
    }

    /// Inject all UI overlays into `editor`. Also hides the native caret and
    /// cleans up orphaned overlay nodes from a previous session that didn't
    /// detach cleanly (e.g. editor reload, crash recovery).
    pub(crate) fn attach(&mut self, editor: &mut Gd<CodeEdit>) {
        log::debug!("ui::attach: editor=#{}", editor.instance_id().to_i64());
        // ── 1. Status bar ────────────────────────────────────────────────
        let bar = VimStatusBar::new_alloc();
        status_bar::inject_status_bar(editor, &bar);
        self.status_bar = Some(bar);

        // ── 2. Remove orphaned VimCursor children ────────────────────────
        {
            let editor_node = editor.clone().upcast::<Node>();
            for child in editor_node.get_children().iter_shared() {
                let name = child.get_name().to_string();
                if name.starts_with("VimCursor") {
                    log::warn!("ui::attach: removing orphaned VimCursor {}", name);
                    let mut orphan = child;
                    if let Some(mut parent) = orphan.get_parent() {
                        parent.remove_child(&orphan);
                    }
                    orphan.queue_free();
                }
            }
        }

        // ── 3. Cursor ────────────────────────────────────────────────────
        let mut cursor = VimCursor::new_alloc();
        editor.add_child(&cursor.clone().upcast::<Node>());
        // Snap immediately -- without this the cursor lerps from (0,0).
        if let Some(geom) = compute_cursor_geometry(editor, None) {
            let mut vim_cursor = cursor.bind_mut();
            vim_cursor.set_target(geom.pos, geom.height, geom.width);
            vim_cursor.force_snap();
        }
        self.cursor = Some(cursor);
        self.cursor_enabled = true;

        // ── 4. Hide native caret ─────────────────────────────────────────
        // Save the existing override (or None for theme-inherited) so detach
        // can restore the original behavior without leaking our transparent
        // override into future theme changes.
        self.saved.caret_color = if editor.has_theme_color_override("caret_color") {
            Some(editor.get_theme_color("caret_color"))
        } else {
            None
        };
        editor.add_theme_color_override(
            "caret_color",
            Color::from_rgba(0.0, 0.0, 0.0, 0.0),
        );

        // ── 4b. Cache caret type and line highlight state ────────────────
        self.saved.caret_type = Some(editor.get_caret_type());
        self.saved.highlight_line = Some(editor.is_highlight_current_line_enabled());
        self.saved.line_highlight_color =
            if editor.has_theme_color_override("current_line_color") {
                Some(editor.get_theme_color("current_line_color"))
            } else {
                None
            };

        // ── 5. Line numbers ─────────────────────────────────────────────
        let mut line_numbers = LineNumberManager::new_alloc();
        editor.add_child(&line_numbers.clone().upcast::<Node>());
        line_numbers.bind_mut().attach(editor.clone());
        self.line_numbers = Some(line_numbers);

        // ── 6. Inccommand overlay ────────────────────────────────────────
        self.inccommand = Some(create_overlay::<InccommandOverlay>(editor, 50));

        // ── 7. Debug range overlay ───────────────────────────────────────
        self.debug_overlay = Some(create_overlay::<DebugRangeOverlay>(editor, 60));

        // ── 8. Virtual text overlay ────────────────────────────────────────
        self.virtual_text = Some(create_overlay::<VirtualTextOverlay>(editor, 51));

        // ── 9. Highlight yank overlay ──────────────────────────────────────
        let mut hy_overlay = create_overlay::<HighlightYankOverlay>(editor, 52);
        hy_overlay.set_process(false);
        self.highlight_yank = Some(hy_overlay);
    }

    /// Remove all overlays and restore the editor to its pre-attach state.
    pub(crate) fn detach(&mut self, editor: &mut Gd<CodeEdit>) {
        log::debug!("ui::detach: editor=#{}", editor.instance_id().to_i64());
        // ── 1. Line numbers ──────────────────────────────────────────────
        if let Some(ln) = self.line_numbers.as_mut() {
            ln.bind_mut().detach();
        }
        remove_and_free(&mut self.line_numbers);

        // ── 2. Cursor ────────────────────────────────────────────────────
        remove_and_free(&mut self.cursor);

        if let Some(color) = self.saved.caret_color.take() {
            editor.add_theme_color_override("caret_color", color);
        } else {
            editor.remove_theme_color_override("caret_color");
        }

        if let Some(caret_type) = self.saved.caret_type.take() {
            editor.set_caret_type(caret_type);
        }

        if let Some(val) = self.saved.highlight_line.take() {
            editor.set_highlight_current_line(val);
        }
        if let Some(color) = self.saved.line_highlight_color.take() {
            editor.add_theme_color_override("current_line_color", color);
        } else {
            editor.remove_theme_color_override("current_line_color");
        }

        // ── 2b. Inccommand overlay ───────────────────────────────────────
        remove_and_free(&mut self.inccommand);

        // ── 2c. Debug range overlay ─────────────────────────────────────
        remove_and_free(&mut self.debug_overlay);

        // ── 2d. Virtual text overlay ───────────────────────────────────────
        remove_and_free(&mut self.virtual_text);

        // ── 2e. Highlight yank overlay ─────────────────────────────────────
        remove_and_free(&mut self.highlight_yank);

        // ── 3. Status bar ────────────────────────────────────────────────
        // The recording-indicator tween runs in an infinite loop. It must be
        // killed before freeing the node, or Godot keeps stepping it against
        // a freed panel (use-after-free in the tween list).
        if let Some(bar) = self.status_bar.as_mut() {
            bar.bind_mut().stop_recording_animation();
        }
        remove_and_free(&mut self.status_bar);

        // ── 4. Search highlighting ──────────────────────────────────────
        self.search_hl.clear(editor);

        // ── 5. Clear cached UI state ────────────────────────────────────
        self.cache = DirtyCache::default();
    }

    /// Hard reset after the parent editor has been freed externally.
    ///
    /// The editor (parent node) is already gone, so Godot auto-freed all child
    /// overlay nodes. We drop the stale `Gd<T>` references here to avoid holding
    /// dangling instance IDs, and reset the dirty cache so the next `update()`
    /// after reattach repaints unconditionally.
    pub(crate) fn reset_cached_state(&mut self) {
        self.cache = DirtyCache::default();
        self.saved = SavedEditorState::default();
        self.cursor_enabled = true;
        self.inccommand_enabled = true;
        self.search_hl = SearchHighlighter::new();
        self.status_bar.take();
        self.cursor.take();
        self.line_numbers.take();
        self.inccommand.take();
        self.debug_overlay.take();
        self.virtual_text.take();
        self.highlight_yank.take();
    }

    /// Refresh all overlays from the latest engine snapshot. Called once per
    /// keystroke.
    pub(crate) fn update(&mut self, snap: &UiSnapshot, editor: &mut Gd<CodeEdit>) {
        // Force re-shaping so character widths reflect any text changes.
        // Cost: ~4 FFI calls per keystroke (free + create + add_string +
        // tab_align on one line). Negligible.
        super::cursor_shape::invalidate_shaped_cache();

        // ── 1. Status bar ────────────────────────────────────────────────
        // Always repaint while command line is active (every character typed
        // changes display). Otherwise dirty-check individual fields.
        let mode_changed = self.cache.last_mode != Some(snap.mode);
        let message_changed = self.cache.last_message != snap.message;
        let recording_changed = self.cache.last_recording != snap.recording_register;
        let pending_changed = self.cache.last_pending_command.as_deref() != Some(snap.pending_command.as_str())
            || self.cache.last_pending_keys.as_deref() != Some(snap.pending_keys.as_str());
        let cmdline_active = snap.cmdline.prompt.is_some();
        let vimdebug_active = snap.vimdebug.is_active();

        if mode_changed || message_changed || recording_changed || pending_changed || cmdline_active || vimdebug_active {
            if let Some(ref mut bar) = self.status_bar {
                bar.bind_mut().update(snap);
            }
            self.cache.last_mode = Some(snap.mode);
            self.cache.last_message = snap.message.clone();
            self.cache.last_recording = snap.recording_register;
            self.cache.last_pending_command = Some(snap.pending_command.clone());
            self.cache.last_pending_keys = Some(snap.pending_keys.clone());
        }

        // ── 2. Cursor ────────────────────────────────────────────────────
        self.cache.cached_visual_head = snap.visual_head;
        if let Some(ref mut cursor) = self.cursor {
            let mut vim_cursor = cursor.bind_mut();
            // Always push mode even without geometry, so the shape doesn't
            // get stuck (e.g. beam lingering after insert-repeat exit).
            vim_cursor.set_mode(snap.mode);
            // visual_head: in visual mode, Godot's caret is at the exclusive
            // selection end, but Vim's cursor should render at the head.
            if let Some(geom) = compute_cursor_geometry(editor, snap.visual_head) {
                vim_cursor.set_target(geom.pos, geom.height, geom.width);
            } else {
                log::trace!("cursor geometry unavailable (folded/offscreen?), keeping previous position");
            }
        }

        // ── 2b. Native caret type (when custom cursor is disabled) ──────
        if !self.cursor_enabled {
            editor.set_caret_type(mode_to_native_caret(snap.mode));
        }

        // ── 3. Search highlighting ──────────────────────────────────────
        // Incremental search (`incsearch`): while typing in `/` or `?`, use
        // the live cmdline input so matches highlight in real-time.
        let incsearch_pattern;
        let is_incsearch = matches!(
            snap.cmdline.prompt,
            Some(CommandLinePrompt::SearchForward) | Some(CommandLinePrompt::SearchBackward)
        );
        let effective_search = match snap.cmdline.prompt {
            Some(CommandLinePrompt::SearchForward) if !snap.cmdline.input.is_empty() => {
                incsearch_pattern = (snap.cmdline.input.clone(), Direction::Forward);
                Some(&incsearch_pattern)
            }
            Some(CommandLinePrompt::SearchBackward) if !snap.cmdline.input.is_empty() => {
                incsearch_pattern = (snap.cmdline.input.clone(), Direction::Backward);
                Some(&incsearch_pattern)
            }
            _ => snap.search_pattern.as_ref(),
        };
        // Incsearch with typed input overrides `:noh` -- Vim always shows
        // matches while typing. But an empty `/` prompt must NOT re-show
        // old highlights that were cleared by `:noh`.
        let has_incsearch_input = is_incsearch && !snap.cmdline.input.is_empty();
        let hlsearch = snap.hlsearch_enabled || has_incsearch_input;
        if let Some((pattern, _)) = effective_search {
            log::trace!("ui::update: search={} hlsearch={}", pattern.as_str(), hlsearch);
        }
        self.search_hl.update(editor, effective_search, hlsearch);

        // ── 4. Line numbers (signal-driven, no per-keystroke update) ────

        // ── 5. Inccommand preview ────────────────────────────────────────
        if let Some(ref positions) = snap.substitute_preview {
            if !self.inccommand_enabled || positions.is_empty() {
                self.clear_substitute_preview();
            } else {
                self.update_substitute_preview(positions, editor);
            }
        } else if mode_changed && !snap.mode.is_command_line() {
            // Safety net: clear stale preview on command-line exit even if
            // the engine didn't explicitly emit ClearSubstitutePreview.
            self.clear_substitute_preview();
        }

        // ── 6. Highlight yank ───────────────────────────────────────────
        if let Some(ref yank) = snap.highlight_yank {
            if yank.duration_ms > 0 {
                if let Some(ref mut overlay) = self.highlight_yank {
                    overlay.bind_mut().show_yank(yank.start, yank.end, yank.duration_ms, editor);
                }
            }
        }

        // ── 7. Debug range overlay ───────────────────────────────────────
        match snap.vimdebug.range() {
            Some(range) => {
                if let Some(ref mut overlay) = self.debug_overlay {
                    overlay.bind_mut().show_range(range.start, range.end, editor);
                }
            }
            // Only clear when vimdebug is fully inactive, not just range-less.
            None if !snap.vimdebug.is_active() => {
                self.clear_debug_overlay();
            }
            _ => {}
        }
    }

    /// Lightweight cursor-only refresh for scroll/draw signal handlers.
    /// Keeps the overlay in sync with the viewport between keystrokes.
    pub(crate) fn update_cursor_position(&mut self, editor: &Gd<CodeEdit>) {
        if let Some(ref mut cursor) = self.cursor {
            if let Some(geom) = compute_cursor_geometry(editor, self.cache.cached_visual_head) {
                cursor.bind_mut().set_target(geom.pos, geom.height, geom.width);
            } else {
                log::trace!("cursor_position: geometry unavailable, skipping update");
            }
        }
    }

    /// Push user settings into all relevant overlays. Called on attach
    /// and whenever settings are reloaded at runtime.
    pub(crate) fn apply_settings(
        &mut self,
        snapshot: &crate::settings::SettingsSnapshot,
        current_mode: Mode,
        editor: &mut Gd<CodeEdit>,
    ) {
        if let Some(ref mut cursor) = self.cursor {
            let mut vim_cursor = cursor.bind_mut();
            vim_cursor.set_color_map(CursorColorMap {
                normal: snapshot.cursor.normal,
                insert: snapshot.cursor.insert,
                visual: snapshot.cursor.visual,
                replace: snapshot.cursor.replace,
                operator: snapshot.cursor.operator,
                command: snapshot.cursor.command,
            });
            // Force repaint now so color is correct before the next keystroke.
            vim_cursor.set_mode(current_mode);

            vim_cursor.set_animation(snapshot.cursor.lerp_speed, snapshot.cursor.blink_speed);
            vim_cursor.set_dimensions(
                snapshot.cursor.beam_width as f32,
                snapshot.cursor.underline_height as f32,
            );
        }

        // ── Cursor enable/disable toggle ────────────────────────────────
        // Swaps between the custom VimCursor overlay and Godot's native caret.
        // The native caret was made transparent on attach; toggling requires
        // restoring/re-hiding the caret_color override.
        if snapshot.cursor.enabled != self.cursor_enabled {
            if !snapshot.cursor.enabled {
                if let Some(ref mut cursor) = self.cursor {
                    cursor.set_visible(false);
                }
                if let Some(color) = self.saved.caret_color {
                    editor.add_theme_color_override("caret_color", color);
                } else {
                    editor.remove_theme_color_override("caret_color");
                }
                editor.set_caret_type(mode_to_native_caret(current_mode));
            } else {
                editor.add_theme_color_override(
                    "caret_color",
                    Color::from_rgba(0.0, 0.0, 0.0, 0.0),
                );
                if let Some(ref mut cursor) = self.cursor {
                    cursor.set_visible(true);
                }
            }
            self.cursor_enabled = snapshot.cursor.enabled;
        }

        if snapshot.cursor.line_highlight_enabled {
            editor.set_highlight_current_line(true);
            editor.add_theme_color_override(
                "current_line_color",
                snapshot.cursor.line_highlight_color,
            );
        } else {
            editor.set_highlight_current_line(
                self.saved.highlight_line.unwrap_or(false),
            );
            if let Some(color) = self.saved.line_highlight_color {
                editor.add_theme_color_override("current_line_color", color);
            } else {
                editor.remove_theme_color_override("current_line_color");
            }
        }

        if let Some(ref mut ln) = self.line_numbers {
            ln.bind_mut().set_mode(snapshot.line_number_mode);
        }

        self.inccommand_enabled = snapshot.inccommand.is_enabled();
        if !self.inccommand_enabled {
            self.clear_substitute_preview();
        }

        if let Some(ref mut bar) = self.status_bar {
            bar.bind_mut().apply_colors(&snapshot.status_bar);
        }
    }

    pub(crate) fn update_substitute_preview(
        &mut self,
        positions: &[crate::types::MatchRange],
        editor: &Gd<CodeEdit>,
    ) {
        if let Some(ref mut overlay) = self.inccommand {
            overlay.bind_mut().update_matches(positions, editor);
        }
    }

    pub(crate) fn clear_substitute_preview(&mut self) {
        if let Some(ref mut overlay) = self.inccommand {
            overlay.bind_mut().clear_highlights();
        }
    }

    /// Recompute inccommand pixel rects from stored logical positions.
    ///
    /// Called from scroll/resize/draw signal handlers to keep highlights
    /// aligned with the viewport between keystrokes. No-op when no preview
    /// is active.
    pub(crate) fn recompute_inccommand_rects(&mut self, editor: &Gd<CodeEdit>) {
        if let Some(ref mut overlay) = self.inccommand {
            overlay.bind_mut().recompute_rects(editor);
        }
    }

    pub(crate) fn clear_debug_overlay(&mut self) {
        if let Some(ref mut overlay) = self.debug_overlay {
            overlay.bind_mut().clear_highlights();
        }
    }
}

impl Drop for UiCoordinator {
    fn drop(&mut self) {
        // Diagnostic only -- we can't queue_free() here because the parent
        // node may already be freed, making these Gd references dangling.
        if self.status_bar.is_some()
            || self.cursor.is_some()
            || self.line_numbers.is_some()
            || self.inccommand.is_some()
            || self.debug_overlay.is_some()
            || self.virtual_text.is_some()
            || self.highlight_yank.is_some()
        {
            log::warn!("UiCoordinator dropped with overlays still attached — detach() was not called");
        }
    }
}

/// Take a node out of `slot`, detach from parent, and `queue_free()`.
/// Callers needing pre-teardown cleanup (e.g. stopping tweens) must do so
/// via `slot.as_mut()` before calling this.
fn remove_and_free<T: GodotClass + Inherits<Node>>(slot: &mut Option<Gd<T>>) {
    if let Some(node) = slot.take() {
        let mut gd_node = node.upcast::<Node>();
        if let Some(mut parent) = gd_node.get_parent() {
            parent.remove_child(&gd_node);
        }
        gd_node.queue_free();
    }
}

/// Allocate an overlay Control, stretch to parent rect, make it mouse-
/// transparent, set z-index, and add as child of the editor.
fn create_overlay<T: NewAlloc + Inherits<Control> + Inherits<Node>>(
    editor: &mut Gd<CodeEdit>,
    z_index: i32,
) -> Gd<T> {
    let overlay = T::new_alloc();
    // gdext's Deref dispatch only works on concrete types, so upcast to
    // Control to access inherited configuration methods.
    let mut control = overlay.clone().upcast::<Control>();
    control.set_anchors_preset(LayoutPreset::FULL_RECT);
    control.set_mouse_filter(MouseFilter::IGNORE);
    control.set_z_index(z_index);
    editor.add_child(&overlay.clone().upcast::<Node>());
    overlay
}

/// Fallback when VimCursor is disabled: Insert -> line, everything else -> block.
fn mode_to_native_caret(mode: Mode) -> CaretType {
    match mode {
        Mode::Insert => CaretType::LINE,
        _ => CaretType::BLOCK,
    }
}
