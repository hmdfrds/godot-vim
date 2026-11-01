//! Keys the Android editor does not deliver the way a desktop does.

use godot::classes::{InputEvent, InputEventKey, Window};
use godot::global::Key;
use godot::prelude::*;

use super::signals::{connect_immediate, safe_disconnect};
use super::GodotVimCore;

const SIG_WINDOW_INPUT: &str = "window_input";

/// Esc presses and releases as the root window saw them, before any control
/// or dialog could consume one.
///
/// A keyboard app such as SwiftKey takes the Esc press of a hardware keyboard
/// and passes only the release. A release with no press since the last
/// release then stands in for the press; an app that passes the press takes
/// the normal path, so nothing fires twice.
#[derive(Debug, Default)]
pub(super) struct EscRelease {
    pressed: bool,
    /// The release that stands in for a press, by event instance id. Both
    /// input planes ask about the same event, so the answer is kept.
    orphan: Option<i64>,
}

impl EscRelease {
    pub(super) fn observe(&mut self, event: i64, pressed: bool) {
        if pressed {
            self.pressed = true;
            self.orphan = None;
        } else {
            self.orphan = (!self.pressed).then_some(event);
            self.pressed = false;
        }
    }

    pub(super) fn acts_as_press(&self, event: i64) -> bool {
        self.orphan == Some(event)
    }
}

/// A plain Esc press in place of `like`.
fn esc_press(like: &Gd<InputEventKey>) -> Gd<InputEventKey> {
    let mut esc = InputEventKey::new_gd();
    esc.set_keycode(Key::ESCAPE);
    esc.set_physical_keycode(Key::ESCAPE);
    esc.set_pressed(true);
    esc.set_echo(like.is_echo());
    esc.set_device(like.get_device());
    esc.set_window_id(like.get_window_id());
    esc
}

impl GodotVimCore {
    fn root_window(&self) -> Option<Gd<Window>> {
        self.base().get_tree_or_null()?.get_root()
    }

    pub(super) fn connect_android_signals(&mut self) {
        if !crate::platform::is_android() {
            return;
        }
        let Some(mut root) = self.root_window() else {
            log::warn!("android: no root window, Esc releases are not watched");
            return;
        };
        let callable = self.base().callable("on_window_input");
        connect_immediate(&mut root, SIG_WINDOW_INPUT, &callable);
    }

    pub(super) fn disconnect_android_signals(&mut self) {
        if !crate::platform::is_android() {
            return;
        }
        let Some(mut root) = self.root_window() else {
            return;
        };
        let callable = self.base().callable("on_window_input");
        safe_disconnect(&mut root, SIG_WINDOW_INPUT, &callable);
    }

    pub(super) fn on_window_input_impl(&mut self, event: Gd<InputEvent>) {
        let Ok(key) = event.try_cast::<InputEventKey>() else {
            return;
        };
        if key.get_keycode() == Key::ESCAPE {
            self.esc_release
                .observe(key.instance_id().to_i64(), key.is_pressed());
        }
    }

    /// The key godot-vim acts on in place of `event`, on either input plane.
    pub(super) fn android_key(&self, event: Gd<InputEventKey>) -> Gd<InputEventKey> {
        if !crate::platform::is_android() {
            return event;
        }
        if event.get_keycode() == Key::ESCAPE
            && !event.is_pressed()
            && self.esc_release.acts_as_press(event.instance_id().to_i64())
        {
            log::debug!("android: Esc release with no press, taken as Esc");
            return esc_press(&event);
        }
        event
    }
}

#[cfg(test)]
mod tests {
    use super::EscRelease;

    #[test]
    fn a_release_after_its_press_is_only_a_release() {
        let mut esc = EscRelease::default();
        esc.observe(1, true);
        esc.observe(2, false);
        assert!(!esc.acts_as_press(1));
        assert!(!esc.acts_as_press(2));
    }

    #[test]
    fn a_release_with_no_press_acts_as_the_press() {
        let mut esc = EscRelease::default();
        esc.observe(1, false);
        assert!(esc.acts_as_press(1));
        // The engine plane asks about the same event after the shell plane.
        assert!(esc.acts_as_press(1));
    }

    #[test]
    fn each_release_is_judged_by_the_presses_since_the_last_release() {
        let mut esc = EscRelease::default();
        esc.observe(1, false);
        esc.observe(2, true);
        assert!(!esc.acts_as_press(1));
        esc.observe(3, true);
        esc.observe(4, false);
        assert!(!esc.acts_as_press(4));
        esc.observe(5, false);
        assert!(esc.acts_as_press(5));
        esc.observe(6, false);
        assert!(esc.acts_as_press(6));
        assert!(!esc.acts_as_press(5));
    }
}
