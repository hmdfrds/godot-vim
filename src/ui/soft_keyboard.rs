//! Android: the on-screen keyboard's switch on the attached CodeEdit.
//!
//! The keyboard app rebuilds keys from text diffs, a backspace and a retype
//! per key, which garbles any command, so the keyboard is off outside Insert
//! and Replace. It is off there too while a hardware keyboard is connected:
//! Godot leaves the app's text listener attached when a keyboard connects
//! after the on-screen one opened (`GodotEditText.java`, `hideKeyboard`).

use godot::classes::display_server::Feature;
use godot::classes::{CodeEdit, DisplayServer};
use godot::prelude::*;
use vim_core::primitives::Mode;

#[derive(Debug, Default)]
pub(super) struct SoftKeyboard {
    /// The CodeEdit's own value, restored on detach. `None` off Android.
    saved: Option<bool>,
    /// What was last written.
    written: Option<bool>,
}

/// Whether `mode` may have the on-screen keyboard. `hardware_keyboard` is
/// asked only in the modes that type text.
fn wanted(mode: Mode, hardware_keyboard: impl FnOnce() -> bool) -> bool {
    (mode.is_insert() || mode.is_replace()) && !hardware_keyboard()
}

impl SoftKeyboard {
    pub(super) fn attach(&mut self, editor: &Gd<CodeEdit>) {
        *self = Self::default();
        if crate::platform::is_android() {
            self.saved = Some(editor.is_virtual_keyboard_enabled());
        }
    }

    pub(super) fn update(&mut self, editor: &mut Gd<CodeEdit>, mode: Mode) {
        if self.saved.is_none() {
            return;
        }
        let wanted = wanted(mode, || DisplayServer::singleton().has_hardware_keyboard());
        let Some(enabled) = self.next(wanted) else {
            return;
        };
        editor.set_virtual_keyboard_enabled(enabled);
        if !enabled {
            // Left open, it would go on typing into Normal mode.
            let mut display = DisplayServer::singleton();
            if display.has_feature(Feature::VIRTUAL_KEYBOARD)
                && display.virtual_keyboard_get_height() > 0
            {
                display.virtual_keyboard_hide();
            }
        }
    }

    pub(super) fn detach(&mut self, editor: &mut Gd<CodeEdit>) {
        if let Some(saved) = self.saved.take() {
            editor.set_virtual_keyboard_enabled(saved);
        }
        self.written = None;
    }

    /// The value to write, or `None` when it is written already.
    fn next(&mut self, wanted: bool) -> Option<bool> {
        if self.written == Some(wanted) {
            return None;
        }
        self.written = Some(wanted);
        Some(wanted)
    }
}

#[cfg(test)]
mod tests {
    use vim_core::primitives::{Mode, Operator, VisualType};

    use super::{wanted, SoftKeyboard};

    #[test]
    fn only_the_modes_that_type_text_take_the_on_screen_keyboard() {
        for mode in [Mode::Insert, Mode::Replace, Mode::VirtualReplace] {
            assert!(wanted(mode, || false), "{mode:?}");
        }
        for mode in [
            Mode::Normal,
            Mode::Visual(VisualType::Char),
            Mode::Select(VisualType::Line),
            Mode::OperatorPending(Operator::Delete),
            Mode::CommandLine,
        ] {
            assert!(!wanted(mode, || panic!("asked outside Insert")), "{mode:?}");
        }
    }

    #[test]
    fn a_hardware_keyboard_turns_it_off_in_insert_too() {
        assert!(!wanted(Mode::Insert, || true));
        assert!(!wanted(Mode::Replace, || true));
    }

    #[test]
    fn the_switch_is_written_only_when_it_changes() {
        let mut keyboard = SoftKeyboard::default();
        assert_eq!(keyboard.next(false), Some(false));
        assert_eq!(keyboard.next(false), None);
        assert_eq!(keyboard.next(true), Some(true));
        assert_eq!(keyboard.next(true), None);
        assert_eq!(keyboard.next(false), Some(false));
    }
}
