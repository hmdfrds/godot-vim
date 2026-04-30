//! Vim status bar overlay — floating HBoxContainer pinned to the editor's bottom-right.
//!
//! Displays the current mode, command-line input, messages, and recording state.
//! Designed as a lightweight, self-contained widget: create with `VimStatusBar::new_alloc()`,
//! inject into a `CodeEdit` via [`inject_status_bar`], then call [`VimStatusBar::update`]
//! each frame with a fresh [`UiSnapshot`].
//!
//! # Layout
//!
//! ```text
//! ┌─ HBoxContainer ("VimStatusBar") ───────────────┐
//! │  ┌─ PanelContainer (colored bg) ─────────────┐  │
//! │  │  Label ("NORMAL" / ":wq" / "recording @q") │  │
//! │  └───────────────────────────────────────────┘  │
//! └─────────────────────────────────────────────────┘
//! ```

use std::fmt::Write;

use godot::classes::control::{GrowDirection, MouseFilter, SizeFlags};
use godot::classes::{
    CodeEdit, Control, HBoxContainer, IHBoxContainer, Label, Node, PanelContainer, StyleBoxFlat,
    Tween,
};
use godot::prelude::*;

use vim_core::primitives::{CommandLinePrompt, Mode};

use crate::safety::panic_guard;

use crate::types::UiSnapshot;

// ─────────────────────────────────────────────────────────────────────────────
// Theme
// ─────────────────────────────────────────────────────────────────────────────

/// Minimum alpha for the recording-mode pulse animation.
const RECORDING_PULSE_MIN_ALPHA: f64 = 0.5;
/// Duration in seconds for each leg (dim → bright, bright → dim) of the pulse.
const RECORDING_PULSE_DURATION: f64 = 0.4;

/// Color palette for the status bar, built from a settings snapshot.
///
/// Rebuilt via [`VimStatusBar::apply_colors`] on attach and settings change.
#[derive(Debug)]
struct StatusBarTheme {
    mode_normal: Color,
    mode_insert: Color,
    mode_visual: Color,
    mode_replace: Color,
    mode_command: Color,
    mode_recording: Color,
    text_fg: Color,
    error_fg: Color,
}

impl StatusBarTheme {
    /// Build from a [`crate::settings::StatusBarColors`] snapshot.
    fn from_snapshot(colors: &crate::settings::StatusBarColors) -> Self {
        Self {
            mode_normal: colors.normal_bg,
            mode_insert: colors.insert_bg,
            mode_visual: colors.visual_bg,
            mode_replace: colors.replace_bg,
            mode_command: colors.command_bg,
            mode_recording: colors.recording_bg,
            text_fg: colors.text_fg,
            error_fg: colors.error_fg,
        }
    }

    /// Placeholder defaults for init -- immediately overwritten when the
    /// coordinator calls `apply_colors()` with the real settings snapshot.
    fn init_defaults() -> Self {
        use crate::settings::defaults;
        Self {
            mode_normal: defaults::status_bar_normal_bg(),
            mode_insert: defaults::status_bar_insert_bg(),
            mode_visual: defaults::status_bar_visual_bg(),
            mode_replace: defaults::status_bar_replace_bg(),
            mode_command: defaults::status_bar_command_bg(),
            mode_recording: defaults::status_bar_recording_bg(),
            text_fg: defaults::status_bar_text_fg(),
            error_fg: defaults::status_bar_error_fg(),
        }
    }

    fn bg_for_mode(&self, mode: Mode, is_recording: bool) -> Color {
        if is_recording {
            return self.mode_recording;
        }
        match mode {
            Mode::Insert => self.mode_insert,
            Mode::Visual(_) => self.mode_visual,
            Mode::Replace => self.mode_replace,
            Mode::CommandLine => self.mode_command,
            // OperatorPending reuses visual color -- visually similar "pending" semantics.
            Mode::OperatorPending(_) => self.mode_visual,
            // Mode is #[non_exhaustive]; future variants fall back to normal.
            _ => self.mode_normal,
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// GodotClass
// ─────────────────────────────────────────────────────────────────────────────

/// Floating status bar overlay for the Vim editor plugin.
///
/// Anchored to the bottom-right of a `CodeEdit`. Displays mode, command-line,
/// messages, and recording state via a colored panel with a text label.
#[derive(GodotClass)]
#[class(base = HBoxContainer)]
pub struct VimStatusBar {
    base: Base<HBoxContainer>,

    panel: Option<Gd<PanelContainer>>,
    label: Option<Gd<Label>>,
    /// Infinite-loop tween for the recording-mode pulse; must be killed on detach.
    recording_tween: Option<Gd<Tween>>,

    theme: StatusBarTheme,

    /// Reused across updates to avoid a Godot object allocation per keystroke.
    cached_style: Option<Gd<StyleBoxFlat>>,
    /// Reused across updates to avoid a heap allocation per keystroke.
    display_buffer: String,
}

// ─────────────────────────────────────────────────────────────────────────────
// IHBoxContainer lifecycle
// ─────────────────────────────────────────────────────────────────────────────

#[godot_api]
impl IHBoxContainer for VimStatusBar {
    fn init(base: Base<HBoxContainer>) -> Self {
        Self {
            base,
            panel: None,
            label: None,
            recording_tween: None,
            theme: StatusBarTheme::init_defaults(),
            cached_style: None,
            display_buffer: String::with_capacity(64),
        }
    }

    fn ready(&mut self) {
        panic_guard(
            "status_bar::ready",
            || {
                {
                    let mut base = self.base_mut();
                    base.set_name("VimStatusBar");
                    base.set_h_size_flags(SizeFlags::SHRINK_END);
                    base.set_v_size_flags(SizeFlags::SHRINK_CENTER);
                    base.add_theme_constant_override("separation", 8);
                }

                // MouseFilter::IGNORE on all children so clicks fall through to
                // the CodeEdit underneath (Godot's default STOP would absorb them).
                // The HBoxContainer itself is set to IGNORE by inject_status_bar().
                let mut panel = PanelContainer::new_alloc();
                panel.set_name("ModePanel");
                panel.set_mouse_filter(MouseFilter::IGNORE);

                let mut label = Label::new_alloc();
                label.set_name("ModeLabel");
                label.set_text("NORMAL");
                label.set_mouse_filter(MouseFilter::IGNORE);

                panel.add_child(&label);
                self.base_mut().add_child(&panel);

                self.panel = Some(panel);
                self.label = Some(label);
                self.apply_panel_style(self.theme.mode_normal, self.theme.text_fg);
            },
            (),
        );
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Public API
// ─────────────────────────────────────────────────────────────────────────────

#[godot_api]
impl VimStatusBar {
    /// Apply new status bar colors from a [`crate::settings::StatusBarColors`] snapshot.
    ///
    /// Called by [`super::coordinator::UiCoordinator::apply_settings`] on attach
    /// and whenever settings change.
    pub(crate) fn apply_colors(&mut self, colors: &crate::settings::StatusBarColors) {
        self.theme = StatusBarTheme::from_snapshot(colors);
    }

    /// Refresh the status bar text, background color, and recording animation
    /// from the latest engine snapshot.
    pub fn update(&mut self, snap: &UiSnapshot) {
        let is_recording = snap.recording_register.is_some();

        // Display priority cascade — first matching arm wins:
        // vimdebug > command-line > message > recording > showcmd > pending keys > mode name.
        self.display_buffer.clear();

        let text_color = if let Some(step) = snap.vimdebug.step_status() {
            write!(self.display_buffer, "{step}").ok();
            self.theme.text_fg
        } else if let Some(ref prompt) = snap.cmdline.prompt {
            let prompt_char = prompt_char_for(prompt);
            format_cmdline_into(
                &mut self.display_buffer,
                prompt_char,
                &snap.cmdline.input,
                snap.cmdline.cursor,
            );
            self.theme.text_fg
        } else if let Some(message) = snap.message.text() {
            self.display_buffer.push_str(message);
            if snap.message.is_error() {
                self.theme.error_fg
            } else {
                self.theme.text_fg
            }
        } else if let Some(reg) = snap.recording_register {
            let mode_name = snap.mode.short_name();
            write!(self.display_buffer, "recording @{reg} {mode_name}").ok();
            self.theme.text_fg
        } else if !snap.pending_command.is_empty() {
            let mode_name = snap.mode.short_name();
            write!(self.display_buffer, "{mode_name}  {}", snap.pending_command).ok();
            self.theme.text_fg
        } else if !snap.pending_keys.is_empty() {
            let mode_name = snap.mode.short_name();
            write!(self.display_buffer, "{mode_name}  {}", snap.pending_keys).ok();
            self.theme.text_fg
        } else {
            self.display_buffer.push_str(snap.mode.short_name());
            self.theme.text_fg
        };

        // Vimdebug annotations are appended regardless of which priority arm was taken.
        if let Some(prov) = snap.vimdebug.provenance() {
            write!(self.display_buffer, "  [{prov}]").ok();
        }
        if let Some(fx) = snap.vimdebug.effects() {
            write!(self.display_buffer, "  fx: {fx}").ok();
        }

        let bg_color = self.theme.bg_for_mode(snap.mode, is_recording);

        if let Some(ref mut label) = self.label {
            label.set_text(&self.display_buffer);
        }
        self.apply_panel_style(bg_color, text_color);

        if is_recording {
            if self.recording_tween.is_none() {
                self.start_recording_animation();
            }
        } else {
            self.stop_recording_animation();
        }
    }

    // ─────────────────────────────────────────────────────────────────────────
    // Styling (private)
    // ─────────────────────────────────────────────────────────────────────────

    /// Reuses a cached `StyleBoxFlat` to avoid a Godot object allocation on
    /// every keystroke. The style is created once on first call and mutated
    /// in-place thereafter (Godot reflects the change automatically since the
    /// panel holds a reference to the same resource).
    fn apply_panel_style(&mut self, bg: Color, fg: Color) {
        if let Some(ref mut panel) = self.panel {
            if let Some(ref mut style) = self.cached_style {
                style.set_bg_color(bg);
            } else {
                let mut style = StyleBoxFlat::new_gd();
                style.set_bg_color(bg);
                style.set_corner_radius_all(4);
                style.set_content_margin(Side::LEFT, 8.0);
                style.set_content_margin(Side::RIGHT, 8.0);
                style.set_content_margin(Side::TOP, 0.0);
                style.set_content_margin(Side::BOTTOM, 0.0);

                let style_box = style.clone().upcast::<godot::classes::StyleBox>();
                panel.add_theme_stylebox_override("panel", &style_box);
                self.cached_style = Some(style);
            }
        }

        if let Some(ref mut label) = self.label {
            label.add_theme_color_override("font_color", fg);
        }
    }

    // ─────────────────────────────────────────────────────────────────────────
    // Recording animation (private)
    // ─────────────────────────────────────────────────────────────────────────

    /// Looping tween that oscillates `modulate:a` between 0.5 and 1.0 to
    /// give visual "recording" feedback. Each leg is 0.4s.
    fn start_recording_animation(&mut self) {
        self.stop_recording_animation();

        let panel = match self.panel.as_ref() {
            Some(p) => p.clone().upcast::<godot::classes::Object>(),
            None => return,
        };

        // Scoped block so the `base_mut()` borrow is released before we
        // assign `self.recording_tween`.
        let tween_opt = {
            let tween_created = self.base_mut().create_tween();
            if let Some(mut tween) = tween_created {
                tween.set_loops_ex().loops(0).done(); // 0 = infinite loops

                let property = "modulate:a";
                drop(tween.tween_property(
                    &panel,
                    property,
                    &Variant::from(RECORDING_PULSE_MIN_ALPHA),
                    RECORDING_PULSE_DURATION,
                ));
                drop(tween.tween_property(
                    &panel,
                    property,
                    &Variant::from(1.0_f64),
                    RECORDING_PULSE_DURATION,
                ));

                Some(tween)
            } else {
                None
            }
        };

        self.recording_tween = tween_opt;
    }

    /// Public because the coordinator must kill the tween before freeing the
    /// node on detach — an infinite-loop tween attached to a freed node
    /// would leak in the SceneTree.
    pub fn stop_recording_animation(&mut self) {
        if let Some(mut tween) = self.recording_tween.take() {
            tween.kill();
        }

        if let Some(ref mut panel) = self.panel {
            panel.set_modulate(Color::WHITE);
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Command-line formatting helpers
// ─────────────────────────────────────────────────────────────────────────────

/// Map a [`CommandLinePrompt`] to its display character.
fn prompt_char_for(prompt: &CommandLinePrompt) -> &'static str {
    match prompt {
        CommandLinePrompt::Ex => ":",
        CommandLinePrompt::SearchForward => "/",
        CommandLinePrompt::SearchBackward => "?",
        // CommandLinePrompt is #[non_exhaustive].
        _ => ":",
    }
}

/// Allocating convenience wrapper for tests. Production code calls
/// `format_cmdline_into` directly to reuse a pre-allocated buffer.
#[cfg(test)]
fn format_cmdline(prompt: &str, input: &str, cursor: usize) -> String {
    let mut out = String::with_capacity(prompt.len() + input.len() + 1);
    format_cmdline_into(&mut out, prompt, input, cursor);
    out
}

/// Render `{prompt}{input}` into `buf` with a `|` pipe at the cursor position.
///
/// `cursor` is a char-offset (not byte-offset) into `input`, clamped to input length.
fn format_cmdline_into(buf: &mut String, prompt: &str, input: &str, cursor: usize) {
    let char_count = input.chars().count();
    let clamped = cursor.min(char_count);

    buf.push_str(prompt);
    for (i, ch) in input.chars().enumerate() {
        if i == clamped {
            buf.push('|');
        }
        buf.push(ch);
    }
    if clamped >= char_count {
        buf.push('|');
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Injection free function
// ─────────────────────────────────────────────────────────────────────────────

/// Inject a `VimStatusBar` into a `CodeEdit` as a floating overlay.
///
/// Handles reparenting, anchor pinning to bottom-right, and mouse passthrough.
/// Orphan cleanup is handled by `remove_orphaned_overlays` before this is called.
pub(crate) fn inject_status_bar(editor: &Gd<CodeEdit>, bar: &Gd<VimStatusBar>) {
    let bar_node = bar.clone().upcast::<Node>();
    let editor_node = editor.clone().upcast::<Node>();

    // Reparent if the bar is already attached to a different editor.
    if let Some(mut old_parent) = bar_node.get_parent() {
        if old_parent.instance_id() != editor_node.instance_id() {
            old_parent.remove_child(&bar_node);
            editor.clone().add_child(&bar_node);
        }
    } else {
        editor.clone().add_child(&bar_node);
    }

    // Pin all four anchors to (1.0, 1.0) so the bar sticks to bottom-right.
    // RIGHT/BOTTOM offsets pull it inward for visual padding.
    let mut control = bar.clone().upcast::<Control>();

    for side in [Side::LEFT, Side::TOP, Side::RIGHT, Side::BOTTOM] {
        control.set_anchor(side, 1.0);
        let offset = if side == Side::RIGHT || side == Side::BOTTOM {
            -10.0
        } else {
            0.0
        };
        control.set_offset(side, offset);
    }

    // Grow toward top-left so the bar expands away from the corner.
    control.set_h_grow_direction(GrowDirection::BEGIN);
    control.set_v_grow_direction(GrowDirection::BEGIN);

    // IGNORE propagates to children (also set to IGNORE in ready()).
    control.set_mouse_filter(MouseFilter::IGNORE);
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── format_cmdline: basic behavior ──────────────────────────────────

    #[test]
    fn format_cmdline_cursor_at_start() {
        assert_eq!(format_cmdline(":", "hello", 0), ":|hello");
    }

    #[test]
    fn format_cmdline_cursor_at_end() {
        assert_eq!(format_cmdline(":", "hello", 5), ":hello|");
    }

    #[test]
    fn format_cmdline_cursor_in_middle() {
        assert_eq!(format_cmdline(":", "hello", 2), ":he|llo");
    }

    #[test]
    fn format_cmdline_cursor_at_position_one() {
        assert_eq!(format_cmdline("/", "abc", 1), "/a|bc");
    }

    // ── format_cmdline: empty input ─────────────────────────────────────

    #[test]
    fn format_cmdline_empty_input_cursor_zero() {
        assert_eq!(format_cmdline(":", "", 0), ":|");
    }

    #[test]
    fn format_cmdline_empty_input_cursor_beyond() {
        // Cursor past end of empty input is clamped to 0.
        assert_eq!(format_cmdline("?", "", 5), "?|");
    }

    // ── format_cmdline: different prompt chars ──────────────────────────

    #[test]
    fn format_cmdline_forward_search_prompt() {
        assert_eq!(format_cmdline("/", "pattern", 0), "/|pattern");
    }

    #[test]
    fn format_cmdline_backward_search_prompt() {
        assert_eq!(format_cmdline("?", "pattern", 7), "?pattern|");
    }

    // ── format_cmdline: cursor clamping ─────────────────────────────────

    #[test]
    fn format_cmdline_cursor_beyond_input_length() {
        assert_eq!(format_cmdline(":", "abc", 100), ":abc|");
    }

    #[test]
    fn format_cmdline_cursor_exactly_at_length() {
        assert_eq!(format_cmdline(":", "ab", 2), ":ab|");
    }

    // ── format_cmdline: Unicode input ───────────────────────────────────

    // Cursor offset is in char units, not bytes — these verify multi-byte
    // characters are counted correctly.

    #[test]
    fn format_cmdline_unicode_multibyte() {
        assert_eq!(format_cmdline(":", "hëllo", 2), ":hë|llo");
    }

    #[test]
    fn format_cmdline_unicode_cjk() {
        assert_eq!(format_cmdline("/", "日本語", 1), "/日|本語");
    }

    #[test]
    fn format_cmdline_unicode_emoji() {
        assert_eq!(format_cmdline(":", "a😀b", 2), ":a😀|b");
    }

    #[test]
    fn format_cmdline_unicode_cursor_at_start() {
        assert_eq!(format_cmdline(":", "日本語", 0), ":|日本語");
    }

    #[test]
    fn format_cmdline_unicode_cursor_at_end() {
        assert_eq!(format_cmdline(":", "日本語", 3), ":日本語|");
    }

    // ── format_cmdline: single character input ──────────────────────────

    #[test]
    fn format_cmdline_single_char_cursor_before() {
        assert_eq!(format_cmdline(":", "x", 0), ":|x");
    }

    #[test]
    fn format_cmdline_single_char_cursor_after() {
        assert_eq!(format_cmdline(":", "x", 1), ":x|");
    }

    // ── format_cmdline: spaces in input ─────────────────────────────────

    #[test]
    fn format_cmdline_spaces_in_input() {
        assert_eq!(format_cmdline(":", "a b c", 2), ":a |b c");
    }

    // ── prompt_char_for ─────────────────────────────────────────────────

    #[test]
    fn prompt_char_for_ex() {
        assert_eq!(prompt_char_for(&CommandLinePrompt::Ex), ":");
    }

    #[test]
    fn prompt_char_for_search_forward() {
        assert_eq!(prompt_char_for(&CommandLinePrompt::SearchForward), "/");
    }

    #[test]
    fn prompt_char_for_search_backward() {
        assert_eq!(prompt_char_for(&CommandLinePrompt::SearchBackward), "?");
    }
}
