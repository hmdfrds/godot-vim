//! Keys the Android editor does not deliver the way a desktop does.

use godot::classes::{Engine, InputEvent, InputEventKey, Window};
use godot::global::Key;
use godot::prelude::*;

use super::signals::{connect_immediate, safe_disconnect};
use super::GodotVimCore;

const SIG_WINDOW_INPUT: &str = "window_input";
const SIG_GO_BACK_REQUESTED: &str = "go_back_requested";

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

/// Tells gesture back from the Back key, so one Back gives one Esc.
///
/// A Back key press sends a key event and then, deferred, a go-back request
/// (`android_input_handler.cpp`, `process_key_event`); gesture and predictive
/// back send only the request (`GodotLib.back()`). While the on-screen
/// keyboard's text view holds the input connection, the Back key's request
/// also comes from the activity a few milliseconds before the key event.
#[derive(Debug, Default)]
pub(super) struct BackKey {
    /// Process frame of the last Back key press.
    pressed_at: Option<u64>,
    /// Process frame of the last request taken as gesture back.
    gesture_at: Option<u64>,
    /// The Back key press of that request, by event instance id.
    paired: Option<i64>,
}

impl BackKey {
    /// A key press and its request arrive within two frames of each other.
    const REQUEST_LAG: u64 = 2;

    pub(super) fn observe_press(&mut self, event: i64, frame: u64) {
        self.pressed_at = Some(frame);
        self.paired = self
            .gesture_at
            .take()
            .filter(|&at| frame <= at.saturating_add(Self::REQUEST_LAG))
            .map(|_| event);
    }

    pub(super) fn request_is_gesture(&self, frame: u64) -> bool {
        self.pressed_at
            .is_none_or(|at| frame > at.saturating_add(Self::REQUEST_LAG))
    }

    pub(super) fn observe_gesture(&mut self, frame: u64) {
        self.gesture_at = Some(frame);
    }

    /// The press that follows a request already taken as gesture back.
    pub(super) fn is_paired(&self, event: i64) -> bool {
        self.paired == Some(event)
    }
}

/// A plain Esc press, in place of `like` when there is a key to replace.
fn esc_press(like: Option<&Gd<InputEventKey>>) -> Gd<InputEventKey> {
    let mut esc = InputEventKey::new_gd();
    esc.set_keycode(Key::ESCAPE);
    esc.set_physical_keycode(Key::ESCAPE);
    esc.set_pressed(true);
    if let Some(like) = like {
        esc.set_echo(like.is_echo());
        esc.set_device(like.get_device());
        esc.set_window_id(like.get_window_id());
    }
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
            log::warn!("android: no root window, Esc releases and gesture back are not watched");
            return;
        };
        let callable = self.base().callable("on_window_input");
        connect_immediate(&mut root, SIG_WINDOW_INPUT, &callable);
        let callable = self.base().callable("on_go_back_requested");
        connect_immediate(&mut root, SIG_GO_BACK_REQUESTED, &callable);
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
        let callable = self.base().callable("on_go_back_requested");
        safe_disconnect(&mut root, SIG_GO_BACK_REQUESTED, &callable);
    }

    fn back_as_escape(&self) -> bool {
        self.settings
            .as_ref()
            .map_or(crate::settings::defaults::BACK_AS_ESCAPE, |s| {
                s.back_as_escape
            })
    }

    pub(super) fn on_window_input_impl(&mut self, event: Gd<InputEvent>) {
        let Ok(key) = event.try_cast::<InputEventKey>() else {
            return;
        };
        match key.get_keycode() {
            Key::ESCAPE => self
                .esc_release
                .observe(key.instance_id().to_i64(), key.is_pressed()),
            Key::BACK if key.is_pressed() => self.back_key.observe_press(
                key.instance_id().to_i64(),
                Engine::singleton().get_process_frames(),
            ),
            _ => {}
        }
    }

    /// An Esc press for a gesture back, or `None` for the request a Back key
    /// press sends after its key event, which `android_key` has already taken.
    pub(super) fn gesture_back_key(&mut self) -> Option<Gd<InputEventKey>> {
        if !self.back_as_escape() {
            return None;
        }
        let frame = Engine::singleton().get_process_frames();
        if !self.back_key.request_is_gesture(frame) {
            log::trace!("android: go-back request of the Back key, already taken");
            return None;
        }
        // A dialog or menu over the editor is not Vim's to answer.
        if self
            .root_window()
            .is_some_and(|root| !root.get_embedded_subwindows().is_empty())
        {
            log::debug!("android: gesture back with a window open, left alone");
            return None;
        }
        // Only an Esc that lands may make the Back key that follows a no-op.
        if !self
            .attached_editor
            .as_ref()
            .is_some_and(|e| e.is_instance_valid() && e.has_focus())
        {
            return None;
        }
        log::debug!("android: gesture back, taken as Esc");
        self.back_key.observe_gesture(frame);
        Some(esc_press(None))
    }

    /// The key godot-vim acts on in place of `event`, on either input plane,
    /// or `None` for a Back key press whose Esc a gesture back already gave:
    /// the caller consumes it so the script editor's history stays put.
    pub(super) fn android_key(&self, event: Gd<InputEventKey>) -> Option<Gd<InputEventKey>> {
        if !crate::platform::is_android() {
            return Some(event);
        }
        if event.get_keycode() == Key::ESCAPE
            && !event.is_pressed()
            && self.esc_release.acts_as_press(event.instance_id().to_i64())
        {
            log::debug!("android: Esc release with no press, taken as Esc");
            return Some(esc_press(Some(&event)));
        }
        if event.get_keycode() == Key::BACK && event.is_pressed() && self.back_as_escape() {
            if self.back_key.is_paired(event.instance_id().to_i64()) {
                log::debug!("android: Back key of a gesture back already taken, dropped");
                return None;
            }
            log::debug!("android: Back key, taken as Esc");
            return Some(esc_press(Some(&event)));
        }
        Some(event)
    }
}

#[cfg(test)]
mod tests {
    use super::{BackKey, EscRelease};

    #[test]
    fn a_lone_gesture_pairs_with_no_later_key() {
        let mut back = BackKey::default();
        assert!(back.request_is_gesture(100));
        back.observe_gesture(100);
        back.observe_press(7, 103);
        assert!(!back.is_paired(7));
    }

    #[test]
    fn a_request_after_its_key_is_not_a_gesture() {
        let mut back = BackKey::default();
        back.observe_press(7, 100);
        assert!(!back.is_paired(7));
        assert!(!back.request_is_gesture(100));
        assert!(!back.request_is_gesture(102));
        assert!(back.request_is_gesture(103));
    }

    #[test]
    fn a_key_after_its_request_gives_no_second_esc() {
        let mut back = BackKey::default();
        assert!(back.request_is_gesture(100));
        back.observe_gesture(100);
        back.observe_press(7, 101);
        assert!(back.is_paired(7));
        // The key's own deferred request is skipped as before.
        assert!(!back.request_is_gesture(101));
        // The pairing is spent: the next Back key acts again.
        back.observe_press(8, 200);
        assert!(!back.is_paired(8));
        assert!(back.request_is_gesture(203));
    }

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
