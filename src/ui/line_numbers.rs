//! Relative and hybrid line numbers drawn beside Godot's own gutters.
//!
//! Absolute mode leaves the `CodeEdit` alone. Relative and Hybrid hide only
//! Godot's `line_numbers` gutter and insert one STRING gutter right after it,
//! so the breakpoint, connection and fold gutters stay Godot's. Visibility,
//! width, zero padding and per-line colours are read from Godot on every
//! update. None hides Godot's numbers and nothing else.

use godot::classes::text_edit::GutterType;
use godot::classes::{CodeEdit, EditorInterface, EditorSettings, INode, Node};
use godot::prelude::*;

use crate::safety::panic_guard;
use crate::settings::LineNumberMode;

const SIG_CARET_CHANGED: &str = "caret_changed";
const SIG_TEXT_CHANGED: &str = "text_changed";
const SIG_VALUE_CHANGED: &str = "value_changed";
const SIG_THEME_CHANGED: &str = "theme_changed";
const SIG_VISIBILITY_CHANGED: &str = "visibility_changed";
// User signal fold_line, unfold_line and their _all variants emit
// (code_edit.cpp@4.7:2002-2030, declared at :4184; same in 4.5).
const SIG_FOLD_UPDATED: &str = "_fold_line_updated";
const SIG_DRAW: &str = "draw";
const SIG_SETTINGS_CHANGED: &str = "settings_changed";

/// +1 for the exclusive range, +5 for smooth-scroll lookahead.
const VISIBLE_LINE_BUFFER: i32 = 6;

const GUTTER_NAME: &str = "VimRelativeNumbers";
const NATIVE_GUTTER_NAME: &str = "line_numbers";
// The key CodeTextEditor applies to its CodeEdit (code_editor.cpp@4.7:1124).
const SHOW_LINE_NUMBERS: &str = "text_editor/appearance/gutters/show_line_numbers";
// Godot 4.6+ only; Nil on 4.5, where the minimum is one digit.
const MIN_DIGITS_PROPERTY: &str = "gutters_line_numbers_min_digits";

#[derive(GodotClass)]
#[class(base = Node)]
pub struct LineNumberManager {
    base: Base<Node>,
    editor: Option<Gd<CodeEdit>>,
    /// None until the first `set_mode`, so attaching in Absolute never
    /// touches the editor.
    mode: Option<LineNumberMode>,
    /// Godot's own choice for this editor's line numbers.
    native_shown: bool,
    /// We turned Godot's number gutter off and must turn it back on.
    hid_native: bool,
    last_caret_line: i32,
    last_scroll_line: i32,
    force_next_update: bool,
    colors_queued: bool,
}

#[godot_api]
impl INode for LineNumberManager {
    fn init(base: Base<Node>) -> Self {
        Self {
            base,
            editor: None,
            mode: None,
            native_shown: true,
            hid_native: false,
            last_caret_line: -1,
            last_scroll_line: -1,
            force_next_update: true,
            colors_queued: false,
        }
    }

    fn exit_tree(&mut self) {
        panic_guard("line_numbers::exit_tree", || self.detach(), ());
    }
}

#[godot_api]
impl LineNumberManager {
    pub fn attach(&mut self, editor: Gd<CodeEdit>) {
        if let Some(current) = &self.editor {
            if current.is_instance_valid() && current.instance_id() == editor.instance_id() {
                return;
            }
            self.detach();
        }
        self.hid_native = false;
        self.native_shown = native_intent(&editor, false, true);
        self.editor = Some(editor.clone());
        self.connect_signals(editor);
        self.apply_layout();
        self.force_update();
    }

    pub fn set_mode(&mut self, mode: LineNumberMode) {
        if self.mode != Some(mode) {
            self.mode = Some(mode);
            self.apply_layout();
            self.force_update();
        }
    }

    pub fn detach(&mut self) {
        let Some(mut editor) = self.editor.take() else {
            return;
        };
        if !editor.is_instance_valid() {
            return;
        }
        self.disconnect_signals(editor.clone());
        if let Some(idx) = find_gutter(&editor, GUTTER_NAME) {
            if can_remove_gutter(editor.get_line_count(), idx) {
                editor.remove_gutter(idx);
            } else {
                // The next attach reuses it by name.
                editor.set_gutter_draw(idx, false);
            }
        }
        if self.hid_native {
            editor.set_draw_line_numbers(self.native_shown);
            self.hid_native = false;
        }
    }

    /// CodeTextEditor re-applies its gutter settings on theme change
    /// (code_editor.cpp@4.7:1645), so re-read after the whole propagation.
    #[func]
    pub fn on_theme_changed(&mut self) {
        panic_guard(
            "line_numbers::on_theme_changed",
            || {
                self.base_mut().call_deferred("reread_settings", &[]);
            },
            (),
        );
    }

    #[func]
    pub fn on_scroll_changed(&mut self, _value: f64) {
        panic_guard(
            "line_numbers::on_scroll_changed",
            || self.update_gutters(),
            (),
        );
    }

    #[func]
    pub fn on_text_changed(&mut self) {
        panic_guard("line_numbers::on_text_changed", || self.force_update(), ());
    }

    #[func]
    pub fn on_fold_updated(&mut self) {
        panic_guard("line_numbers::on_fold_updated", || self.force_update(), ());
    }

    /// Deferred so the caret has settled before it is read.
    #[func]
    pub fn on_caret_changed(&mut self) {
        panic_guard(
            "line_numbers::on_caret_changed",
            || {
                self.base_mut().call_deferred("update_gutters", &[]);
            },
            (),
        );
    }

    /// Godot re-applies its gutter settings when a floating script editor is
    /// shown again, so re-check after visibility changes.
    #[func]
    pub fn on_visibility_changed(&mut self) {
        panic_guard(
            "line_numbers::on_visibility_changed",
            || {
                self.force_next_update = true;
                self.base_mut().call_deferred("update_gutters", &[]);
            },
            (),
        );
    }

    /// The script editor recolours number cells after validation without a
    /// signal; mirror them after each redraw. Setters are no-ops when
    /// unchanged, so this settles after one pass.
    #[func]
    pub fn on_draw(&mut self) {
        panic_guard(
            "line_numbers::on_draw",
            || {
                if !self.colors_queued && self.custom_numbers_active() {
                    self.colors_queued = true;
                    self.base_mut().call_deferred("sync_colors", &[]);
                }
            },
            (),
        );
    }

    /// Deferred so Godot's own handler has re-applied its settings first.
    #[func]
    pub fn on_settings_changed(&mut self) {
        panic_guard(
            "line_numbers::on_settings_changed",
            || {
                self.base_mut().call_deferred("reread_settings", &[]);
            },
            (),
        );
    }

    #[func]
    pub fn reread_settings(&mut self) {
        panic_guard(
            "line_numbers::reread_settings",
            || {
                let Some(editor) = self.valid_editor() else {
                    return;
                };
                self.native_shown = native_intent(&editor, self.hid_native, self.native_shown);
                self.apply_layout();
                self.force_update();
            },
            (),
        );
    }

    #[func]
    pub fn sync_colors(&mut self) {
        panic_guard(
            "line_numbers::sync_colors",
            || {
                self.colors_queued = false;
                let Some(mut editor) = self.valid_editor() else {
                    return;
                };
                let (Some(native), Some(ours)) = (
                    find_gutter(&editor, NATIVE_GUTTER_NAME),
                    find_gutter(&editor, GUTTER_NAME),
                ) else {
                    return;
                };
                let default_color = editor.get_theme_color("line_number_color");
                let (first, last) = visible_range(&editor);
                for line in first..last {
                    let color = number_color(&editor, line, native, default_color);
                    editor.set_line_gutter_item_color(line, ours, color);
                }
            },
            (),
        );
    }

    #[func]
    pub fn update_gutters(&mut self) {
        panic_guard(
            "line_numbers::update_gutters",
            || self.update_gutters_impl(),
            (),
        );
    }
}

impl LineNumberManager {
    fn valid_editor(&mut self) -> Option<Gd<CodeEdit>> {
        let editor = self.editor.clone()?;
        if editor.is_instance_valid() {
            Some(editor)
        } else {
            self.editor = None;
            None
        }
    }

    fn force_update(&mut self) {
        self.force_next_update = true;
        self.update_gutters();
    }

    fn custom_numbers_active(&self) -> bool {
        layout(self.mode, self.native_shown).custom
    }

    /// Show or hide Godot's number gutter and ours for the current mode.
    fn apply_layout(&mut self) {
        let Some(mut editor) = self.valid_editor() else {
            return;
        };
        let Layout {
            hide_native,
            custom,
        } = layout(self.mode, self.native_shown);
        if hide_native {
            if editor.is_draw_line_numbers_enabled() {
                editor.set_draw_line_numbers(false);
            }
            self.hid_native = true;
        } else if self.hid_native {
            editor.set_draw_line_numbers(self.native_shown);
            self.hid_native = false;
        }
        match (custom, find_gutter(&editor, GUTTER_NAME)) {
            (true, Some(idx)) => editor.set_gutter_draw(idx, true),
            (true, None) => {
                let Some(native) = find_gutter(&editor, NATIVE_GUTTER_NAME) else {
                    return;
                };
                let at = native + 1;
                editor.add_gutter_ex().at(at).done();
                editor.set_gutter_name(at, GUTTER_NAME);
                editor.set_gutter_type(at, GutterType::STRING);
            }
            (false, Some(idx)) => editor.set_gutter_draw(idx, false),
            (false, None) => {}
        }
    }

    fn update_gutters_impl(&mut self) {
        let Some(mut editor) = self.valid_editor() else {
            return;
        };

        // While we are not hiding it the flag is Godot's; while we are, Godot
        // drawing it again (settings re-applied, floating window shown) means on.
        let drawn = editor.is_draw_line_numbers_enabled();
        let shown = if self.hid_native {
            self.native_shown || drawn
        } else {
            drawn
        };
        if shown != self.native_shown || (self.hid_native && drawn) {
            self.native_shown = shown;
            self.apply_layout();
        }
        if !self.custom_numbers_active() {
            return;
        }
        let (Some(native), Some(ours)) = (
            find_gutter(&editor, NATIVE_GUTTER_NAME),
            find_gutter(&editor, GUTTER_NAME),
        ) else {
            return;
        };

        let caret_line = editor.get_caret_line();
        let first_line = editor.get_first_visible_line();
        if !self.force_next_update
            && first_line == self.last_scroll_line
            && caret_line == self.last_caret_line
        {
            return;
        }
        self.force_next_update = false;
        self.last_caret_line = caret_line;
        self.last_scroll_line = first_line;

        let width = editor.get_gutter_width(native);
        if editor.get_gutter_width(ours) != width {
            editor.set_gutter_width(ours, width);
        }

        let line_count = editor.get_line_count();
        let min_digits = editor.get(MIN_DIGITS_PROPERTY).try_to::<i32>().unwrap_or(1);
        let digits = crate::bridge::codec::i32_to_usize(min_digits.max(digit_count(line_count)));
        let pad = if editor.is_line_numbers_zero_padded() {
            '0'
        } else {
            ' '
        };
        let folds = FoldMap::from_editor(&editor);
        let default_color = editor.get_theme_color("line_number_color");
        let hybrid = self.mode == Some(LineNumberMode::Hybrid);

        let (first, last) = visible_range(&editor);
        let mut buf = String::with_capacity(digits + 2);
        for line in first..last {
            let value = if line == caret_line {
                if hybrid {
                    line + 1
                } else {
                    0
                }
            } else {
                folds.visible_distance(caret_line, line)
            };
            buf.clear();
            write_padded(&mut buf, value, digits, pad);
            editor.set_line_gutter_text(line, ours, &buf);
            let color = number_color(&editor, line, native, default_color);
            editor.set_line_gutter_item_color(line, ours, color);
        }
    }

    fn connect_signals(&mut self, mut editor: Gd<CodeEdit>) {
        for (signal, method) in Self::editor_signals() {
            let callable = self.base().callable(method);
            if !editor.is_connected(signal, &callable) {
                editor.connect(signal, &callable);
            }
        }
        if let Some(mut scroll) = editor.get_v_scroll_bar() {
            let callable = self.base().callable("on_scroll_changed");
            if !scroll.is_connected(SIG_VALUE_CHANGED, &callable) {
                scroll.connect(SIG_VALUE_CHANGED, &callable);
            }
        }
        if let Some(mut settings) = editor_settings() {
            let callable = self.base().callable("on_settings_changed");
            if !settings.is_connected(SIG_SETTINGS_CHANGED, &callable) {
                settings.connect(SIG_SETTINGS_CHANGED, &callable);
            }
        }
    }

    fn disconnect_signals(&mut self, mut editor: Gd<CodeEdit>) {
        for (signal, method) in Self::editor_signals() {
            let callable = self.base().callable(method);
            if editor.is_connected(signal, &callable) {
                editor.disconnect(signal, &callable);
            }
        }
        if let Some(mut scroll) = editor.get_v_scroll_bar() {
            let callable = self.base().callable("on_scroll_changed");
            if scroll.is_connected(SIG_VALUE_CHANGED, &callable) {
                scroll.disconnect(SIG_VALUE_CHANGED, &callable);
            }
        }
        if let Some(mut settings) = editor_settings() {
            let callable = self.base().callable("on_settings_changed");
            if settings.is_connected(SIG_SETTINGS_CHANGED, &callable) {
                settings.disconnect(SIG_SETTINGS_CHANGED, &callable);
            }
        }
    }

    fn editor_signals() -> [(&'static str, &'static str); 6] {
        [
            (SIG_CARET_CHANGED, "on_caret_changed"),
            (SIG_TEXT_CHANGED, "on_text_changed"),
            (SIG_THEME_CHANGED, "on_theme_changed"),
            (SIG_VISIBILITY_CHANGED, "on_visibility_changed"),
            (SIG_FOLD_UPDATED, "on_fold_updated"),
            (SIG_DRAW, "on_draw"),
        ]
    }
}

fn editor_settings() -> Option<Gd<EditorSettings>> {
    EditorInterface::singleton().get_editor_settings()
}

#[derive(Debug, PartialEq, Eq)]
struct Layout {
    hide_native: bool,
    custom: bool,
}

/// With Godot's numbers off there is no column to hide or replace.
fn layout(mode: Option<LineNumberMode>, native_shown: bool) -> Layout {
    let (hide_native, custom) = match mode {
        Some(LineNumberMode::None) if native_shown => (true, false),
        Some(LineNumberMode::Relative | LineNumberMode::Hybrid) if native_shown => (true, true),
        _ => (false, false),
    };
    Layout {
        hide_native,
        custom,
    }
}

/// Godot's own choice for the number gutter. Script, shader and text tabs
/// take it from Editor Settings; once we have hidden the gutter its flag no
/// longer tells, so read the key there. Other CodeEdits keep their flag.
fn native_intent(editor: &Gd<CodeEdit>, hid_native: bool, previous: bool) -> bool {
    if is_managed_by_code_text_editor(editor) {
        if let Some(shown) =
            editor_settings().and_then(|s| s.get_setting(SHOW_LINE_NUMBERS).try_to::<bool>().ok())
        {
            return shown;
        }
    }
    let drawn = editor.is_draw_line_numbers_enabled();
    if hid_native {
        previous || drawn
    } else {
        drawn
    }
}

fn find_gutter(editor: &Gd<CodeEdit>, name: &str) -> Option<i32> {
    (0..editor.get_gutter_count()).find(|&i| editor.get_gutter_name(i) == name.into())
}

/// `TextEdit::Text::remove_gutter` checks the gutter index against the line
/// count (text_edit.cpp@4.7:582; issue #21). On a shorter file it drops the
/// gutter but keeps its per-line slots, which shifts every later gutter's
/// cells, Godot's fold gutter among them.
fn can_remove_gutter(line_count: i32, index: i32) -> bool {
    index < line_count
}

/// Script, shader and text tabs get their gutter settings from
/// CodeTextEditor; other CodeEdits keep what they had at attach.
fn is_managed_by_code_text_editor(editor: &Gd<CodeEdit>) -> bool {
    let mut node = editor.get_parent();
    for _ in 0..4 {
        let Some(n) = node else {
            return false;
        };
        if n.is_class("CodeTextEditor") {
            return true;
        }
        node = n.get_parent();
    }
    false
}

fn visible_range(editor: &Gd<CodeEdit>) -> (i32, i32) {
    let first = editor.get_first_visible_line();
    let last =
        (editor.get_last_full_visible_line() + VISIBLE_LINE_BUFFER).min(editor.get_line_count());
    (first, last)
}

/// Godot's own cell colour: white means unset, drawn in the theme colour
/// (code_edit.cpp@4.7:1752).
fn number_color(editor: &Gd<CodeEdit>, line: i32, native: i32, default_color: Color) -> Color {
    let color = editor.get_line_gutter_item_color(line, native);
    if color == Color::from_rgb(1.0, 1.0, 1.0) {
        default_color
    } else {
        color
    }
}

fn digit_count(line_count: i32) -> i32 {
    let mut n = line_count.max(1);
    let mut digits = 1;
    while n >= 10 {
        n /= 10;
        digits += 1;
    }
    digits
}

fn write_padded(buf: &mut String, value: i32, digits: usize, pad: char) {
    let text = value.to_string();
    for _ in text.len()..digits {
        buf.push(pad);
    }
    buf.push_str(&text);
}

/// Hidden line ranges from closed folds, so a relative number counts a
/// closed fold as one line, as `j`/`k` do through the fold provider.
#[derive(Debug, Default, PartialEq)]
struct FoldMap {
    /// Sorted, merged, inclusive.
    hidden: Vec<(i32, i32)>,
}

impl FoldMap {
    fn from_editor(editor: &Gd<CodeEdit>) -> Self {
        let line_count = editor.get_line_count();
        let mut ranges = Vec::new();
        for start in editor.get_folded_lines().iter_shared() {
            let Ok(fold) = i32::try_from(start) else {
                continue;
            };
            if fold + 1 >= line_count {
                continue;
            }
            // Lines walked from fold+1 up to and including the next visible one.
            let walked = editor.get_next_visible_line_offset_from(fold + 1, 1);
            let next = fold + walked;
            let end = if next < line_count && editor.get_visible_line_count_in_range(next, next) > 0
            {
                next - 1
            } else {
                line_count - 1
            };
            if end > fold {
                ranges.push((fold + 1, end));
            }
        }
        Self::from_ranges(ranges)
    }

    fn from_ranges(mut ranges: Vec<(i32, i32)>) -> Self {
        ranges.sort_unstable();
        let mut hidden: Vec<(i32, i32)> = Vec::with_capacity(ranges.len());
        for (s, e) in ranges {
            match hidden.last_mut() {
                Some(last) if s <= last.1 + 1 => last.1 = last.1.max(e),
                _ => hidden.push((s, e)),
            }
        }
        Self { hidden }
    }

    fn visible_distance(&self, from: i32, to: i32) -> i32 {
        let (lo, hi) = if from <= to { (from, to) } else { (to, from) };
        let hidden: i32 = self
            .hidden
            .iter()
            .map(|&(s, e)| (e.min(hi) - s.max(lo + 1) + 1).max(0))
            .sum();
        hi - lo - hidden
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn digits_follow_line_count() {
        assert_eq!(digit_count(0), 1);
        assert_eq!(digit_count(9), 1);
        assert_eq!(digit_count(10), 2);
        assert_eq!(digit_count(1000), 4);
    }

    #[test]
    fn padding_matches_godot_lpad() {
        let mut buf = String::new();
        write_padded(&mut buf, 7, 3, ' ');
        assert_eq!(buf, "  7");
        buf.clear();
        write_padded(&mut buf, 7, 3, '0');
        assert_eq!(buf, "007");
        buf.clear();
        write_padded(&mut buf, 1234, 3, ' ');
        assert_eq!(buf, "1234");
    }

    #[test]
    fn layout_follows_mode_while_godot_shows_numbers() {
        use LineNumberMode as M;
        // (mode, Godot shows numbers, hide Godot's, draw ours)
        let table = [
            (None, true, false, false),
            (Some(M::Absolute), true, false, false),
            (Some(M::None), true, true, false),
            (Some(M::Relative), true, true, true),
            (Some(M::Hybrid), true, true, true),
            (None, false, false, false),
            (Some(M::Absolute), false, false, false),
            (Some(M::None), false, false, false),
            (Some(M::Relative), false, false, false),
            (Some(M::Hybrid), false, false, false),
        ];
        for (mode, shown, hide_native, custom) in table {
            assert_eq!(
                layout(mode, shown),
                Layout {
                    hide_native,
                    custom
                },
                "{mode:?} shown={shown}"
            );
        }
    }

    #[test]
    fn a_gutter_past_the_line_count_is_never_removed() {
        // Ours sits at index 3 in a script tab: a 2-line file must keep it
        // (hidden) or Godot's fold gutter cells shift by one.
        assert!(!can_remove_gutter(2, 3));
        assert!(!can_remove_gutter(3, 3));
        assert!(can_remove_gutter(4, 3));
    }

    #[test]
    fn distance_without_folds_is_line_difference() {
        let map = FoldMap::default();
        assert_eq!(map.visible_distance(10, 15), 5);
        assert_eq!(map.visible_distance(15, 10), 5);
        assert_eq!(map.visible_distance(4, 4), 0);
    }

    #[test]
    fn a_closed_fold_counts_as_one_line() {
        // Fold header on line 5 hides 6..=9.
        let map = FoldMap::from_ranges(vec![(6, 9)]);
        assert_eq!(map.visible_distance(3, 5), 2);
        assert_eq!(map.visible_distance(3, 10), 3);
        assert_eq!(map.visible_distance(10, 3), 3);
        assert_eq!(map.visible_distance(5, 12), 3);
    }

    #[test]
    fn nested_and_adjacent_ranges_merge() {
        let map = FoldMap::from_ranges(vec![(6, 20), (8, 10), (21, 25)]);
        assert_eq!(map.hidden, vec![(6, 25)]);
        assert_eq!(map.visible_distance(5, 26), 1);
    }
}
