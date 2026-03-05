use crate::bridge::godot::names::{callbacks, control};
use crate::bridge::settings::VimSettings;
use crate::bridge::types::mode::{CmdLineKind, EditorMode};
use crate::bridge::vim_adapter::core::cast::usize_to_i32;
use godot::classes::{
    control::SizeFlags, HBoxContainer, IHBoxContainer, InputEvent, InputEventMouseButton, Label,
    LineEdit, PanelContainer, StyleBoxFlat, Tween,
};
use godot::global::MouseButton;
use godot::prelude::*;

/// Vim status bar component displaying mode, cursor position, and command input.
///
/// Layout: `[Separator] [Cursor Label] [Mode Panel]`
///
/// The Mode Panel contains either:
/// - A `Label` showing the current mode (NORMAL/INSERT/VISUAL)
/// - A `LineEdit` for command input (when in `CmdLine` mode)
#[derive(GodotClass)]
#[class(base=HBoxContainer)]
pub struct VimCmdLine {
    #[base]
    base: Base<HBoxContainer>,

    // UI Components
    main_panel: Option<Gd<PanelContainer>>,
    main_label: Option<Gd<Label>>,
    command_input: Option<Gd<LineEdit>>,

    // Animation
    /// Active tween for recording animation (pulsing effect)
    recording_tween: Option<Gd<Tween>>,

    // Theme
    theme: StatusBarTheme,
}

/// Theme colors for status bar. Extracted to enable future customization.
struct StatusBarTheme {
    mode_normal: Color,
    mode_insert: Color,
    mode_visual: Color,
    mode_cmd: Color,
    mode_recording: Color,
    text_fg: Color,
}

impl Default for StatusBarTheme {
    fn default() -> Self {
        Self {
            mode_normal: Color::from_rgb(0.5, 0.6, 0.8),
            mode_insert: Color::from_rgb(0.6, 0.8, 0.5),
            mode_visual: Color::from_rgb(0.8, 0.5, 0.5),
            mode_cmd: Self::parse_color("#282c34", Color::BLACK),
            mode_recording: Color::from_rgb(0.9, 0.2, 0.2),
            text_fg: Color::WHITE,
        }
    }
}

impl StatusBarTheme {
    fn parse_color(hex: &str, fallback: Color) -> Color {
        Color::from_string(hex).unwrap_or_else(|| {
            log::warn!("Failed to parse color hex={}, using fallback", hex);
            fallback
        })
    }
}

#[godot_api]
impl IHBoxContainer for VimCmdLine {
    fn init(base: Base<HBoxContainer>) -> Self {
        Self {
            base,
            main_panel: None,
            main_label: None,
            command_input: None,

            recording_tween: None,
            theme: StatusBarTheme::default(),
        }
    }

    fn ready(&mut self) {
        self.setup_container();
        let (mut main_panel, main_label, command_input) = Self::create_mode_panel();

        self.add_children_to_base(&main_panel);

        // Connect input signal for click-to-toggle
        main_panel.connect(
            control::signals::GUI_INPUT,
            &self.base().callable(callbacks::ON_PANEL_GUI_INPUT),
        );

        // Store references
        self.main_panel = Some(main_panel.clone());
        self.main_label = Some(main_label);
        self.command_input = Some(command_input);

        // Apply initial style (not recording at startup)
        self.apply_mode_style(EditorMode::Normal, false);
    }
}

#[godot_api]
impl VimCmdLine {
    // ─────────────────────────────────────────────────────────────────────────
    // Setup Helpers (called from ready)
    // ─────────────────────────────────────────────────────────────────────────

    fn setup_container(&mut self) {
        let mut base = self.base_mut();
        base.set_name("VimStatusBar");
        base.set_h_size_flags(SizeFlags::SHRINK_END);
        base.set_v_size_flags(SizeFlags::SHRINK_CENTER);
        base.add_theme_constant_override("separation", 8);
    }

    fn create_mode_panel() -> (Gd<PanelContainer>, Gd<Label>, Gd<LineEdit>) {
        let mut panel = PanelContainer::new_alloc();
        panel.set_name("MainPanel");
        panel.set_mouse_filter(godot::classes::control::MouseFilter::STOP);

        let mut label = Label::new_alloc();
        label.set_name("ModeLabel");
        label.set_text("NORMAL");

        let mut input = LineEdit::new_alloc();
        input.set_name("CommandInput");
        input.set_visible(false);
        input.set_flat(true);
        input.set_horizontal_alignment(godot::global::HorizontalAlignment::LEFT);
        input.set_custom_minimum_size(Vector2::new(200.0, 0.0));
        input.set_caret_blink_enabled(true);
        input.set_context_menu_enabled(false);

        panel.add_child(&label);
        panel.add_child(&input);

        (panel, label, input)
    }

    fn add_children_to_base(&mut self, panel: &Gd<PanelContainer>) {
        let mut base = self.base_mut();
        base.add_child(panel);
    }

    // ─────────────────────────────────────────────────────────────────────────
    // Styling
    // ─────────────────────────────────────────────────────────────────────────

    fn apply_panel_style(panel: &mut Gd<PanelContainer>, bg: Color, fg: Color) {
        let mut style = StyleBoxFlat::new_gd();
        style.set_bg_color(bg);
        style.set_corner_radius_all(4);
        // Horizontal margins only
        style.set_content_margin(godot::builtin::Side::LEFT, 8.0);
        style.set_content_margin(godot::builtin::Side::RIGHT, 8.0);
        style.set_content_margin(godot::builtin::Side::TOP, 0.0);
        style.set_content_margin(godot::builtin::Side::BOTTOM, 0.0);

        let style_box = style.upcast::<godot::classes::StyleBox>();
        panel.add_theme_stylebox_override("panel", &style_box);
        panel.set_self_modulate(Color::WHITE);

        // Apply text color to children
        for child in panel.get_children().iter_shared() {
            if let Ok(mut label) = child.clone().try_cast::<Label>() {
                label.add_theme_color_override("font_color", fg);
            }
            if let Ok(mut input) = child.try_cast::<LineEdit>() {
                input.add_theme_color_override("font_color", fg);
            }
        }
    }

    fn apply_mode_style(&mut self, mode: EditorMode, is_recording: bool) {
        // Stop any existing recording animation when leaving Recording mode
        if !is_recording {
            self.stop_recording_animation();
        }

        // Recording takes priority for styling (red background with animation)
        let bg_color = if is_recording {
            self.theme.mode_recording
        } else {
            match mode {
                EditorMode::Insert => self.theme.mode_insert,
                EditorMode::Visual | EditorMode::VisualLine | EditorMode::VisualBlock => {
                    self.theme.mode_visual
                }
                EditorMode::OperatorPending => self.theme.mode_visual,
                EditorMode::CmdLine(_) => self.theme.mode_cmd,
                EditorMode::Recording { .. } => self.theme.mode_recording,
                _ => self.theme.mode_normal,
            }
        };

        // Apply style to panel
        if let Some(ref mut panel) = self.main_panel {
            Self::apply_panel_style(panel, bg_color, self.theme.text_fg);
        }

        // Start animation after style is applied (for Recording mode)
        if is_recording {
            self.start_recording_animation();
        }
    }

    /// Starts the pulsing animation for recording mode.
    ///
    /// Creates a looping tween that animates the panel's modulate alpha
    /// between bright and dim to create a "recording" visual feedback.
    fn start_recording_animation(&mut self) {
        // Kill any existing tween first
        self.stop_recording_animation();

        // Get panel as Object for tween_property
        let panel = match self.main_panel.as_ref() {
            Some(p) => p.clone().upcast::<godot::classes::Object>(),
            None => return,
        };

        // Create tween and configure it
        // Store the result in a block so the base_mut() borrow is released before further use.
        let tween_opt = {
            let tween_created = self.base_mut().create_tween();
            if let Some(mut tween) = tween_created {
                // Configure for infinite looping
                tween.set_loops_ex().loops(0).done(); // 0 = infinite

                // Animate modulate alpha for pulsing (1.0 → 0.5 → 1.0)
                let property = "modulate:a";

                // Tween owns the PropertyTweener internally; return value not needed.
                // Fade to dim (0.5 alpha), then fade back to bright (1.0 alpha).
                drop(tween.tween_property(&panel, property, &Variant::from(0.5_f64), 0.4));
                drop(tween.tween_property(&panel, property, &Variant::from(1.0_f64), 0.4));

                Some(tween)
            } else {
                None
            }
        };

        // Assign after borrow ends
        self.recording_tween = tween_opt;
    }

    /// Stops the recording animation and resets panel modulate.
    fn stop_recording_animation(&mut self) {
        // Kill the tween if it exists
        if let Some(mut tween) = self.recording_tween.take() {
            tween.kill();
        }

        // Reset panel modulate to fully opaque
        if let Some(ref mut panel) = self.main_panel {
            panel.set_modulate(Color::WHITE);
        }
    }

    // ─────────────────────────────────────────────────────────────────────────
    // Public API
    // ─────────────────────────────────────────────────────────────────────────

    /// Handler for click events on the main panel needed to toggle Vim status.
    #[func]
    fn on_panel_gui_input(&mut self, event: Gd<InputEvent>) {
        if let Ok(mouse_event) = event.try_cast::<InputEventMouseButton>() {
            if mouse_event.is_pressed() && mouse_event.get_button_index() == MouseButton::LEFT {
                let current = VimSettings::enabled();
                VimSettings::set_enabled(!current);

                // Force a visual update immediately when disabled, for responsiveness.
                self.update_mode(EditorMode::Normal, None);
            }
        }
    }

    /// Updates the mode display and styling.
    ///
    /// # Arguments
    /// * `mode` - Current Vim mode
    /// * `recording_register` - If Some, macro recording is active for that register
    pub fn update_mode(&mut self, mode: EditorMode, recording_register: Option<char>) {
        // Override if disabled
        if !VimSettings::enabled() {
            if let Some(mut label) = self.main_label.clone() {
                label.set_visible(true);
                label.set_text("OFF");
            }
            if let Some(mut input) = self.command_input.clone() {
                input.set_visible(false);
            }
            // Use gray color for disabled state
            if let Some(ref mut panel) = self.main_panel {
                Self::apply_panel_style(
                    panel,
                    Color::from_rgb(0.3, 0.3, 0.3),
                    Color::from_rgb(0.6, 0.6, 0.6),
                );
            }
            return;
        }

        let is_cmd = matches!(mode, EditorMode::CmdLine(_));

        // Always show in CmdLine mode; otherwise honor the cmdline_enabled setting.
        if is_cmd {
            self.base_mut().set_visible(true);
        } else {
            self.base_mut().set_visible(VimSettings::cmdline_enabled());
        }

        if is_cmd {
            // CmdLine mode: Hide label, show LineEdit with prompt
            if let Some(ref mut label) = self.main_label {
                label.set_visible(false);
            }
            if let Some(ref mut input) = self.command_input {
                input.set_visible(true);
                // Set prompt based on command type
                let prompt = match mode {
                    EditorMode::CmdLine(CmdLineKind::SearchForward) => "/",
                    EditorMode::CmdLine(CmdLineKind::SearchBackward) => "?",
                    _ => ":",
                };
                log::debug!("Cmdline update_mode mode={:?} prompt={}", mode, prompt);
                // Block signals during set_text to avoid triggering text_changed
                // while VimController is still borrowed during mode change
                input.set_block_signals(true);
                input.set_text(prompt);
                input.set_block_signals(false);
                // Use char count, not byte length, for correct caret positioning with Unicode
                input.set_caret_column(usize_to_i32(prompt.chars().count()));
                input.grab_focus();
            }
        } else {
            // Non-CmdLine mode: Show label, hide LineEdit
            if let Some(ref mut input) = self.command_input {
                input.set_visible(false);
                // Block signals during clear to avoid triggering text_changed
                input.set_block_signals(true);
                input.clear();
                input.set_block_signals(false);
            }
            if let Some(ref mut label) = self.main_label {
                label.set_visible(true);
                // Use EditorMode's display_name for text
                let mode_text = match mode {
                    EditorMode::Recording { register } => {
                        // Recording indicator takes priority
                        label.set_text(&format!("● recording @{register}"));
                        self.apply_mode_style(mode, recording_register.is_some());
                        return;
                    }
                    _ => mode.display_name(),
                };

                // If recording, show recording indicator with mode
                if let Some(reg) = recording_register {
                    label.set_text(&format!("● @{reg} {mode_text}"));
                } else {
                    label.set_text(mode_text);
                }
            }
        }

        // Always apply style on mode change
        self.apply_mode_style(mode, recording_register.is_some());
    }

    /// Sets the command input text and positions cursor.
    /// Called when entering `CmdLine` mode to initialize the input.
    pub fn start_input(&mut self, prompt: &str) {
        if let Some(ref mut input) = self.command_input {
            input.set_visible(true);
            // Block signals during set_text to avoid triggering text_changed
            input.set_block_signals(true);
            input.set_text(prompt);
            input.set_block_signals(false);
            // Use char count for correct caret positioning with Unicode
            input.set_caret_column(i32::try_from(prompt.chars().count()).unwrap_or(1));
            input.grab_focus();
        }
        if let Some(ref mut label) = self.main_label {
            label.set_visible(false);
        }
    }

    /// Returns a clone of the command input widget.
    pub fn get_command_input(&self) -> Option<Gd<LineEdit>> {
        self.command_input.clone()
    }

    /// Displays a message in the mode label (for informational messages).
    ///
    /// The message will be shown in the mode panel until the next mode change.
    pub fn show_message(&mut self, message: &str) {
        if let Some(mut label) = self.main_label.clone() {
            label.set_text(message);
            label.set_visible(true);
        }
        if let Some(mut input) = self.command_input.clone() {
            input.set_visible(false);
        }
    }
}
