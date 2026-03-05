use crate::bridge::godot::names::{callbacks, code_edit, control, range, text_edit, theme};
use crate::bridge::settings::{types::LineNumberMode, VimSettings};
use godot::classes::image::Format;
use godot::classes::text_edit::GutterType;
use godot::classes::{CodeEdit, INode, Image, ImageTexture, Node, Texture2D};
use godot::global::HorizontalAlignment;
use godot::prelude::*;

#[derive(GodotClass)]
#[class(base=Node)]
pub struct LineNumberManager {
    base: Base<Node>,

    editor: Option<Gd<CodeEdit>>,
    line_gutter_index: i32,
    fold_gutter_index: i32,

    // Resources
    empty_icon: Option<Gd<Texture2D>>,

    // Config Cache
    config_mode: LineNumberMode,
    last_line_count: i32,
    // State Cache for Optimization
    last_caret_line: i32,
    last_scroll_line: i32,
    force_next_update: bool,
}

#[godot_api]
impl INode for LineNumberManager {
    fn init(base: Base<Node>) -> Self {
        Self {
            base,
            editor: None,
            line_gutter_index: -1,
            fold_gutter_index: -1,
            empty_icon: None,
            config_mode: LineNumberMode::Absolute,
            last_line_count: -1,
            last_caret_line: -1,
            last_scroll_line: -1,
            force_next_update: true,
        }
    }
}

#[godot_api]
impl LineNumberManager {
    /// Attach to a CodeEdit instance and set up gutters.
    pub fn attach(&mut self, mut editor: Gd<CodeEdit>) {
        // Skip re-attaching to the same editor.
        if let Some(current) = &self.editor {
            if current.is_instance_valid() && current.instance_id() == editor.instance_id() {
                return;
            }
            // Switching editors: disconnect signals from the previous editor.
            if current.is_instance_valid() {
                self.disconnect_signals(current.clone());
            }
        }

        self.editor = Some(editor.clone());

        // Connect signals for updates
        // Use deferred update for CARET_CHANGED to ensure cursor position is settled
        let callable_update_deferred = self.base_mut().callable(callbacks::UPDATE_GUTTERS_DEFERRED);
        if !editor.is_connected(text_edit::signals::CARET_CHANGED, &callable_update_deferred) {
            editor.connect(text_edit::signals::CARET_CHANGED, &callable_update_deferred);
        }

        let callable_update = self.base_mut().callable(callbacks::UPDATE_GUTTERS);
        if !editor.is_connected(text_edit::signals::TEXT_CHANGED, &callable_update) {
            editor.connect(text_edit::signals::TEXT_CHANGED, &callable_update);
        }

        // Connect Scroll
        if let Some(mut scroll) = editor.get_v_scroll_bar() {
            let callable_scroll = self.base_mut().callable(callbacks::ON_SCROLL_CHANGED);
            if !scroll.is_connected(range::signals::VALUE_CHANGED, &callable_scroll) {
                scroll.connect(range::signals::VALUE_CHANGED, &callable_scroll);
            }
        }

        // Connect gutter click
        let callable_click = self.base_mut().callable(callbacks::ON_GUTTER_CLICKED);
        if !editor.is_connected(text_edit::signals::GUTTER_CLICKED, &callable_click) {
            editor.connect(text_edit::signals::GUTTER_CLICKED, &callable_click);
        }

        let callable_theme = self.base_mut().callable(callbacks::ON_THEME_CHANGED);
        if !editor.is_connected(control::signals::THEME_CHANGED, &callable_theme) {
            editor.connect(control::signals::THEME_CHANGED, &callable_theme);
        }

        // Connect to VISIBILITY_CHANGED for floating window support.
        // Connecting to DRAW is intentionally avoided; it fires too frequently (mouse hover, etc.).
        if !editor.is_connected(
            control::signals::VISIBILITY_CHANGED,
            &callable_update_deferred,
        ) {
            editor.connect(
                control::signals::VISIBILITY_CHANGED,
                &callable_update_deferred,
            );
        }

        // Setup Gutters
        self.setup_gutters();

        // Update initial state
        self.force_next_update = true;
        self.sync_settings();
    }

    fn setup_gutters(&mut self) {
        let Some(mut editor) = self.editor.clone() else {
            return;
        };

        // Disable the built-in gutters so the custom ones take their place.
        editor.set_draw_line_numbers(false);
        editor.set_draw_fold_gutter(false);

        // CONNECTION_GUTTER is intentionally left in place — it shows signal connection
        // indicators in GDScript, and removing it crashes ScriptTextEditor.

        // Locate or register the custom gutters for this editor.
        // Reset indices to ensure valid ones are found for this specific editor instance.
        self.line_gutter_index = -1;
        self.fold_gutter_index = -1;

        // Find existing custom gutters
        let count = editor.get_gutter_count();
        for i in 0..count {
            let name = editor.get_gutter_name(i);
            if name == code_edit::gutters::RELATIVE_NUMBERS.into() {
                self.line_gutter_index = i;
            } else if name == code_edit::gutters::CUSTOM_FOLD.into() {
                self.fold_gutter_index = i;
            }
        }

        // Create if missing: Line Numbers
        if self.line_gutter_index == -1 {
            editor.add_gutter();
            self.line_gutter_index = editor.get_gutter_count() - 1;
            editor.set_gutter_name(self.line_gutter_index, code_edit::gutters::RELATIVE_NUMBERS);
            editor.set_gutter_type(self.line_gutter_index, GutterType::STRING);
        }

        // Create if missing: Fold Gutter
        if self.fold_gutter_index == -1 {
            editor.add_gutter();
            self.fold_gutter_index = editor.get_gutter_count() - 1;
            editor.set_gutter_name(self.fold_gutter_index, code_edit::gutters::CUSTOM_FOLD);
            editor.set_gutter_type(self.fold_gutter_index, GutterType::ICON);
            editor.set_gutter_width(self.fold_gutter_index, 16);
        }
    }

    /// Sync settings from VimSettings and update state
    pub fn sync_settings(&mut self) {
        self.config_mode = VimSettings::line_number_mode();
        if self.config_mode == LineNumberMode::None {
            self.clear_custom_gutters();
        } else {
            self.setup_gutters();
            self.update_gutters();
        }
    }

    #[func]
    pub fn on_gutter_clicked(&mut self, line: i32, gutter: i32) {
        if gutter == self.fold_gutter_index {
            if let Some(mut editor) = self.editor.clone() {
                editor.toggle_foldable_line(line);
                // Update icons immediately after toggle
                self.force_next_update = true;
                self.update_gutters();
            }
        }
    }

    #[func]
    pub fn on_theme_changed(&mut self) {
        self.last_line_count = -1; // Invalidate cache
        self.force_next_update = true;
        self.update_gutters();
    }

    #[func]
    pub fn on_scroll_changed(&mut self, _value: f64) {
        // Scroll changed means visible lines may have changed; update_gutters checks visibility.
        self.update_gutters();
    }

    /// Deferred version of update_gutters - ensures cursor position is settled
    /// before recalculating relative line numbers.
    #[func]
    pub fn update_gutters_deferred(&mut self) {
        // Force update since caret changed - the deferred call ensures position is settled
        self.force_next_update = true;
        self.base_mut()
            .call_deferred(callbacks::UPDATE_GUTTERS, &[]);
    }

    #[func]
    pub fn update_gutters(&mut self) {
        let Some(mut editor) = self.editor.clone() else {
            return;
        };

        // Floating windows can re-enable standard gutters; force them off.
        if editor.is_draw_line_numbers_enabled() {
            editor.set_draw_line_numbers(false);
        }
        if editor.is_drawing_fold_gutter() {
            editor.set_draw_fold_gutter(false);
        }

        let caret_line = editor.get_caret_line();
        let first_line = editor.get_first_visible_line();

        // Skip update if nothing meaningful changed for the current mode.
        if !self.force_next_update {
            let scroll_changed = first_line != self.last_scroll_line;
            let caret_changed = caret_line != self.last_caret_line;

            if !scroll_changed {
                match self.config_mode {
                    LineNumberMode::Absolute => {
                        // Absolute numbers only change on scroll (or text change -> handled by force_update)
                        if !caret_changed {
                            return;
                        }
                        return; // Absolute mode: caret movement alone does not change numbers.
                    }
                    LineNumberMode::Relative | LineNumberMode::Hybrid => {
                        // Relative numbers change on Scroll OR Vertical Caret Move.
                        if !caret_changed {
                            return; // No scroll, no caret move -> IGNORE.
                        }
                        // If caret moved, but stayed on same line (horizontal), numbers don't change.
                        if caret_line == self.last_caret_line {
                            return;
                        }
                    }
                    LineNumberMode::None => return,
                }
            }
        }

        // Apply Update
        self.force_next_update = false;
        self.last_caret_line = caret_line;
        self.last_scroll_line = first_line;

        let line_count = editor.get_line_count();
        let visible_lines = editor.get_visible_line_count();
        let last_line = std::cmp::min(first_line + visible_lines + 5, line_count);

        // Fetch standard icons safely.
        let can_fold_icon = editor.get_theme_icon(theme::CAN_FOLD_ICON);
        let folded_icon = editor.get_theme_icon(theme::FOLDED_ICON);
        let line_number_color = editor.get_theme_color(theme::LINE_NUMBER_COLOR);

        for line in first_line..last_line {
            // Update line numbers.
            if self.line_gutter_index != -1 {
                if matches!(self.config_mode, LineNumberMode::None) {
                    editor.set_line_gutter_text(line, self.line_gutter_index, "");
                } else {
                    let num = if self.config_mode == LineNumberMode::Absolute {
                        format!("{}", line + 1)
                    } else {
                        self.calculate_number(line, caret_line)
                    };
                    editor.set_line_gutter_text(line, self.line_gutter_index, &num);
                    editor.set_line_gutter_item_color(
                        line,
                        self.line_gutter_index,
                        line_number_color,
                    );
                }

                self.update_gutter_width(&mut editor);
            }

            // Update fold icons.
            if self.fold_gutter_index != -1 {
                if editor.is_line_folded(line) {
                    if let Some(icon) = &folded_icon {
                        editor.set_line_gutter_icon(line, self.fold_gutter_index, icon);
                    }
                } else if editor.can_fold_line(line) {
                    if let Some(icon) = &can_fold_icon {
                        editor.set_line_gutter_icon(line, self.fold_gutter_index, icon);
                    }
                } else {
                    let empty_tex = self.get_empty_icon();
                    editor.set_line_gutter_icon(line, self.fold_gutter_index, &empty_tex);
                    editor.set_line_gutter_clickable(line, self.fold_gutter_index, false);
                }
            }
        }
    }

    fn calculate_number(&self, line_idx: i32, caret_line: i32) -> String {
        let diff = (line_idx - caret_line).abs();

        if diff == 0 {
            if self.config_mode == LineNumberMode::Hybrid {
                return format!("{}", line_idx + 1);
            } else {
                return "0".to_string();
            }
        }
        format!("{}", diff)
    }

    fn get_empty_icon(&mut self) -> Gd<Texture2D> {
        if let Some(tex) = &self.empty_icon {
            return tex.clone();
        }

        // Create 1x1 Transparent Image
        let Some(mut img) = Image::create(1, 1, false, Format::RGBA8) else {
            // Image creation failed; return a default texture.
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

        // Fetch font and calculate char width using the "font" and "font_size" theme items.
        let Some(font) = editor.get_theme_font(theme::FONT) else {
            log::warn!("Failed to get editor font for gutter width calculation");
            return;
        };
        let font_size = editor.get_theme_font_size(theme::FONT_SIZE);
        let char_width = font
            .get_string_size_ex("0")
            .alignment(HorizontalAlignment::LEFT)
            .font_size(font_size)
            .done()
            .x;

        // (Digits for padding) * Char Width
        let total_width = digits as f32 * char_width;

        editor.set_gutter_width(self.line_gutter_index, total_width as i32);
    }

    fn clear_custom_gutters(&mut self) {
        let Some(mut editor) = self.editor.clone() else {
            return;
        };
        if self.line_gutter_index != -1 {
            editor.remove_gutter(self.line_gutter_index);
            self.line_gutter_index = -1;
        }
        if self.fold_gutter_index != -1 {
            editor.remove_gutter(self.fold_gutter_index);
            self.fold_gutter_index = -1;
        }
    }
    fn disconnect_signals(&mut self, mut editor: Gd<CodeEdit>) {
        let callable_update = self.base().callable(callbacks::UPDATE_GUTTERS);
        let callable_update_deferred = self.base().callable(callbacks::UPDATE_GUTTERS_DEFERRED);

        if editor.is_connected(text_edit::signals::CARET_CHANGED, &callable_update_deferred) {
            editor.disconnect(text_edit::signals::CARET_CHANGED, &callable_update_deferred);
        }
        if editor.is_connected(text_edit::signals::TEXT_CHANGED, &callable_update) {
            editor.disconnect(text_edit::signals::TEXT_CHANGED, &callable_update);
        }
        if editor.is_connected(
            control::signals::VISIBILITY_CHANGED,
            &callable_update_deferred,
        ) {
            editor.disconnect(
                control::signals::VISIBILITY_CHANGED,
                &callable_update_deferred,
            );
        }

        // Gutter Click
        let callable_click = self.base().callable(callbacks::ON_GUTTER_CLICKED);
        if editor.is_connected(text_edit::signals::GUTTER_CLICKED, &callable_click) {
            editor.disconnect(text_edit::signals::GUTTER_CLICKED, &callable_click);
        }

        // Theme Changed
        let callable_theme = self.base().callable(callbacks::ON_THEME_CHANGED);
        if editor.is_connected(control::signals::THEME_CHANGED, &callable_theme) {
            editor.disconnect(control::signals::THEME_CHANGED, &callable_theme);
        }

        // Scroll
        if let Some(mut scroll) = editor.get_v_scroll_bar() {
            let callable_scroll = self.base().callable(callbacks::ON_SCROLL_CHANGED);
            if scroll.is_connected(range::signals::VALUE_CHANGED, &callable_scroll) {
                scroll.disconnect(range::signals::VALUE_CHANGED, &callable_scroll);
            }
        }
    }
}
