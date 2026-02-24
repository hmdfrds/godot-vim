use crate::bridge::godot::names::canvas_item;
use crate::bridge::settings::VimSettings;
use crate::bridge::types::mode::EditorMode;
use crate::bridge::vim_adapter::managers::mode;
use godot::classes::{CanvasItem, Control, IControl, Panel, Shader, ShaderMaterial, StyleBoxFlat};
use godot::prelude::*;

#[derive(GodotClass)]
#[class(base=Control)]
pub struct VimCursor {
    base: Base<Control>,

    // Child node for the actual shape
    visual: Option<Gd<Panel>>,

    // Animation state
    target_pos: Vector2,
    current_pos: Vector2,

    // Config
    lerp_speed: f64,
    blink_speed: f64,
    blink_time: f64,

    // Mode state for styling
    current_mode: EditorMode,
    font_height: f32,
    char_width: f32,

    // Style state
    base_alpha: f32,
}

#[godot_api]
impl VimCursor {
    /// Primary API: Set the target position (pixel coordinates).
    /// Wakes up the processing loop.
    #[func]
    pub fn set_target(&mut self, target: Vector2, font_height: f32, char_width: f32) {
        self.target_pos = target;
        self.font_height = font_height;
        self.char_width = char_width;

        // Snap to target immediately on first placement to avoid interpolating from the origin.
        if self.current_pos.length_squared() < 0.1 {
            self.current_pos = target;
            self.base_mut().set_position(target);
        }

        // Reset blink phase on interaction
        self.blink_time = 0.0;
        self.update_alpha(1.0);

        // Enable per-frame processing.
        self.base_mut().set_process(true);

        // Refresh the visual shape to reflect the updated metrics.
        self.update_visual_shape();
    }

    /// Primary API: Update visual style based on Vim Mode.
    /// Not an FFI #[func] because Mode is not Godot-compatible by default.
    pub fn set_mode(&mut self, mode: EditorMode) {
        self.current_mode = mode;
        // Reset blink on mode change
        self.blink_time = 0.0;
        self.update_alpha(1.0);

        self.update_visual_style();
        self.update_visual_shape();
    }

    #[func]
    pub fn force_snap(&mut self) {
        let target = self.target_pos;
        self.current_pos = target;
        self.base_mut().set_position(target);
    }

    // Internal helper to set alpha using self_modulate
    fn update_alpha(&mut self, alpha: f32) {
        if let Some(mut visual) = self.visual.clone() {
            // Use self_modulate for transparency (affects panel + borders)
            visual.set_self_modulate(Color::from_rgba(1.0, 1.0, 1.0, alpha));
        }
    }
}

#[godot_api]
impl IControl for VimCursor {
    fn init(base: Base<Control>) -> Self {
        Self {
            base,
            visual: None,
            target_pos: Vector2::ZERO,
            current_pos: Vector2::ZERO,
            lerp_speed: 25.0,
            blink_speed: 4.0,
            blink_time: 0.0,
            current_mode: EditorMode::Normal,
            font_height: 20.0,
            char_width: 10.0,
            base_alpha: 0.5,
        }
    }

    fn ready(&mut self) {
        // Create the visual child as a Panel (required for StyleBox support).
        let mut visual = Panel::new_alloc();
        visual.set_name("CursorShape");
        visual.set_mouse_filter(godot::classes::control::MouseFilter::IGNORE);

        // Apply difference-blend shader so the character underneath remains visible.
        let mut shader = Shader::new_gd();
        shader.set_code(
            r#"
            shader_type canvas_item;
            uniform sampler2D screen_texture : hint_screen_texture, repeat_disable, filter_nearest;
            
            void fragment() {
                vec4 bg = texture(screen_texture, SCREEN_UV);
                float a = COLOR.a;
                
                // Difference Blending: |bg - cursor_color|
                // Alpha controls the blend strength, enabling blinking.
                vec3 diff = abs(bg.rgb - COLOR.rgb);
                
                // Manual mix to output. 
                // The alpha channel controls the strength of the color inversion.
                COLOR.rgb = mix(bg.rgb, diff, a);
                
                // Output opaque alpha so blending logic is baked into RGB.
                // This prevents Godot from double-blending.
                COLOR.a = 1.0; 
            }
        "#,
        );

        let mut material = ShaderMaterial::new_gd();
        material.set_shader(&shader);

        let mat_base = material.upcast::<godot::classes::Material>();
        visual.set_material(&mat_base);

        let visual_node: Gd<godot::classes::Node> = visual.clone().upcast();
        self.base_mut().add_child(&visual_node);

        // High z-index ensures the cursor renders on top of selection highlights.
        visual.clone().upcast::<CanvasItem>().set_z_index(100);
        self.base().clone().upcast::<CanvasItem>().set_z_index(100);

        self.visual = Some(visual);

        self.update_visual_style();
        self.base_mut().set_process(true);
    }

    fn process(&mut self, delta: f64) {
        let current = self.current_pos;
        let target = self.target_pos;
        let dist = target - current;
        let is_moving = dist.length_squared() > 0.25;

        if is_moving {
            let new_pos = current.lerp(target, (self.lerp_speed * delta) as f32);
            self.current_pos = new_pos;
            self.base_mut().set_position(new_pos);

            // While moving, stay solid.
            self.blink_time = 0.0;
            self.update_alpha(1.0);
        } else {
            // Snap if very close
            if dist.length_squared() > 0.0 {
                self.current_pos = target;
                self.base_mut().set_position(target);
            }

            self.blink_time += delta * self.blink_speed;

            // Square wave blink: 50% duty cycle, no easing.
            let blink_factor = if self.blink_time.sin() >= 0.0 {
                1.0
            } else {
                0.0
            };

            let max_alpha = self.base_alpha;

            let current_alpha = max_alpha * blink_factor;
            self.update_alpha(current_alpha);
        }
    }
}

impl VimCursor {
    fn update_visual_style(&mut self) {
        let Some(mut visual) = self.visual.clone() else {
            return;
        };

        // Use centralized mode color logic from mode_manager
        let color = if let Some(c) = mode::get_mode_cursor_color(&self.current_mode) {
            // Special case: Replace mode has a distinct red tint
            if matches!(self.current_mode, EditorMode::Replace) {
                Color::from_rgba(1.0, 0.2, 0.2, 0.6)
            } else {
                c
            }
        } else {
            // Mode colors disabled - use normal color
            VimSettings::normal_mode_color()
        };

        if VimSettings::premium_cursor_enabled() {
            // Custom cursor: solid fill with difference-blend shader and blinking.
            let mut style = StyleBoxFlat::new_gd();
            style.set_bg_color(color);
            style.set_draw_center(true);
            style.set_border_width_all(0);

            let style_box: Gd<godot::classes::StyleBox> = style.upcast();
            visual.add_theme_stylebox_override("panel", &style_box);

            // Difference blend shader is attached in ready().
            self.base_mut().set_process(true);

            // Restore cached alpha
            self.base_alpha = color.a;
        } else {
            // Fallback cursor: hollow block outline, no shader, no blinking.
            let mut style = StyleBoxFlat::new_gd();
            style.set_bg_color(Color::from_rgba(0.0, 0.0, 0.0, 0.0));
            style.set_draw_center(false);

            style.set_border_width_all(2);
            style.set_border_color(color);

            let style_box: Gd<godot::classes::StyleBox> = style.upcast();
            visual.add_theme_stylebox_override("panel", &style_box);

            // Clear shader via dynamic call; the typed API does not accept a null material.
            visual.call(canvas_item::methods::SET_MATERIAL, &[Variant::nil()]);

            self.base_mut().set_process(false);
            self.force_snap();
        }
    }

    fn update_visual_shape(&mut self) {
        let Some(mut visual) = self.visual.clone() else {
            return;
        };

        // Block vs Beam vs Underline logic
        match self.current_mode {
            EditorMode::Insert => {
                // Beam
                let width = 2.0;
                visual.set_size(Vector2::new(width, self.font_height));
                visual.set_position(Vector2::new(0.0, 0.0));
            }
            EditorMode::Replace => {
                // Underline
                let height = 4.0;
                visual.set_size(Vector2::new(self.char_width, height));
                visual.set_position(Vector2::new(0.0, self.font_height - height));
                // Bottom align
            }
            _ => {
                // Block (Normal, Visual)
                visual.set_size(Vector2::new(self.char_width, self.font_height));
                visual.set_position(Vector2::new(0.0, 0.0));
            }
        }
    }
}
