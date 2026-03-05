use super::CmdLineHandler;
use crate::bridge::godot::names::{control, text_edit};
use crate::bridge::vim_adapter::core::cast::i32_to_usize;
use crate::bridge::vim_adapter::core::column_codec;
use crate::bridge::vim_adapter::handlers::mode::ModeHandler;
use crate::bridge::vim_adapter::managers::preview::parse_substitute_command;
use crate::bridge::vim_wrapper::VimController;
use godot::prelude::*;
use vim_core::inputs::{KeyCode, VimKey, VimModifiers};
use vim_core::state::mode::{CmdType, Mode};

#[derive(Default)]
struct CmdSubmitOutcome {
    restore_visual_mode: Option<Mode>,
    search_target: Option<(i32, i32)>,
    extend_visual: bool,
}

impl CmdLineHandler for VimController {
    fn handle_cmd_submitted(&mut self, text: &str) {
        log::debug!("Command submitted: {}", text);

        record_cmdline_for_macro(self, text);
        self.engine.push_cmd_history(text);
        self.engine.reset_history_nav();

        let command = normalize_submitted_command(text);
        if command.is_empty() {
            restore_normal_mode_with_focus(self);
            return;
        }

        let previous_visual_mode = self.engine.take_pending_visual_mode();
        let outcome = execute_cmdline_mode_command(self, command, previous_visual_mode);
        restore_mode_after_submit(self, &outcome);
        apply_focus_and_search_target(self, &outcome);
        flush_post_submit_notifications(self);
    }
}

fn record_cmdline_for_macro(controller: &mut VimController, text: &str) {
    if controller.engine.recording_register().is_none() {
        return;
    }

    for c in text.chars().skip(1) {
        controller
            .engine
            .record_macro_key(VimKey::new(KeyCode::Char(c), VimModifiers::NONE));
    }
    controller
        .engine
        .record_macro_key(VimKey::new(KeyCode::Enter, VimModifiers::NONE));
    log::debug!(
        "Recorded cmdline to macro: {} chars + Enter",
        text.len().saturating_sub(1)
    );
}

fn normalize_submitted_command(text: &str) -> &str {
    text.strip_prefix(':')
        .or_else(|| text.strip_prefix('/'))
        .or_else(|| text.strip_prefix('?'))
        .unwrap_or(text)
}

fn execute_cmdline_mode_command(
    controller: &mut VimController,
    command: &str,
    previous_visual_mode: Option<Mode>,
) -> CmdSubmitOutcome {
    let mut outcome = CmdSubmitOutcome {
        restore_visual_mode: previous_visual_mode,
        ..CmdSubmitOutcome::default()
    };

    match controller.engine.mode() {
        Mode::CmdLine(CmdType::Ex | CmdType::ExVisualRange) => {
            if parse_substitute_command(command).is_some() {
                // Preview already applied substitute edits; keep edits and clear preview state.
                controller.clear_substitute_preview_state();
            }

            if controller.get_editor().is_some() {
                let prev_mode = controller.engine.mode();
                controller.execute_ex_command_with_visuals(command, prev_mode);

                if let Some(mut editor) = controller.get_editor() {
                    editor.set_search_text(&GString::new());
                    editor.queue_redraw();
                }
            }
        }
        Mode::CmdLine(CmdType::SearchForward) => {
            outcome.search_target = controller.find_search_target(command, true);
            outcome.extend_visual =
                outcome.restore_visual_mode.is_some() && outcome.search_target.is_some();
        }
        Mode::CmdLine(CmdType::SearchBackward) => {
            outcome.search_target = controller.find_search_target(command, false);
            outcome.extend_visual =
                outcome.restore_visual_mode.is_some() && outcome.search_target.is_some();
        }
        _ => {}
    }

    outcome
}

fn restore_mode_after_submit(controller: &mut VimController, outcome: &CmdSubmitOutcome) {
    if outcome.extend_visual {
        if let Some(prev_mode) = outcome.restore_visual_mode {
            controller.engine.set_mode(prev_mode);
            controller.handle_mode_change(prev_mode, None);
            return;
        }
    }

    controller.engine.set_mode(Mode::Normal);
    controller.handle_mode_change(Mode::Normal, None);
}

fn apply_focus_and_search_target(controller: &mut VimController, outcome: &CmdSubmitOutcome) {
    if let Some(mut editor) = controller.get_editor() {
        let mut control = editor.clone().upcast::<godot::classes::Control>();
        control.call_deferred(control::methods::GRAB_FOCUS, &[]);
        log::debug!("Deferring grab_focus for editor after command submit");

        if let Some((line, col)) = outcome.search_target {
            let line_usize = i32_to_usize(line);
            let byte_col =
                column_codec::editor_col_to_byte_in_editor(&editor, line_usize, i32_to_usize(col));
            controller
                .engine
                .sync_cursor(vim_core::domain::position::Position::from_byte(
                    line_usize, byte_col,
                ));

            editor.call_deferred(text_edit::methods::SET_CARET_LINE, &[line.to_variant()]);
            editor.call_deferred(text_edit::methods::SET_CARET_COLUMN, &[col.to_variant()]);
            log::debug!("Cursor deferred to line={} col={} after focus", line, col);

            if outcome.extend_visual {
                log::debug!(
                    "Extending visual selection to search target line={} col={}",
                    line,
                    col
                );
            }
        }
    }
}

fn restore_normal_mode_with_focus(controller: &mut VimController) {
    controller.engine.set_mode(Mode::Normal);
    controller.handle_mode_change(Mode::Normal, None);
    if let Some(editor) = controller.get_editor() {
        let mut control = editor.clone().upcast::<godot::classes::Control>();
        control.grab_focus();
    }
}

fn flush_post_submit_notifications(controller: &mut VimController) {
    controller.flush_pending_message();
}
