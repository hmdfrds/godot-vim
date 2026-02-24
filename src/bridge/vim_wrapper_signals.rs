//! Signal handler implementations for VimController.
//!
//! Extracted from `vim_wrapper.rs`. These are the bodies of signal callbacks.
//! The `#[func]` stubs remain in `vim_wrapper.rs` and delegate here.

use crate::bridge::vim_adapter::controller::SignalHandlersTrait;
use crate::bridge::vim_adapter::core::cast::i32_to_usize;
use crate::bridge::vim_wrapper::VimController;

use godot::classes::InputEvent;
use godot::prelude::*;

impl VimController {
    pub(crate) fn on_mapping_timeout_impl(&mut self) {
        crate::bridge::safety::guard(
            || {
                log::trace!("Mapping timeout fired");

                if !self.input.mapping_state.has_pending() {
                    log::trace!("No pending keys to replay");
                    return;
                }

                let pending = self.input.mapping_state.flush();
                log::trace!("Replaying {} pending keys", pending.len());

                for (i, key) in pending.iter().enumerate() {
                    let allow_mapping = i != 0;
                    log::debug!(
                        "Replaying key index={} key={:?} allow_mapping={}",
                        i,
                        key,
                        allow_mapping
                    );
                    self.process_vim_key_internal(key, allow_mapping, false);
                }
                self.drain_macro_call_stack();
            },
            (),
        );
    }

    pub(crate) fn on_scrollbar_changed_impl(&mut self, _value: f64) {
        crate::bridge::safety::guard(
            || {
                self.on_cursor_visual_update_impl();
            },
            (),
        );
    }

    pub(crate) fn on_cursor_visual_update_impl(&mut self) {
        crate::bridge::safety::guard(
            || {
                self.update_cursor_visual();
            },
            (),
        );
    }

    pub(crate) fn on_caret_moved_impl(&mut self) {
        crate::bridge::safety::guard(
            || {
                if let Some(editor) = self.get_editor() {
                    let line = i32_to_usize(editor.get_caret_line());

                    // Snapshot the line on first visit to support undo-line (U).
                    let should_snapshot = match self.engine.current_line_snapshot() {
                        None => true,
                        Some((saved_line, _)) => *saved_line != line,
                    };

                    if should_snapshot {
                        let text = editor
                            .get_line(crate::bridge::vim_adapter::core::cast::usize_to_i32(line))
                            .to_string();
                        self.engine.update_line_snapshot(line, text);
                    }

                    // Synchronize vim-state cursor when caret moves via mouse click.
                    let col = i32_to_usize(editor.get_caret_column());
                    let cur = self.engine.cursor_pos();
                    if cur.line != line || cur.col != col {
                        self.engine.set_cursor(line, col);
                        self.engine.set_preferred_column(col);
                    }
                }
            },
            (),
        );
    }

    pub(crate) fn handle_gui_input_impl(&mut self, event: Gd<InputEvent>) {
        crate::bridge::safety::guard(
            || {
                let Some(vim_key) = self.parse_and_filter(&event) else {
                    return;
                };

                self.trace_key_pipeline(&vim_key);
                self.record_macro_key(&vim_key);

                if self.try_window_nav(&vim_key) {
                    return;
                }

                if self.try_process_mapping(&vim_key, true) {
                    self.set_input_handled();
                    self.drain_macro_call_stack();
                    return;
                }

                self.dispatch_by_priority(&vim_key);
                self.drain_macro_call_stack();
            },
            (),
        );
    }

    /// Drain the macro call stack, replaying all pending keys iteratively.
    ///
    /// Called once per user key press after the primary dispatch. New frames
    /// pushed by nested macros during draining are picked up by the next loop
    /// iteration — no Rust call-stack recursion occurs.
    fn drain_macro_call_stack(&mut self) {
        if !self.engine.has_pending_macro_keys() {
            return;
        }

        if let Some(mut editor) = self.get_editor() {
            editor.begin_complex_operation();
        }

        while let Some(key) = self.engine.pop_macro_key() {
            self.process_vim_key_internal(&key, true, false);
        }

        if let Some(mut editor) = self.get_editor() {
            editor.end_complex_operation();
            self.engine.sync_cursor(Self::cursor_from_editor(&editor));
            let mut control = editor.clone().upcast::<godot::classes::Control>();
            control.grab_focus();
        }
    }
}
