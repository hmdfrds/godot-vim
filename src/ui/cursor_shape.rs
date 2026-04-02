//! Vim cursor overlay — animated block/beam/underline with difference-blend shader.
//!
//! Two components:
//!
//! 1. **`CursorGeometry`** + **`compute_cursor_geometry()`**: pixel-perfect cursor placement
//!    using Godot's TextServer shaped-text API for correct tab/ligature handling.
//!
//! 2. **`VimCursor`** (`Control`-based GodotClass): the visible overlay that lerps toward
//!    the target position, blinks when stationary, and uses a GLSL difference-blend shader
//!    so the cursor character is always readable against any color scheme.

use godot::classes::control::MouseFilter;
use godot::classes::{
    CanvasItem, CodeEdit, Control, Font, IControl, Panel, Shader, ShaderMaterial, StyleBoxFlat,
    TextServerManager,
};
use godot::prelude::*;

use vim_core::primitives::Mode;

use crate::bridge::code_edit_ext::CodeEditExt;
use crate::bridge::codec;
use crate::safety::panic_guard;
use crate::types::CharLineCol;

// ─────────────────────────────────────────────────────────────────────────────
// CursorShapeMode — which cursor shape to display
// ─────────────────────────────────────────────────────────────────────────────

/// Cursor shape, derived from Vim mode. Each variant maps to a distinct
/// geometry in [`VimCursor::update_visual_shape`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CursorShapeMode {
    /// Full-cell block (Normal, Visual, Operator-Pending).
    Block,
    /// Thin vertical beam at the left edge of the cell (Insert).
    Beam,
    /// Thin horizontal underline at the bottom of the cell (Replace).
    Underline,
}

// ─────────────────────────────────────────────────────────────────────────────
// CursorGeometry — pixel-perfect cursor placement
// ─────────────────────────────────────────────────────────────────────────────

/// Floor for cursor width -- prevents invisible cursors on zero-width glyphs
/// (e.g. combining characters, ZWJ sequences).
const MIN_CURSOR_WIDTH: f32 = 2.0;

const BEAM_CURSOR_WIDTH: f32 = 2.0;
const UNDERLINE_CURSOR_HEIGHT: f32 = 4.0;

/// Must be above all editor content layers so the shader's screen_texture
/// sampling reads the final composited text, not an intermediate layer.
const CURSOR_Z_INDEX: i32 = 100;

/// Pixel-space cursor placement, computed from Godot's TextServer shaping.
/// Coordinates are relative to the `CodeEdit` origin, not the viewport.
#[derive(Debug, Clone, Copy)]
pub(crate) struct CursorGeometry {
    pub(crate) pos: Vector2,
    pub(crate) height: f32,
    /// Character cell width -- the VimCursor uses this for block/underline
    /// sizing, while beam mode ignores it in favor of a fixed pixel width.
    pub(crate) width: f32,
}

/// Compute pixel-space cursor geometry for the overlay.
///
/// `override_pos` decouples the Vim cursor from Godot's native caret. In visual
/// mode, Godot's caret sits at the exclusive selection end, but the Vim cursor
/// must render at the engine's head position -- `override_pos` provides that.
///
/// Returns `None` when Godot's layout is incomplete (sentinel rects, zero-y on
/// non-first lines), which happens during editor initialization or when the
/// target line is folded/off-screen.
pub(crate) fn compute_cursor_geometry(
    editor: &Gd<CodeEdit>,
    override_pos: Option<CharLineCol>,
) -> Option<CursorGeometry> {
    let line_height = editor.safe_line_height() as f32;
    let font = editor.get_theme_font("font")?;
    let font_size = editor.get_theme_font_size("font_size");
    let fallback_char_width = font.get_char_size('m' as u32, font_size).x.max(1.0);

    let (line, col) = if let Some(lc) = override_pos {
        (lc.line, lc.col)
    } else {
        (editor.get_caret_line(), editor.get_caret_column())
    };

    let rect = editor.get_rect_at_line_column(line, col);
    if is_invalid_rect(rect) {
        log::trace!("cursor_geom: invalid rect at line={} col={}", line, col);
        return None;
    }

    // Empty-document sentinel: all-zeros Rect2i. The cursor belongs at
    // line 0 col 0, so use get_caret_draw_pos().x for correct gutter offset.
    if is_empty_doc_rect(rect) {
        log::trace!("cursor_geom: empty document, using fallback geometry");
        return Some(CursorGeometry {
            pos: Vector2::new(
                editor.get_caret_draw_pos().x,
                0.0,
            ),
            height: line_height,
            width: fallback_char_width,
        });
    }

    let target_y = rect.position.y as f32;
    // Zero y on line > 0 means Godot hasn't laid out this line yet (e.g.
    // the editor was just added to the tree). Returning None lets the caller
    // keep the previous cursor position until layout stabilizes.
    if target_y.abs() < f32::EPSILON && line > 0 {
        log::trace!("cursor_geom: zero y for line={}", line);
        return None;
    }

    let height = {
        let h = rect.size.y as f32;
        if h > 0.1 { h } else { line_height }
    };

    // chars().len() gives a UTF-32 character count, matching
    // get_caret_column()'s semantics (not byte or grapheme count).
    let line_len = editor.get_line(line).chars().len();

    let (target_x, width) = if override_pos.is_some() {
        // Override path: shapes the line once for both x and width.
        compute_override_x_and_width(
            editor, &font, font_size, line, col, line_len, fallback_char_width,
        )
    } else {
        // Native caret path: x from Godot's built-in draw_pos (free),
        // width from a shaped-text measurement (cached per line).
        let x = editor.get_caret_draw_pos().x;
        let w = compute_char_width_ts(editor, &font, font_size, line, col, line_len, fallback_char_width);
        (x, w)
    };

    Some(CursorGeometry {
        pos: Vector2::new(target_x, target_y),
        height,
        width,
    })
}

/// Compute x-coordinate and character width for an override position using a
/// single shaped-text session (avoids shaping the line twice).
///
/// Two strategies for the x-coordinate depending on whether the override is on
/// the same line as the native caret:
/// - **Same line**: delta from native caret's shaped position to override col,
///   added to `get_caret_draw_pos().x`. This inherits Godot's correct gutter
///   and scroll offsets.
/// - **Different line**: absolute position via `get_rect_at_line_column(line, 0)`
///   base + shaped offset from col 0. Less accurate but the only option when
///   the native caret is on another line.
fn compute_override_x_and_width(
    editor: &Gd<CodeEdit>,
    font: &Gd<Font>,
    font_size: i32,
    line: i32,
    col: i32,
    line_len: usize,
    fallback_char_width: f32,
) -> (f32, f32) {
    let draw_pos = editor.get_caret_draw_pos();
    let caret_line = editor.get_caret_line();
    let caret_col = editor.get_caret_column();

    // Fast path: override matches native caret, so draw_pos is exact.
    if caret_line == line && caret_col == col {
        let w = compute_char_width_ts(editor, font, font_size, line, col, line_len, fallback_char_width);
        return (draw_pos.x, w);
    }

    let result = with_shaped_text(editor, font, font_size, line, |ts, rid| {
        let x = if caret_line == line {
            // Same line: shaped delta from native caret col to target col,
            // anchored to draw_pos.x which already accounts for gutter/scroll.
            let target_x = caret_x_from_dict(&ts.shaped_text_get_carets(rid, col as i64));
            let caret_x = caret_x_from_dict(&ts.shaped_text_get_carets(rid, caret_col as i64));
            match (target_x, caret_x) {
                (Some(tx), Some(cx)) => {
                    let result = draw_pos.x + (tx - cx);
                    if is_sane_coord(result) { Some(result) } else { None }
                }
                _ => None,
            }
        } else {
            // Different line: col-0 rect provides the pixel base, then
            // shaped offset from col 0 to target col gives the x delta.
            let base_rect = editor.get_rect_at_line_column(line, 0);
            if is_invalid_rect(base_rect) || is_empty_doc_rect(base_rect) {
                None
            } else {
                let base_x = base_rect.position.x as f32;
                let col_x = caret_x_from_dict(&ts.shaped_text_get_carets(rid, col as i64));
                let col0_x = caret_x_from_dict(&ts.shaped_text_get_carets(rid, 0));
                match (col_x, col0_x) {
                    (Some(cx), Some(c0)) => {
                        let result = base_x + (cx - c0);
                        if is_sane_coord(result) { Some(result) } else { None }
                    }
                    _ => None,
                }
            }
        };

        // Width: shaped delta between col and col+1.
        let width = if codec::i32_to_usize(col) < line_len {
            let col_next = caret_x_from_dict(&ts.shaped_text_get_carets(rid, (col + 1) as i64));
            let col_cur = caret_x_from_dict(&ts.shaped_text_get_carets(rid, col as i64));
            match (col_next, col_cur) {
                (Some(nx), Some(cx)) => {
                    let w = (nx - cx).abs();
                    if is_sane_coord(w) && w >= MIN_CURSOR_WIDTH { w } else { fallback_char_width }
                }
                _ => fallback_char_width,
            }
        } else {
            fallback_char_width
        };

        Some((x, width))
    });

    let fallback_x = || -> f32 {
        let rect = editor.get_rect_at_line_column(line, col);
        if is_invalid_rect(rect) || is_empty_doc_rect(rect) {
            draw_pos.x
        } else {
            rect.position.x as f32
        }
    };

    match result {
        Some((Some(x), w)) => (x, w),
        Some((None, w)) => (fallback_x(), w),
        None => (fallback_x(), fallback_char_width),
    }
}

/// Caches a single shaped-text RID to avoid re-shaping on every keystroke.
///
/// Holds a TextServer RID for exactly one line at a time. When the cursor
/// moves to a different line, the old RID is freed and a new one shaped.
/// The TextServer `Gd` reference is kept alive to ensure the RID remains
/// valid and can be freed on invalidation or drop.
struct ShapedTextCache {
    /// -1 = no cached line.
    line: i32,
    rid: Rid,
    /// Kept alive so the RID can be freed. `None` when cache is empty.
    ts: Option<Gd<godot::classes::TextServer>>,
}

impl ShapedTextCache {
    fn new() -> Self {
        Self {
            line: -1,
            rid: Rid::new(0),
            ts: None,
        }
    }

    fn get_or_shape(
        &mut self,
        editor: &Gd<CodeEdit>,
        font: &Gd<Font>,
        font_size: i32,
        line: i32,
    ) -> Option<(Rid, &mut Gd<godot::classes::TextServer>)> {
        if self.line == line && self.ts.is_some() {
            let ts = self.ts.as_mut()?;
            return Some((self.rid, ts));
        }

        self.invalidate();

        let tsm = TextServerManager::singleton();
        let mut ts = tsm.get_primary_interface()?;

        let line_text = editor.get_line(line);
        let font_rids = font.get_rids();

        let rid = ts.create_shaped_text();
        ts.shaped_text_add_string(rid, &line_text, &font_rids, font_size as i64);

        let tab_size = editor.safe_tab_size();
        let space_w = font.get_char_size(' ' as u32, font_size).x;
        let tab_px = space_w * tab_size as f32;
        let tab_stops = PackedFloat32Array::from(&[tab_px]);
        ts.shaped_text_tab_align(rid, &tab_stops);

        self.line = line;
        self.rid = rid;
        self.ts = Some(ts);

        let ts_ref = self.ts.as_mut()?;
        Some((self.rid, ts_ref))
    }

    fn invalidate(&mut self) {
        // Guard: only free when Godot is still running, otherwise the
        // TextServer singleton is already destroyed and the call would crash.
        if self.line >= 0 && godot::sys::is_initialized() {
            if let Some(ref mut ts) = self.ts {
                ts.free_rid(self.rid);
            }
        }
        self.line = -1;
        self.rid = Rid::new(0);
        self.ts = None;
    }
}

impl Drop for ShapedTextCache {
    fn drop(&mut self) {
        self.invalidate();
    }
}

// Thread-local: Godot is single-threaded, and multiple `with_shaped_text`
// calls in the same frame (x + width) reuse the cached RID.
thread_local! {
    static SHAPED_CACHE: std::cell::RefCell<ShapedTextCache> =
        std::cell::RefCell::new(ShapedTextCache::new());
}

fn with_shaped_text<T>(
    editor: &Gd<CodeEdit>,
    font: &Gd<Font>,
    font_size: i32,
    line: i32,
    f: impl FnOnce(&mut Gd<godot::classes::TextServer>, Rid) -> Option<T>,
) -> Option<T> {
    SHAPED_CACHE.with(|cache| {
        let mut cache = cache.borrow_mut();
        let (rid, ts) = cache.get_or_shape(editor, font, font_size, line)?;
        f(ts, rid)
    })
}

/// Must be called after any text mutation to prevent stale glyph measurements.
pub(crate) fn invalidate_shaped_cache() {
    SHAPED_CACHE.with(|cache| {
        cache.borrow_mut().invalidate();
    });
}

fn shaped_text_caret_delta(
    editor: &Gd<CodeEdit>,
    font: &Gd<Font>,
    font_size: i32,
    line: i32,
    target_col: i32,
    caret_col: i32,
) -> Option<f32> {
    with_shaped_text(editor, font, font_size, line, |ts, rid| {
        let target_x = caret_x_from_dict(&ts.shaped_text_get_carets(rid, target_col as i64));
        let caret_x = caret_x_from_dict(&ts.shaped_text_get_carets(rid, caret_col as i64));
        Some(target_x? - caret_x?)
    })
}

/// Extract the leading caret x from `shaped_text_get_carets()`.
///
/// Godot always inserts both `leading_rect` and `trailing_rect` keys, even
/// when no glyph boundary exists -- those zero-initialized `Rect2{}` sentinels
/// are detected by checking for zero size. Prefers leading, falls back to
/// trailing.
fn caret_x_from_dict(dict: &VarDictionary) -> Option<f32> {
    if let Some(rect) = dict.get("leading_rect") {
        let r: Rect2 = rect.try_to().ok()?;
        if r.size.x != 0.0 || r.size.y != 0.0 {
            return Some(r.position.x);
        }
    }
    if let Some(rect) = dict.get("trailing_rect") {
        let r: Rect2 = rect.try_to().ok()?;
        if r.size.x != 0.0 || r.size.y != 0.0 {
            return Some(r.position.x);
        }
    }
    log::trace!("caret_x_from_dict: no non-zero caret rect found");
    None
}

/// Shaped-text character width at `(line, col)` via col-to-col+1 delta.
/// Only used on the non-override (native caret) path; the override path
/// computes width inside `compute_override_x_and_width` to share the RID.
fn compute_char_width_ts(
    editor: &Gd<CodeEdit>,
    font: &Gd<Font>,
    font_size: i32,
    line: i32,
    col: i32,
    line_len: usize,
    fallback: f32,
) -> f32 {
    if codec::i32_to_usize(col) >= line_len {
        return fallback;
    }

    if let Some(delta) = shaped_text_caret_delta(editor, font, font_size, line, col + 1, col) {
        let w = delta.abs();
        if is_sane_coord(w) && w >= MIN_CURSOR_WIDTH {
            return w;
        }
    }

    fallback
}

/// Godot's (-1,-1) sentinel for off-screen or not-yet-laid-out positions.
fn is_invalid_rect(rect: Rect2i) -> bool {
    rect.position.x == -1 && rect.position.y == -1
}

/// Godot's all-zeros sentinel for empty documents. Distinct from (-1,-1).
fn is_empty_doc_rect(rect: Rect2i) -> bool {
    rect.position.x == 0 && rect.position.y == 0 && rect.size.x == 0 && rect.size.y == 0
}

/// Guard against NaN/Infinity from TextServer shaped-text APIs.
fn is_sane_coord(v: f32) -> bool {
    v.is_finite() && v.abs() < 100_000.0
}

// ─────────────────────────────────────────────────────────────────────────────
// CursorColorMap — user-configurable cursor colors per Vim mode
// ─────────────────────────────────────────────────────────────────────────────

/// Per-mode cursor colors. The alpha channel of each color doubles as the
/// `base_alpha` ceiling for the blink cycle (so a semi-transparent color
/// produces a dimmer cursor at full visibility).
#[derive(Debug, Clone)]
pub(crate) struct CursorColorMap {
    pub(crate) normal: Color,
    pub(crate) insert: Color,
    pub(crate) visual: Color,
    pub(crate) replace: Color,
    pub(crate) operator: Color,
    pub(crate) command: Color,
}

impl Default for CursorColorMap {
    fn default() -> Self {
        use crate::settings::defaults;
        Self {
            normal: defaults::cursor_normal(),
            insert: defaults::cursor_insert(),
            visual: defaults::cursor_visual(),
            replace: defaults::cursor_replace(),
            operator: defaults::cursor_operator(),
            command: defaults::cursor_command(),
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// CursorAnimation — lerp and blink state
// ─────────────────────────────────────────────────────────────────────────────

/// Lerp + blink state, updated every `_process` frame.
struct CursorAnimation {
    target_pos: Vector2,
    current_pos: Vector2,
    /// Exponential-decay factor for `current.lerp(target, 1 - exp(-speed * dt))`.
    lerp_speed: f64,
    /// Radians/sec for `sin(blink_time)` square-wave blink.
    blink_speed: f64,
    /// Accumulated blink phase; reset on any movement to keep cursor visible
    /// during rapid keystrokes.
    blink_time: f64,
    /// False until the first `set_target` call. Prevents lerping from (0,0)
    /// to the real position on initial attach.
    positioned: bool,
    /// Dedup guard: skip `set_self_modulate` when alpha hasn't changed.
    last_alpha: f32,
    /// Cached from parent Control in `set_target`. Used in `_process` to
    /// hide the cursor when it extends below the visible editor rect --
    /// necessary because the screen_texture shader bypasses `clip_children`.
    cached_editor_height: f32,
}

impl Default for CursorAnimation {
    fn default() -> Self {
        use crate::settings::defaults;
        Self {
            target_pos: Vector2::ZERO,
            current_pos: Vector2::ZERO,
            lerp_speed: defaults::CURSOR_LERP_SPEED,
            // No blink until apply_settings reads from Godot's native caret_blink.
            blink_speed: 0.0,
            blink_time: 0.0,
            positioned: false,
            last_alpha: -1.0,
            cached_editor_height: 0.0,
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// VimCursor — animated, shader-blended cursor overlay
// ─────────────────────────────────────────────────────────────────────────────

/// GLSL difference-blend: `abs(bg - fg)` makes the cursor character legible
/// against any background by inverting toward maximum contrast. The alpha
/// channel is used as a mix factor, not as transparency, so the final
/// `COLOR.a = 1.0` avoids double-blending with Godot's compositor.
const CURSOR_SHADER_CODE: &str = r#"
shader_type canvas_item;
uniform sampler2D screen_texture : hint_screen_texture, repeat_disable, filter_nearest;
void fragment() {
    vec4 bg = texture(screen_texture, SCREEN_UV);
    float a = COLOR.a;
    vec3 diff = abs(bg.rgb - COLOR.rgb);
    COLOR.rgb = mix(bg.rgb, diff, a);
    COLOR.a = 1.0;
}
"#;

/// Animated Vim cursor overlay, rendered as a `Control` with a child `Panel`.
///
/// Godot's built-in caret offers only line or block shapes with no per-mode
/// color, no animation, and no difference-blend. This overlay replaces it
/// entirely: we make the native caret transparent and drive this node from
/// `CursorGeometry` computed via TextServer shaping.
///
/// **Shape**: Block (Normal/Visual/Op-Pending), Beam (Insert), Underline (Replace).
/// **Color**: per-mode via `CursorColorMap`, with alpha used as blink ceiling.
/// **Blend**: GLSL difference shader reads `screen_texture` so the character
/// underneath is always legible regardless of color scheme.
/// **Animation**: exponential-decay lerp toward target; square-wave blink when
/// stationary.
#[derive(GodotClass)]
#[class(base = Control)]
pub struct VimCursor {
    base: Base<Control>,

    /// Child Panel with difference-blend shader. Created in `ready()`.
    visual: Option<Gd<Panel>>,
    animation: CursorAnimation,

    /// Character cell dimensions from the latest `CursorGeometry`. Used to
    /// size the block/underline shapes and position the underline at the
    /// cell bottom.
    font_height: f32,
    char_width: f32,

    /// Alpha ceiling from the current mode's color. The blink cycle
    /// oscillates between 0 and this value.
    base_alpha: f32,
    shape_mode: CursorShapeMode,

    /// Reused across mode changes to avoid StyleBoxFlat allocation churn.
    cached_style: Option<Gd<StyleBoxFlat>>,
    color_map: CursorColorMap,
    beam_width: f32,
    underline_height: f32,
}

// ─────────────────────────────────────────────────────────────────────────────
// IControl lifecycle
// ─────────────────────────────────────────────────────────────────────────────

#[godot_api]
impl IControl for VimCursor {
    fn init(base: Base<Control>) -> Self {
        Self {
            base,
            visual: None,
            animation: CursorAnimation::default(),
            font_height: 20.0,
            char_width: 10.0,
            base_alpha: 0.5,
            shape_mode: CursorShapeMode::Block,
            cached_style: None,
            color_map: CursorColorMap::default(),
            beam_width: BEAM_CURSOR_WIDTH,
            underline_height: UNDERLINE_CURSOR_HEIGHT,
        }
    }

    fn ready(&mut self) {
        panic_guard(|| {
            let mut visual = Panel::new_alloc();
            visual.set_name("CursorShape");
            visual.set_mouse_filter(MouseFilter::IGNORE);

            // Attach the difference-blend shader so the cursor inverts
            // against whatever text/background is underneath.
            let mut shader = Shader::new_gd();
            shader.set_code(CURSOR_SHADER_CODE);
            let mut material = ShaderMaterial::new_gd();
            material.set_shader(&shader);
            visual.set_material(&material.upcast::<godot::classes::Material>());

            let visual_node: Gd<godot::classes::Node> = visual.clone().upcast();
            self.base_mut().add_child(&visual_node);

            // Both the Control wrapper and the Panel child need high z-index
            // so the shader's screen_texture reads fully composited text.
            visual.clone().upcast::<CanvasItem>().set_z_index(CURSOR_Z_INDEX);
            self.base().clone().upcast::<CanvasItem>().set_z_index(CURSOR_Z_INDEX);

            // Pass-through: clicks should reach the editor, not the cursor.
            self.base_mut().set_mouse_filter(MouseFilter::IGNORE);

            self.visual = Some(visual);
            self.update_visual_style(Mode::Normal);
            self.update_visual_shape();
            self.base_mut().set_process(true);
        }, ());
    }

    fn process(&mut self, delta: f64) {
        panic_guard(|| {
            let current = self.animation.current_pos;
            let target = self.animation.target_pos;
            let dist = target - current;
            let is_moving = dist.length_squared() > 0.25;

            if is_moving {
                // Exponential decay lerp: frame-rate independent, converges
                // smoothly regardless of delta fluctuations.
                let new_pos = current.lerp(target, (1.0 - (-self.animation.lerp_speed * delta).exp()) as f32);
                self.animation.current_pos = new_pos;
                self.base_mut().set_position(new_pos);
                // Stay fully visible while moving so rapid keystrokes don't
                // produce a flickering cursor.
                self.animation.blink_time = 0.0;
                self.update_alpha(1.0);
            } else {
                // Snap to exact target to avoid sub-pixel jitter.
                if dist.length_squared() > 0.0 {
                    self.animation.current_pos = target;
                    self.base_mut().set_position(target);
                }

                // Square-wave blink: sin(t) >= 0 -> visible, < 0 -> hidden.
                self.animation.blink_time += delta * self.animation.blink_speed;
                let blink_factor = if self.animation.blink_time.sin() >= 0.0 {
                    1.0
                } else {
                    0.0
                };
                self.update_alpha(self.base_alpha * blink_factor);
            }

            // Manual clip: the screen_texture shader bypasses Godot's
            // clip_children, so we hide the cursor when it extends below
            // the editor rect (e.g. bottom dock overlap).
            if self.animation.cached_editor_height > 0.0 {
                let cursor_bottom = self.animation.current_pos.y + self.font_height;
                if let Some(visual) = self.visual.clone() {
                    visual.upcast::<CanvasItem>().set_visible(cursor_bottom <= self.animation.cached_editor_height);
                }
            }
        }, ());
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Public API
// ─────────────────────────────────────────────────────────────────────────────

#[godot_api]
impl VimCursor {
    pub fn set_target(&mut self, target: Vector2, font_height: f32, char_width: f32) {
        self.animation.target_pos = target;

        // Cache editor height once per target update rather than every _process
        // frame (avoids get_parent + try_cast + get_rect at 60Hz).
        if let Some(parent) = self.base().get_parent() {
            if let Ok(editor) = parent.try_cast::<Control>() {
                self.animation.cached_editor_height = editor.get_rect().size.y;
            }
        }

        if self.visual.is_none() {
            log::trace!("set_target: visual not ready, deferred until after ready()");
        }

        let dims_changed = font_height != self.font_height || char_width != self.char_width;
        self.font_height = font_height;
        self.char_width = char_width;

        if !self.animation.positioned {
            self.animation.current_pos = target;
            self.base_mut().set_position(target);
            self.animation.positioned = true;
        }

        self.animation.blink_time = 0.0;
        self.update_alpha(1.0);
        self.base_mut().set_process(true);

        if dims_changed {
            self.update_visual_shape();
        }
    }

    pub fn set_mode(&mut self, mode: Mode) {
        self.shape_mode = if mode.is_insert() {
            CursorShapeMode::Beam
        } else if mode.is_replace() {
            CursorShapeMode::Underline
        } else {
            CursorShapeMode::Block
        };
        self.animation.blink_time = 0.0;
        self.update_alpha(1.0);
        self.update_visual_style(mode);
        self.update_visual_shape();
    }

    /// Takes effect on the next `set_mode` call.
    pub fn set_color_map(&mut self, map: CursorColorMap) {
        self.color_map = map;
    }

    /// Skip lerp and teleport to target. Called on attach so the cursor
    /// doesn't visibly slide from (0,0).
    pub fn force_snap(&mut self) {
        let target = self.animation.target_pos;
        self.animation.current_pos = target;
        self.base_mut().set_position(target);
        self.animation.positioned = true;
    }

    pub fn set_animation(&mut self, lerp_speed: f64, blink_speed: f64) {
        self.animation.lerp_speed = lerp_speed;
        self.animation.blink_speed = blink_speed;
    }

    pub fn set_dimensions(&mut self, beam_width: f32, underline_height: f32) {
        self.beam_width = beam_width;
        self.underline_height = underline_height;
        self.update_visual_shape();
    }

}

// ─────────────────────────────────────────────────────────────────────────────
// Private helpers
// ─────────────────────────────────────────────────────────────────────────────

impl VimCursor {
    /// `visual.clone()` is a `Gd<T>` refcount bump (not a heap alloc) --
    /// idiomatic gdext for accessing a child while `&mut self` is held.
    fn update_alpha(&mut self, alpha: f32) {
        if alpha == self.animation.last_alpha {
            return;
        }
        self.animation.last_alpha = alpha;
        if let Some(mut visual) = self.visual.clone() {
            visual.set_self_modulate(Color::from_rgba(1.0, 1.0, 1.0, alpha));
        }
    }

    fn update_visual_style(&mut self, mode: Mode) {
        let Some(mut visual) = self.visual.clone() else {
            return;
        };

        let color = match mode {
            Mode::Normal => self.color_map.normal,
            Mode::Insert => self.color_map.insert,
            Mode::Visual(_) => self.color_map.visual,
            Mode::Replace | Mode::VirtualReplace => self.color_map.replace,
            Mode::OperatorPending(_) => self.color_map.operator,
            Mode::CommandLine => self.color_map.command,
            // Mode is #[non_exhaustive] in vim-core.
            _ => self.color_map.normal,
        };

        if let Some(ref mut style) = self.cached_style {
            style.set_bg_color(color);
        } else {
            let mut style = StyleBoxFlat::new_gd();
            style.set_bg_color(color);
            style.set_draw_center(true);
            style.set_border_width_all(0);
            let style_box: Gd<godot::classes::StyleBox> = style.clone().upcast();
            visual.add_theme_stylebox_override("panel", &style_box);
            self.cached_style = Some(style);
        }

        self.base_alpha = color.a;
    }

    /// Resize the Panel child to match the current shape mode and cell
    /// dimensions. Underline is positioned at `font_height - underline_height`
    /// so it sits at the cell bottom edge.
    fn update_visual_shape(&mut self) {
        let Some(mut visual) = self.visual.clone() else {
            return;
        };

        match self.shape_mode {
            CursorShapeMode::Beam => {
                visual.set_size(Vector2::new(self.beam_width, self.font_height));
                visual.set_position(Vector2::new(0.0, 0.0));
            }
            CursorShapeMode::Underline => {
                visual.set_size(Vector2::new(self.char_width, self.underline_height));
                visual.set_position(Vector2::new(
                    0.0,
                    self.font_height - self.underline_height,
                ));
            }
            CursorShapeMode::Block => {
                visual.set_size(Vector2::new(self.char_width, self.font_height));
                visual.set_position(Vector2::new(0.0, 0.0));
            }
        }
    }
}
