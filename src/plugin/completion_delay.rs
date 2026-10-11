//! Code Complete Delay for popups opened by typing.
//!
//! Godot opens the popup from its own timer, which starts only for a typing
//! operation (code_editor.cpp:1010-1014) and so never fires for text the
//! engine writes. This is that timer: armed by `effects::completion` on every
//! insertion, under the rules in [`DelayArm::still_wanted`], and stopped by
//! Vim's own popup keys (`controller::process`), since Ctrl+E closing the
//! popup must not see it come back.

use godot::classes::{EditorInterface, Timer};
use godot::prelude::*;

use super::signals::{connect_immediate, SIG_TIMEOUT};
use super::GodotVimCore;
use crate::effects::completion::{DelayArm, DelayFacts};

impl GodotVimCore {
    pub(super) fn init_completion_timer(&mut self) {
        let mut timer = Timer::new_alloc();
        timer.set_one_shot(true);
        let callable = self.base().callable("on_completion_timeout");
        connect_immediate(&mut timer, SIG_TIMEOUT, &callable);
        self.base_mut().add_child(&timer.clone().upcast::<Node>());
        self.completion_timer = Some(timer);
    }

    /// Symmetric teardown for `init_completion_timer`.
    pub(super) fn teardown_completion_timer(&mut self) {
        self.completion_arm = None;
        if let Some(mut timer) = self.completion_timer.take() {
            timer.stop();
            timer.queue_free();
        }
    }

    /// Restart the delay for the caret on `line`, reading the delay each
    /// time so a changed setting applies at once.
    pub(super) fn arm_completion_delay(&mut self, line: i32) {
        let Some(editor) = self.attached_editor.as_ref() else {
            return;
        };
        self.completion_arm = Some(DelayArm {
            editor: editor.instance_id().to_i64(),
            line,
        });
        let delay = EditorInterface::singleton()
            .get_editor_settings()
            .map_or(crate::settings::defaults::CODE_COMPLETE_DELAY, |s| {
                crate::settings::reader::code_complete_delay(&s)
            });
        if let Some(timer) = &mut self.completion_timer {
            timer.set_wait_time(delay);
            timer.start();
        }
    }

    /// Run after every engine entry: Esc, a mapping that leaves Insert or a
    /// caret on another line stops the delay at once rather than at timeout,
    /// so Esc then `a` inside the delay opens nothing.
    pub(super) fn recheck_completion_delay(&mut self) {
        if let Some(arm) = self.completion_arm {
            if !arm.still_wanted(self.completion_delay_facts()) {
                log::debug!("completion_delay: stopped, the caret left the line or Insert");
                self.disarm_completion_delay();
            }
        }
    }

    pub(super) fn disarm_completion_delay(&mut self) {
        self.completion_arm = None;
        if let Some(timer) = &mut self.completion_timer {
            timer.stop();
        }
    }

    /// Godot's `_code_complete_timer_timeout` (code_editor.cpp:1023-1028): an
    /// unforced request on the raw editor, popup open or not, so Godot's own
    /// symbol rule still decides. Checked again here, since a click moves the
    /// caret without a key.
    pub(super) fn on_completion_timeout_impl(&mut self) {
        let Some(arm) = self.completion_arm.take() else {
            return;
        };
        if !arm.still_wanted(self.completion_delay_facts()) {
            return;
        }
        let Some(mut editor) = self.attached_editor.clone() else {
            return;
        };
        if editor.is_visible_in_tree() && !editor.has_ime_text() {
            log::debug!("completion_delay: fired, asking Godot for the popup");
            editor.request_code_completion();
        }
    }

    fn completion_delay_facts(&self) -> DelayFacts {
        let editor = self
            .attached_editor
            .as_ref()
            .filter(|editor| editor.is_instance_valid());
        DelayFacts {
            editor: editor.map(|editor| editor.instance_id().to_i64()),
            line: editor.map_or(-1, |editor| editor.get_caret_line()),
            insert_like: self.controller.as_ref().is_some_and(|controller| {
                let mode = controller.mode();
                mode.is_insert() || mode.is_replace()
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    /// The body of method `name` in `src`, up to its closing brace.
    fn method<'a>(src: &'a str, name: &str) -> &'a str {
        let start = src
            .find(&format!("fn {name}("))
            .unwrap_or_else(|| panic!("input.rs lost `fn {name}`"));
        let body = &src[start..];
        &body[..body.find("\n    }\n").expect("the method's closing brace")]
    }

    /// Source-level, as in `signals.rs`: both engine entries take `Gd` state
    /// and cannot run under `cargo test`.
    #[test]
    fn every_engine_entry_rechecks_the_delay() {
        let src = include_str!("input.rs");
        let production = src.split_once("\n#[cfg(test)]").map_or(src, |(h, _)| h);
        for name in ["handle_gui_input_impl", "on_mapping_timeout_impl"] {
            assert!(
                method(production, name).contains("self.recheck_completion_delay();"),
                "`{name}` must recheck the delay, or Esc then `a` inside the delay \
                 still opens the popup"
            );
        }
    }
}
