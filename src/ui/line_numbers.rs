//! Vim-style line number gutter — relative, absolute, or hybrid numbering with fold icons.
//!
//! Replaces Godot's built-in line number and fold gutters with custom equivalents
//! that support relative/hybrid line numbering (like Vim's `set relativenumber`).
//!
//! # Architecture
//!
//! `LineNumberManager` is a headless `Node` that attaches to a `CodeEdit` via
//! [`LineNumberManager::attach`]. It:
//!
//! 1. Disables the editor's built-in line-number and fold gutters.
//! 2. Creates two custom gutters: a STRING gutter for line numbers and an ICON
//!    gutter for fold indicators.
//! 3. Connects to editor signals (caret, text, scroll, theme, visibility, gutter
//!    click) to keep the gutters up to date.
//!
//! # Dirty-flag optimization
//!
//! Not every signal requires a full gutter repaint:
//! - **Absolute mode**: only scroll or text changes matter.
//! - **Relative/Hybrid mode**: vertical caret movement also triggers an update,
//!   but horizontal-only movement (same line) is skipped.
//! - The `force_next_update` flag bypasses all checks for text changes, theme
//!   changes, and initial setup.

use godot::classes::image::Format;
use godot::classes::text_edit::GutterType;
use godot::classes::{CodeEdit, EditorInterface, INode, Image, ImageTexture, Node, Texture2D};
use godot::global::HorizontalAlignment;
use godot::prelude::*;

use crate::safety::panic_guard;
use crate::settings::LineNumberMode;

// Signal names — constants prevent silent runtime failures from typos.
const SIG_CARET_CHANGED: &str = "caret_changed";
const SIG_TEXT_CHANGED: &str = "text_changed";
const SIG_VALUE_CHANGED: &str = "value_changed";
const SIG_GUTTER_CLICKED: &str = "gutter_clicked";
const SIG_THEME_CHANGED: &str = "theme_changed";
const SIG_VISIBILITY_CHANGED: &str = "visibility_changed";

/// Extra lines beyond `get_last_full_visible_line()` to render in the gutter.
/// +1 for exclusive range, +5 for smooth scroll lookahead.
const VISIBLE_LINE_BUFFER: i32 = 6;

// Custom gutter names registered on the CodeEdit. Used for both creation
// and name-based index lookup (indices go stale across editor recycling).
const LINE_GUTTER_NAME: &str = "VimRelativeNumbers";
const FOLD_GUTTER_NAME: &str = "VimFoldGutter";

// ─────────────────────────────────────────────────────────────────────────────
// GodotClass
// ─────────────────────────────────────────────────────────────────────────────

/// Manages custom line-number and fold-indicator gutters on a `CodeEdit`.
///
/// Designed to be a child `Node` of the plugin controller. Call [`attach`] to
/// bind to a `CodeEdit` and [`detach`] to unbind. The manager handles all
/// signal connections internally.
///
/// [`attach`]: LineNumberManager::attach
/// [`detach`]: LineNumberManager::detach
#[derive(GodotClass)]
#[class(base = Node)]
pub struct LineNumberManager {
    base: Base<Node>,

    editor: Option<Gd<CodeEdit>>,

    /// -1 = not yet created. Looked up by name each frame (see `refresh_gutter_indices`).
    line_gutter_index: i32,
    /// -1 = not yet created. Looked up by name each frame (see `refresh_gutter_indices`).
    fold_gutter_index: i32,

    /// 1x1 transparent texture for non-foldable lines (prevents gutter collapse).
    empty_icon: Option<Gd<Texture2D>>,

    mode: LineNumberMode,

    /// Whether Godot's native `show_line_numbers` EditorSetting was enabled
    /// at attach time. When false, we skip custom gutter creation entirely
    /// and leave the native gutters untouched.
    native_line_numbers_enabled: bool,
    /// Whether Godot's native `show_code_folding_button` EditorSetting was
    /// enabled at attach time. Same early-exit logic as line numbers.
    native_fold_gutter_enabled: bool,

    // ── Dirty-flag state ────────────────────────────────────────────
    // These caches let us skip redundant gutter repaints. See the
    // module-level doc for the per-mode skip rules.
    last_line_count: i32,
    gutter_digits: usize,
    last_caret_line: i32,
    last_scroll_line: i32,
    force_next_update: bool,
}

// ─────────────────────────────────────────────────────────────────────────────
// INode lifecycle
// ─────────────────────────────────────────────────────────────────────────────

#[godot_api]
impl INode for LineNumberManager {
    fn init(base: Base<Node>) -> Self {
        Self {
            base,
            editor: None,
            line_gutter_index: -1,
            fold_gutter_index: -1,
            empty_icon: None,
            mode: LineNumberMode::default(),
            native_line_numbers_enabled: true,
            native_fold_gutter_enabled: true,
            last_line_count: -1,
            gutter_digits: 1,
            last_caret_line: -1,
            last_scroll_line: -1,
            force_next_update: true,
        }
    }

    // Defense-in-depth: disconnect signals on exit_tree.
    fn exit_tree(&mut self) {
        panic_guard(|| {
            self.detach();
        }, ());
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Public API + signal callbacks
// ─────────────────────────────────────────────────────────────────────────────

#[godot_api]
impl LineNumberManager {
    /// Bind to a `CodeEdit`, replacing its built-in gutters with custom ones.
    ///
    /// Idempotent for the same editor. When switching editors, the previous
    /// one is fully disconnected first. Connects six editor signals to keep
    /// the gutters synchronized with caret, scroll, text, theme, visibility,
    /// and fold-gutter click events.
    pub fn attach(&mut self, mut editor: Gd<CodeEdit>) {
        if let Some(current) = &self.editor {
            if current.is_instance_valid() && current.instance_id() == editor.instance_id() {
                return;
            }
            if current.is_instance_valid() {
                self.disconnect_signals(current.clone());
            }
        }

        self.editor = Some(editor.clone());

        // ── Snapshot native gutter settings ────────────────────────────────
        // Read Godot's EditorSettings to check if the user had line numbers
        // and/or fold gutter enabled natively. If disabled, we respect that
        // choice and skip custom gutter creation entirely.
        let (native_ln, native_fold) = read_native_gutter_settings();
        self.native_line_numbers_enabled = native_ln;
        self.native_fold_gutter_enabled = native_fold;

        // ── Connect signals ────────────────────────────────────────────────

        let callable_caret = self.base().callable("on_caret_changed");
        if !editor.is_connected(SIG_CARET_CHANGED, &callable_caret) {
            editor.connect(SIG_CARET_CHANGED, &callable_caret);
        }

        let callable_text_changed = self.base().callable("on_text_changed");
        if !editor.is_connected(SIG_TEXT_CHANGED, &callable_text_changed) {
            editor.connect(SIG_TEXT_CHANGED, &callable_text_changed);
        }

        // Scroll bar is a separate node from the editor; connect its signal directly.
        if let Some(mut scroll) = editor.get_v_scroll_bar() {
            let callable_scroll = self.base().callable("on_scroll_changed");
            if !scroll.is_connected(SIG_VALUE_CHANGED, &callable_scroll) {
                scroll.connect(SIG_VALUE_CHANGED, &callable_scroll);
            }
        }

        let callable_click = self.base().callable("on_gutter_clicked");
        if !editor.is_connected(SIG_GUTTER_CLICKED, &callable_click) {
            editor.connect(SIG_GUTTER_CLICKED, &callable_click);
        }

        let callable_theme = self.base().callable("on_theme_changed");
        if !editor.is_connected(SIG_THEME_CHANGED, &callable_theme) {
            editor.connect(SIG_THEME_CHANGED, &callable_theme);
        }

        // Floating windows can re-enable built-in gutters on visibility toggle,
        // so we force a full repaint when the editor becomes visible again.
        let callable_visibility = self.base().callable("on_visibility_changed");
        if !editor.is_connected(SIG_VISIBILITY_CHANGED, &callable_visibility) {
            editor.connect(SIG_VISIBILITY_CHANGED, &callable_visibility);
        }

        self.setup_gutters();
        self.force_next_update = true;
        self.update_gutters();
    }

    /// Set the line number display mode (Absolute / Relative / Hybrid / None).
    ///
    /// Forces a repaint and invalidates the width cache, because switching
    /// away from None (which collapses the gutter to zero width) requires
    /// re-measuring the font to restore the correct width.
    pub fn set_mode(&mut self, mode: LineNumberMode) {
        if self.mode != mode {
            self.mode = mode;
            self.force_next_update = true;
            self.last_line_count = -1;
        }
    }

    /// Detach from the current editor, restoring its native gutters.
    ///
    /// Only re-enables each native gutter if it was originally enabled before
    /// we attached. This respects the user's Godot EditorSettings choices.
    pub fn detach(&mut self) {
        if let Some(mut editor) = self.editor.take() {
            if editor.is_instance_valid() {
                self.disconnect_signals(editor.clone());
                self.clear_custom_gutters(editor.clone());
                if self.native_line_numbers_enabled {
                    editor.set_draw_line_numbers(true);
                }
                if self.native_fold_gutter_enabled {
                    editor.set_draw_fold_gutter(true);
                }
            }
        }
    }

    // ─────────────────────────────────────────────────────────────────────────
    // Signal callbacks (exposed to Godot via #[func])
    // ─────────────────────────────────────────────────────────────────────────

    #[func]
    pub fn on_gutter_clicked(&mut self, line: i32, gutter: i32) {
        panic_guard(
            || {
                if gutter != self.fold_gutter_index {
                    return;
                }
                let Some(mut editor) = self.editor.clone() else {
                    return;
                };
                if !editor.is_instance_valid() {
                    self.editor = None;
                    return;
                }
                editor.toggle_foldable_line(line);
                self.force_next_update = true;
                self.update_gutters();
            },
            (),
        );
    }

    /// Invalidate font metrics cache so the gutter width is re-measured with
    /// the new theme's font.
    #[func]
    pub fn on_theme_changed(&mut self) {
        panic_guard(
            || {
                self.last_line_count = -1;
                self.force_next_update = true;
                self.update_gutters();
            },
            (),
        );
    }

    #[func]
    pub fn on_scroll_changed(&mut self, _value: f64) {
        panic_guard(|| self.update_gutters(), ());
    }

    /// Force flag ensures Absolute mode also updates when lines are
    /// inserted/deleted (it normally only updates on scroll).
    #[func]
    pub fn on_text_changed(&mut self) {
        panic_guard(
            || {
                self.force_next_update = true;
                self.update_gutters();
            },
            (),
        );
    }

    /// Deferred (not immediate) so the caret position is settled before we
    /// read it. Does NOT set force_next_update, preserving Absolute mode's
    /// dirty-flag optimization (absolute numbers are caret-independent).
    #[func]
    pub fn on_caret_changed(&mut self) {
        panic_guard(
            || {
                self.base_mut().call_deferred("update_gutters", &[]);
            },
            (),
        );
    }

    /// Forced deferred update -- floating windows can re-enable Godot's
    /// built-in gutters on visibility toggle, so we must re-suppress them.
    #[func]
    pub fn on_visibility_changed(&mut self) {
        panic_guard(
            || {
                self.force_next_update = true;
                self.base_mut().call_deferred("update_gutters", &[]);
            },
            (),
        );
    }

    /// Core gutter render: compute the visible line range, set gutter text,
    /// colors, and fold icons.
    ///
    /// Implements dirty-flag optimization to skip redundant updates:
    /// - **Absolute**: only updates on scroll or text change (force flag).
    /// - **Relative / Hybrid**: also updates on vertical caret movement.
    /// - **None**: skips line number updates; fold icons still processed on
    ///   forced updates.
    /// - Horizontal-only caret movement (same line) is always skipped.
    /// Wrapped in `panic_guard` because Godot invokes this as a deferred
    /// callback via `call_deferred("update_gutters", &[])`.
    #[func]
    pub fn update_gutters(&mut self) {
        panic_guard(|| self.update_gutters_impl(), ());
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Private helpers
// ─────────────────────────────────────────────────────────────────────────────

impl LineNumberManager {
    /// Re-scan gutter indices by name on the live editor.
    ///
    /// Cached indices go stale when Godot recycles editors (e.g. during
    /// startup or hot-reload), so we resolve by name every update rather
    /// than trusting stored indices. Returns false if neither custom gutter
    /// exists on this editor.
    fn refresh_gutter_indices(&mut self, editor: &Gd<CodeEdit>) -> bool {
        let count = editor.get_gutter_count();
        self.line_gutter_index = -1;
        self.fold_gutter_index = -1;

        for i in 0..count {
            let name = editor.get_gutter_name(i);
            if name == LINE_GUTTER_NAME.into() {
                self.line_gutter_index = i;
            } else if name == FOLD_GUTTER_NAME.into() {
                self.fold_gutter_index = i;
            }
        }

        self.line_gutter_index != -1 || self.fold_gutter_index != -1
    }

    fn update_gutters_impl(&mut self) {
        let Some(mut editor) = self.editor.clone() else {
            return;
        };
        if !editor.is_instance_valid() {
            self.editor = None;
            return;
        }

        if !self.refresh_gutter_indices(&editor) {
            return;
        }

        // Floating windows can re-enable Godot's built-in gutters; suppress them.
        // Only re-suppress if we actually replaced them (native was originally on).
        if self.native_line_numbers_enabled && editor.is_draw_line_numbers_enabled() {
            editor.set_draw_line_numbers(false);
        }
        if self.native_fold_gutter_enabled && editor.is_drawing_fold_gutter() {
            editor.set_draw_fold_gutter(false);
        }

        let caret_line = editor.get_caret_line();
        let first_line = editor.get_first_visible_line();

        // ── Dirty-flag optimization ────────────────────────────────────────
        // Skip redundant repaints based on what actually changed:
        // - Absolute: only scroll/text changes matter (caret is irrelevant)
        // - Relative/Hybrid: vertical caret movement also triggers update
        // - None: never needs line number updates (fold icons use force flag)
        if !self.force_next_update {
            let scroll_changed = first_line != self.last_scroll_line;
            let caret_changed = caret_line != self.last_caret_line;

            if !scroll_changed {
                match self.mode {
                    LineNumberMode::Absolute => return,
                    LineNumberMode::Relative | LineNumberMode::Hybrid => {
                        if !caret_changed {
                            return;
                        }
                    }
                    LineNumberMode::None => return,
                }
            }
        }

        self.force_next_update = false;
        self.last_caret_line = caret_line;
        self.last_scroll_line = first_line;

        let line_count = editor.get_line_count();
        // get_last_full_visible_line() accounts for folded/hidden lines,
        // unlike get_visible_line_count() which only returns viewport rows.
        let last_line = std::cmp::min(editor.get_last_full_visible_line() + VISIBLE_LINE_BUFFER, line_count);

        let can_fold_icon = editor.get_theme_icon("can_fold");
        let folded_icon = editor.get_theme_icon("folded");
        let line_number_color = editor.get_theme_color("line_number_color");

        // None mode collapses the gutter to zero width (Vim `set nonumber
        // norelativenumber`) but still renders fold icons.
        let show_numbers = self.mode != LineNumberMode::None;

        if self.line_gutter_index != -1 {
            if show_numbers {
                self.update_gutter_width(&mut editor);
            } else {
                editor.set_gutter_width(self.line_gutter_index, 0);
            }
        }

        // Single reusable buffer avoids per-line heap allocation from format!().
        let mut num_buf = String::with_capacity(self.gutter_digits + 4);

        for line in first_line..last_line {
            if show_numbers && self.line_gutter_index != -1 {
                num_buf.clear();
                self.write_number(&mut num_buf, line, caret_line);
                editor.set_line_gutter_text(line, self.line_gutter_index, &num_buf);
                editor.set_line_gutter_item_color(line, self.line_gutter_index, line_number_color);
            }

            if self.fold_gutter_index != -1 {
                if editor.is_line_folded(line) {
                    if let Some(icon) = &folded_icon {
                        editor.set_line_gutter_icon(line, self.fold_gutter_index, icon);
                    }
                    editor.set_line_gutter_clickable(line, self.fold_gutter_index, true);
                } else if editor.can_fold_line(line) {
                    if let Some(icon) = &can_fold_icon {
                        editor.set_line_gutter_icon(line, self.fold_gutter_index, icon);
                    }
                    editor.set_line_gutter_clickable(line, self.fold_gutter_index, true);
                } else {
                    // Non-foldable lines get a transparent 1x1 icon to maintain
                    // consistent gutter width (Godot collapses icon gutters with
                    // no icon set).
                    let empty_tex = self.get_empty_icon();
                    editor.set_line_gutter_icon(line, self.fold_gutter_index, &empty_tex);
                    editor.set_line_gutter_clickable(line, self.fold_gutter_index, false);
                }
            }
        }
    }

    /// Format a line number into `buf` per the current mode.
    ///
    /// Hybrid: current line = absolute (1-based), others = relative distance.
    /// Relative: current line = "0", others = relative distance.
    /// Absolute: always 1-based line number.
    fn write_number(&self, buf: &mut String, line_idx: i32, caret_line: i32) {
        use std::fmt::Write;
        let gutter_width = self.gutter_digits;

        if self.mode == LineNumberMode::Absolute {
            let _ = write!(buf, "{:>gutter_width$}", line_idx + 1);
            return;
        }

        let diff = (line_idx - caret_line).abs();
        if diff == 0 {
            if self.mode == LineNumberMode::Hybrid {
                let _ = write!(buf, "{:>gutter_width$}", line_idx + 1);
            } else {
                let _ = write!(buf, "{:>gutter_width$}", 0);
            }
        } else {
            let _ = write!(buf, "{diff:>gutter_width$}");
        }
    }

    /// Lazily create and cache a 1x1 transparent texture for non-foldable lines.
    fn get_empty_icon(&mut self) -> Gd<Texture2D> {
        if let Some(tex) = &self.empty_icon {
            return tex.clone();
        }

        let Some(mut img) = Image::create(1, 1, false, Format::RGBA8) else {
            log::warn!("Failed to create empty icon image, using default texture");
            return Texture2D::new_gd();
        };
        img.fill(Color::from_rgba(0.0, 0.0, 0.0, 0.0));

        let Some(tex) = ImageTexture::create_from_image(&img) else {
            log::warn!("Failed to create texture from image, using default texture");
            return Texture2D::new_gd();
        };
        let tex_up: Gd<Texture2D> = tex.upcast();

        self.empty_icon = Some(tex_up.clone());
        tex_up
    }

    /// Recalculate gutter pixel width from the theme font's "0" width times
    /// digit count. Skipped when the line count is unchanged (common case).
    fn update_gutter_width(&mut self, editor: &mut Gd<CodeEdit>) {
        if self.line_gutter_index == -1 {
            return;
        }

        let line_count = editor.get_line_count();
        if line_count == self.last_line_count {
            return;
        }
        self.last_line_count = line_count;

        let digits = if line_count > 0 {
            (line_count as f64).log10().floor() as i32 + 1
        } else {
            1
        };
        self.gutter_digits = digits as usize;

        let Some(font) = editor.get_theme_font("font") else {
            log::warn!("Failed to get editor font for gutter width calculation");
            return;
        };
        let font_size = editor.get_theme_font_size("font_size");
        let char_width = font
            .get_string_size_ex("0")
            .alignment(HorizontalAlignment::LEFT)
            .font_size(font_size)
            .done()
            .x;

        // (digits + 1) matches Godot's built-in gutter formula; the extra
        // digit provides padding between numbers and the fold gutter.
        let total_width = (digits + 1) as f32 * char_width;
        if !total_width.is_finite() {
            return;
        }
        editor.set_gutter_width(self.line_gutter_index, total_width as i32);
    }

    /// Disable built-in gutters and create (or reuse) custom STRING + ICON
    /// gutters. Reuses existing gutters by name to avoid duplicates when
    /// re-attaching to the same editor (e.g. after hot-reload).
    ///
    /// Respects native Godot settings: if the user disabled line numbers or
    /// the fold gutter in EditorSettings, we skip the corresponding custom
    /// gutter entirely rather than overriding their preference.
    fn setup_gutters(&mut self) {
        let Some(mut editor) = self.editor.clone() else {
            return;
        };

        // Only disable native gutters that we're going to replace with
        // custom ones. If the user had them off in EditorSettings, leave
        // them off and skip custom gutter creation.
        if self.native_line_numbers_enabled {
            editor.set_draw_line_numbers(false);
        }
        if self.native_fold_gutter_enabled {
            editor.set_draw_fold_gutter(false);
        }

        self.line_gutter_index = -1;
        self.fold_gutter_index = -1;

        let count = editor.get_gutter_count();
        for i in 0..count {
            let name = editor.get_gutter_name(i);
            if name == LINE_GUTTER_NAME.into() {
                self.line_gutter_index = i;
            } else if name == FOLD_GUTTER_NAME.into() {
                self.fold_gutter_index = i;
            }
        }

        if self.native_line_numbers_enabled && self.line_gutter_index == -1 {
            editor.add_gutter();
            self.line_gutter_index = editor.get_gutter_count() - 1;
            editor.set_gutter_name(self.line_gutter_index, LINE_GUTTER_NAME);
            editor.set_gutter_type(self.line_gutter_index, GutterType::STRING);
        }

        if self.native_fold_gutter_enabled && self.fold_gutter_index == -1 {
            editor.add_gutter();
            self.fold_gutter_index = editor.get_gutter_count() - 1;
            editor.set_gutter_name(self.fold_gutter_index, FOLD_GUTTER_NAME);
            editor.set_gutter_type(self.fold_gutter_index, GutterType::ICON);
            editor.set_gutter_width(self.fold_gutter_index, 16);
        }
    }

    /// Remove custom gutters from the editor by scanning for their names.
    ///
    /// Uses name-based lookup instead of cached indices because Godot may add
    /// or remove gutters between attach and detach (e.g., during startup editor
    /// swaps or hot-reload), invalidating stored indices. Scanning in reverse
    /// avoids index-shift issues when removing multiple gutters.
    fn clear_custom_gutters(&mut self, mut editor: Gd<CodeEdit>) {
        for i in (0..editor.get_gutter_count()).rev() {
            let name = editor.get_gutter_name(i);
            if name == LINE_GUTTER_NAME.into() || name == FOLD_GUTTER_NAME.into() {
                editor.remove_gutter(i);
            }
        }
        self.line_gutter_index = -1;
        self.fold_gutter_index = -1;
    }

    /// Disconnect all six signal connections established by `attach`.
    fn disconnect_signals(&mut self, mut editor: Gd<CodeEdit>) {
        let callable_caret = self.base().callable("on_caret_changed");
        if editor.is_connected(SIG_CARET_CHANGED, &callable_caret) {
            editor.disconnect(SIG_CARET_CHANGED, &callable_caret);
        }

        let callable_text_changed = self.base().callable("on_text_changed");
        if editor.is_connected(SIG_TEXT_CHANGED, &callable_text_changed) {
            editor.disconnect(SIG_TEXT_CHANGED, &callable_text_changed);
        }

        let callable_visibility = self.base().callable("on_visibility_changed");
        if editor.is_connected(SIG_VISIBILITY_CHANGED, &callable_visibility) {
            editor.disconnect(SIG_VISIBILITY_CHANGED, &callable_visibility);
        }

        let callable_click = self.base().callable("on_gutter_clicked");
        if editor.is_connected(SIG_GUTTER_CLICKED, &callable_click) {
            editor.disconnect(SIG_GUTTER_CLICKED, &callable_click);
        }

        let callable_theme = self.base().callable("on_theme_changed");
        if editor.is_connected(SIG_THEME_CHANGED, &callable_theme) {
            editor.disconnect(SIG_THEME_CHANGED, &callable_theme);
        }

        // Scroll bar is a separate node; disconnect its signal independently.
        if let Some(mut scroll) = editor.get_v_scroll_bar() {
            let callable_scroll = self.base().callable("on_scroll_changed");
            if scroll.is_connected(SIG_VALUE_CHANGED, &callable_scroll) {
                scroll.disconnect(SIG_VALUE_CHANGED, &callable_scroll);
            }
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Free functions
// ─────────────────────────────────────────────────────────────────────────────

/// Read the native gutter visibility settings from Godot's `EditorSettings`.
///
/// Returns `(show_line_numbers, show_code_folding_button)`. Defaults to
/// `(true, true)` if the settings are unavailable (e.g. during unit tests or
/// if the EditorSettings singleton hasn't been created yet).
fn read_native_gutter_settings() -> (bool, bool) {
    let Some(settings) = EditorInterface::singleton().get_editor_settings() else {
        return (true, true);
    };

    let show_line_numbers = read_bool_setting(
        &settings,
        "text_editor/appearance/gutters/show_line_numbers",
        true,
    );
    let show_fold_gutter = read_bool_setting(
        &settings,
        "text_editor/appearance/gutters/show_code_folding_button",
        true,
    );

    (show_line_numbers, show_fold_gutter)
}

/// Read a single boolean from `EditorSettings`, falling back to `default` if
/// the key doesn't exist or the value isn't convertible to `bool`.
fn read_bool_setting(
    settings: &godot::classes::EditorSettings,
    key: &str,
    default: bool,
) -> bool {
    if settings.has_setting(key) {
        settings
            .get_setting(key)
            .try_to::<bool>()
            .unwrap_or(default)
    } else {
        default
    }
}
