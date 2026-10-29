//! The native caret's colour: transparent while VimCursor draws the only
//! caret, the editor's own colour otherwise.
//!
//! Godot treats every `caret_color` override write as a theme change, which
//! snaps the view to a whole row, so the colour is written only when it
//! differs from the last one written.

use godot::classes::CodeEdit;
use godot::prelude::*;

const HIDDEN: Color = Color::from_rgba(0.0, 0.0, 0.0, 0.0);

/// The editor whose `caret_color` override is read and written.
pub(super) trait CaretColorTarget {
    fn caret_color_override(&self) -> Option<Color>;
    /// `None` removes the override.
    fn set_caret_color_override(&mut self, color: Option<Color>);
}

impl CaretColorTarget for Gd<CodeEdit> {
    fn caret_color_override(&self) -> Option<Color> {
        self.has_theme_color_override("caret_color")
            .then(|| self.get_theme_color("caret_color"))
    }

    fn set_caret_color_override(&mut self, color: Option<Color>) {
        match color {
            Some(color) => self.add_theme_color_override("caret_color", color),
            None => self.remove_theme_color_override("caret_color"),
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Caret {
    Hidden,
    Native,
}

#[derive(Default)]
pub(super) struct NativeCaretColor {
    /// The editor's override before attach; `None` when theme-inherited.
    saved: Option<Color>,
    /// The last colour written; `None` forces the next write.
    written: Option<Caret>,
}

impl NativeCaretColor {
    pub(super) fn attach(&mut self, editor: &mut impl CaretColorTarget) {
        self.saved = editor.caret_color_override();
        self.written = None;
        self.write(editor, Caret::Hidden);
    }

    pub(super) fn detach(&mut self, editor: &mut impl CaretColorTarget) {
        editor.set_caret_color_override(self.saved.take());
        self.written = None;
    }

    /// Several carets show the native ones (VimCursor covers the primary).
    pub(super) fn update(
        &mut self,
        editor: &mut impl CaretColorTarget,
        cursor_count: usize,
        vim_cursor: bool,
    ) {
        let caret = if cursor_count > 1 || !vim_cursor {
            Caret::Native
        } else {
            Caret::Hidden
        };
        self.write(editor, caret);
    }

    /// The cursor setting changed: write even if the last write matches.
    pub(super) fn set_vim_cursor(&mut self, editor: &mut impl CaretColorTarget, enabled: bool) {
        self.written = None;
        let caret = if enabled {
            Caret::Hidden
        } else {
            Caret::Native
        };
        self.write(editor, caret);
    }

    fn write(&mut self, editor: &mut impl CaretColorTarget, caret: Caret) {
        if self.written == Some(caret) {
            return;
        }
        editor.set_caret_color_override(match caret {
            Caret::Hidden => Some(HIDDEN),
            Caret::Native => self.saved,
        });
        self.written = Some(caret);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RED: Color = Color::from_rgba(1.0, 0.0, 0.0, 1.0);

    #[derive(Default)]
    struct FakeEditor {
        colour: Option<Color>,
        writes: usize,
    }

    impl CaretColorTarget for FakeEditor {
        fn caret_color_override(&self) -> Option<Color> {
            self.colour
        }

        fn set_caret_color_override(&mut self, color: Option<Color>) {
            self.colour = color;
            self.writes += 1;
        }
    }

    fn attached(saved: Option<Color>) -> (NativeCaretColor, FakeEditor) {
        let mut editor = FakeEditor {
            colour: saved,
            writes: 0,
        };
        let mut caret = NativeCaretColor::default();
        caret.attach(&mut editor);
        assert_eq!((editor.colour, editor.writes), (Some(HIDDEN), 1));
        (caret, editor)
    }

    #[test]
    fn an_update_with_the_same_state_writes_nothing() {
        let (mut caret, mut editor) = attached(None);
        for _ in 0..3 {
            caret.update(&mut editor, 1, true);
        }
        assert_eq!(editor.writes, 1);
        caret.update(&mut editor, 2, true);
        caret.update(&mut editor, 2, true);
        assert_eq!(editor.writes, 2);
        caret.set_vim_cursor(&mut editor, false);
        caret.update(&mut editor, 1, false);
        caret.update(&mut editor, 1, false);
        assert_eq!(editor.writes, 3);
    }

    #[test]
    fn detach_restores_the_saved_colour() {
        let (mut caret, mut editor) = attached(Some(RED));
        caret.detach(&mut editor);
        assert_eq!(editor.colour, Some(RED));
        let (mut caret, mut editor) = attached(None);
        caret.detach(&mut editor);
        assert_eq!(editor.colour, None);
    }

    #[test]
    fn attach_writes_after_a_detach() {
        let (mut caret, mut editor) = attached(Some(RED));
        caret.detach(&mut editor);
        let mut other = FakeEditor::default();
        caret.attach(&mut other);
        assert_eq!((other.colour, other.writes), (Some(HIDDEN), 1));
        caret.update(&mut other, 2, true);
        assert_eq!(other.colour, None, "the saved colour is the new editor's");
    }

    #[test]
    fn the_cursor_count_shows_and_hides_the_native_caret() {
        let (mut caret, mut editor) = attached(Some(RED));
        caret.update(&mut editor, 3, true);
        assert_eq!(editor.colour, Some(RED));
        caret.update(&mut editor, 1, true);
        assert_eq!(editor.colour, Some(HIDDEN));
        caret.update(&mut editor, 2, false);
        assert_eq!(editor.colour, Some(RED));
        caret.update(&mut editor, 1, false);
        assert_eq!((editor.colour, editor.writes), (Some(RED), 4));
    }

    #[test]
    fn the_cursor_toggle_writes_even_when_the_colour_matches() {
        let (mut caret, mut editor) = attached(Some(RED));
        caret.set_vim_cursor(&mut editor, false);
        assert_eq!((editor.colour, editor.writes), (Some(RED), 2));
        // Something else hid the caret; the toggle still shows it.
        editor.colour = Some(HIDDEN);
        caret.set_vim_cursor(&mut editor, false);
        assert_eq!((editor.colour, editor.writes), (Some(RED), 3));
        caret.set_vim_cursor(&mut editor, true);
        caret.update(&mut editor, 1, true);
        assert_eq!((editor.colour, editor.writes), (Some(HIDDEN), 4));
    }
}
