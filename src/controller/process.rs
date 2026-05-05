//! Main keystroke processing pipeline: the path every key takes from Godot's
//! `gui_input` callback through `VimSession::process_key()` and back out as
//! editor mutations.
//!
//! Key flow:
//! ```text
//! gui_input -> process_cycle
//!   +- clear transient messages
//!   +- vimdebug step-mode intercept
//!   +- completion interception (pre-engine)
//!   +- passthrough check
//!   +- pre-processing: refresh_from_editor, set config
//!   +- session.process_key(key) -> ProcessResult
//!   +- post-processing: deferred actions, pending UI, undo balance
//!   +- completion re-trigger (post-engine)
//!   +- IME lifecycle
//!   +- per-keystroke debug logging
//! ```

use godot::classes::{CodeEdit, DisplayServer};
use godot::prelude::*;
use vim_core::execution::host_api::{DeferredAction, WindowNavAction};
use vim_core::keymap::KeyEvent;

use super::completion;
use super::perf;
use super::VimController;
use crate::bridge::codec::usize_to_i32;
use crate::bridge::godot_host::GodotHost;

impl VimController {
    /// Single entry point from `gui_input`. Returns `true` if Vim consumed
    /// the key (Godot should not process it), `false` to pass through.
    ///
    /// Guarantees undo group balance on return via `ensure_undo_balanced`.
    pub(crate) fn process_cycle_impl(&mut self, key: KeyEvent, editor: &mut Gd<CodeEdit>) -> bool {
        self.transient.operations_this_cycle = 0;

        // Messages are one-shot: displayed after the producing keystroke,
        // cleared on the next. Mirrors vim-core's clear_transient().
        {
            let session = self
                .session
                .as_mut()
                .expect("process_cycle: requires active session");
            session.host_mut().state_mut().globals_mut().clear_message();
        }

        // Step mode intercepts all keys for the effect inspector (n/p/c/q).
        if self.transient.vimdebug.is_step_mode() && self.transient.pending_step_effects.is_some() {
            return self.process_step_key(key, editor);
        }

        // Completion interception (pre-engine): check if completion popup handles key.
        {
            let session = self
                .session
                .as_mut()
                .expect("process_cycle: requires active session");
            if let Some(consumed) =
                completion::try_handle_completion(session.engine_mut(), key, editor)
            {
                log::debug!(
                    "process_cycle: completion intercepted key={} consumed={}",
                    key,
                    consumed
                );
                // When completion confirmed (consumed=true), the editor text changed
                // but the host's text_cache is stale. Invalidate it so the next
                // on_text_changed sees matching texts and skips double-reconciliation.
                if consumed {
                    session.host_mut().invalidate_cache();
                }
                let mode = session.engine().mode();
                session.host_mut().ensure_undo_balanced(mode);
                return consumed;
            }
        }

        // Passthrough check: does this key bypass Vim entirely?
        if self.should_passthrough_key(key) {
            log::debug!(
                "process_cycle: passthrough key={} mode={}",
                key,
                self.engine().mode()
            );
            return false;
        }

        // Capture pre-processing state for debug summary and IME lifecycle.
        let mode_before = self.engine().mode();
        let cursor_before = (editor.get_caret_line(), editor.get_caret_column());
        let total_start = std::time::Instant::now();

        // ── Pre-processing setup ────────────────────────────────────────
        {
            let session = self
                .session
                .as_mut()
                .expect("process_cycle: requires active session");
            let engine_mode = session.engine().mode();
            let auto_pairs_active = session.engine().options().auto_pairs().is_some();
            let scrolloff = usize_to_i32(session.engine().options().scrolloff());
            session.host_mut().refresh_from_editor();
            session
                .host_mut()
                .set_auto_brace_eligible(engine_mode.is_insert());
            session
                .host_mut()
                .set_engine_auto_pairs_active(auto_pairs_active);
            session.host_mut().set_scrolloff(scrolloff);
            session.host_mut().set_current_mode(engine_mode);
            session
                .host_mut()
                .set_vimdebug_enabled(self.transient.vimdebug.is_enabled());
        }

        // ── Vimdebug: capture provenance before engine process ──────────
        self.transient.vimdebug.clear_captures();

        // ── Clipboard sync: pre-populate + register for clipboard=unnamedplus ──
        {
            let session = self
                .session
                .as_mut()
                .expect("process_cycle: requires active session");
            let opts = session.engine().resolved_options();
            if opts.clipboard_has_unnamedplus() || opts.clipboard_has_unnamed() {
                let text = godot::classes::DisplayServer::singleton()
                    .clipboard_get()
                    .to_string();
                session.sync_clipboard(&text);
            }
        }

        // ── CORE: session.process_key(key) ──────────────────────────────
        let result = {
            let session = self
                .session
                .as_mut()
                .expect("process_cycle: requires active session");
            session.process_key(key)
        };
        let consumed = result.consumed;

        self.transient.operations_this_cycle =
            self.transient.operations_this_cycle.saturating_add(1);

        // ── Post-processing: deferred actions ───────────────────────────
        for action in &result.deferred_actions {
            match action {
                DeferredAction::WindowNav(nav) => {
                    if let Some(nav_action) = convert_window_nav_action(*nav) {
                        let control: Gd<godot::classes::Control> = editor.clone().upcast();
                        crate::navigation::handle_window_nav_action(&control, nav_action);
                    }
                }
                _ => {
                    log::warn!("process_cycle: unhandled deferred action {:?}", action);
                }
            }
        }

        // ── Post-processing: drain pending UI actions from host ─────────
        {
            let session = self
                .session
                .as_mut()
                .expect("process_cycle: requires active session");
            let pending_actions = session.host_mut().take_pending_ui_actions();
            for action in pending_actions {
                self.handle_host_pending_ui_action(action, editor);
            }
        }

        // ── Post-processing: ensure undo balanced ───────────────────────
        {
            let session = self
                .session
                .as_mut()
                .expect("process_cycle: requires active session");
            let mode = session.engine().mode();
            session.host_mut().ensure_undo_balanced(mode);
        }

        // ── Post-processing: completion re-trigger ──────────────────────
        {
            let session = self
                .session
                .as_mut()
                .expect("process_cycle: requires active session");
            completion::maybe_retrigger_completion(
                session.engine(),
                key,
                editor,
                self.code_complete_enabled,
            );
        }

        let total_elapsed = total_start.elapsed();

        // ── IME lifecycle ────────────────────────────────────────────────
        // Godot's TextEdit unconditionally re-enables im_active on every redraw
        // (text_edit.cpp: _update_ime_window_position → window_set_ime_active(true)).
        // Any cursor movement triggers queue_redraw(), so by the next frame im_active
        // is true again even though we set it false on mode exit.
        //
        // On macOS, im_active=true causes the keyDown handler to route through
        // interpretKeyEvents, where the Press-and-Hold accent system can suppress
        // insertText on key repeat — losing the unicode value entirely.
        //
        // Fix: re-assert deactivate_ime after every keystroke in non-insert modes,
        // not just on mode transitions. This counteracts TextEdit's re-enablement.
        // See: https://github.com/hmdfrds/godot-vim/issues/33
        let mode_after = self.engine().mode();
        let is_insert_like =
            mode_after.is_insert() || mode_after.is_replace() || mode_after.is_command_line();

        if is_insert_like {
            if mode_after != mode_before {
                let was_insert_like = mode_before.is_insert()
                    || mode_before.is_replace()
                    || mode_before.is_command_line();
                if !was_insert_like {
                    activate_ime(editor);
                }
            }
            update_ime_position(editor);
        } else {
            // Immediate deactivation — effective within this frame's input phase.
            deactivate_ime(editor);
            // Deferred deactivation — runs AFTER the draw phase where TextEdit
            // unconditionally re-enables im_active via _update_ime_window_position.
            // This ensures im_active=false when the next frame's input arrives.
            deactivate_ime_deferred(editor);
        }

        // ── Perf metrics recording ──────────────────────────────────────
        self.perf.record(perf::FrameMetrics {
            context_build_us: perf::Microseconds(0),
            engine_process_us: perf::Microseconds(0),
            effects_dispatch_us: perf::Microseconds(0),
            ui_update_us: perf::Microseconds(0),
            total_us: perf::Microseconds(
                u64::try_from(total_elapsed.as_micros()).unwrap_or(u64::MAX),
            ),
        });

        // ── Per-keystroke DEBUG summary ──────────────────────────────────
        if log::log_enabled!(target: "key", log::Level::Debug) {
            use std::fmt::Write;
            let mut summary = String::with_capacity(128);
            let _ = write!(summary, "{}  {}  session.process_key", key, mode_before);

            let cursor_after = (editor.get_caret_line(), editor.get_caret_column());
            if cursor_after != cursor_before {
                let _ = write!(
                    summary,
                    "  cursor={}:{}\u{2192}{}:{}",
                    cursor_before.0, cursor_before.1, cursor_after.0, cursor_after.1,
                );
            }

            if mode_after != mode_before {
                let _ = write!(summary, "  mode\u{2192}{}", mode_after);
            }

            let _ = write!(summary, "  {}\u{00b5}s", total_elapsed.as_micros(),);

            log::debug!(target: "key", "{}", summary);
        }

        consumed
    }

    /// Handle pending UI actions from GodotHost (deferred commands that need
    /// controller-level access).
    ///
    /// Actions that the plugin layer handles (OpenMappingDialog, SourceConfigFile)
    /// are stored in `transient.pending_ui_action` for the plugin to consume.
    /// Actions the controller handles directly (ShowUndoTree, Perf*, Vimdebug)
    /// are executed inline.
    fn handle_host_pending_ui_action(
        &mut self,
        action: crate::bridge::godot_host::PendingUiAction,
        editor: &Gd<CodeEdit>,
    ) {
        use crate::bridge::godot_host::PendingUiAction;
        match action {
            PendingUiAction::OpenMappingDialog | PendingUiAction::SourceConfigFile => {
                self.transient.pending_ui_action = Some(action);
            }
            PendingUiAction::ShowUndoTree => {
                let editor_id = editor.instance_id();
                let msg = {
                    let session = self.session.as_mut().expect("requires active session");
                    session
                        .host_mut()
                        .state_mut()
                        .buffer(editor_id)
                        .undo_tree()
                        .map_or_else(
                            || "No undo tree for this buffer".to_owned(),
                            |tree| tree.format_tree(),
                        )
                };
                let session = self.session.as_mut().expect("requires active session");
                crate::effects::messages::handle_show_message(
                    session.host_mut().state_mut().globals_mut(),
                    &msg,
                );
            }
            PendingUiAction::PerfReport => {
                let msg = self.perf.format_report();
                let session = self.session.as_mut().expect("requires active session");
                crate::effects::messages::handle_show_message(
                    session.host_mut().state_mut().globals_mut(),
                    &msg,
                );
            }
            PendingUiAction::PerfReset => {
                self.perf.reset();
                let session = self.session.as_mut().expect("requires active session");
                crate::effects::messages::handle_show_message(
                    session.host_mut().state_mut().globals_mut(),
                    ":perf reset",
                );
            }
            PendingUiAction::Vimdebug(cmd) => {
                self.handle_vimdebug_command(&cmd);
            }
        }
    }

    /// Handle :vimdebug commands routed from GodotHost.
    fn handle_vimdebug_command(&mut self, cmd: &str) {
        use super::vimdebug::VimdebugMode;

        let (mode, msg) = match cmd.trim() {
            "vimdebug" | "vimdebug on" => {
                if self.transient.vimdebug.mode() == VimdebugMode::Off {
                    (VimdebugMode::Watch, ":vimdebug ON (watch)")
                } else {
                    (VimdebugMode::Off, ":vimdebug OFF")
                }
            }
            "vimdebug off" => (VimdebugMode::Off, ":vimdebug OFF"),
            "vimdebug watch" => (VimdebugMode::Watch, ":vimdebug ON (watch)"),
            "vimdebug step" => (VimdebugMode::Step, ":vimdebug ON (step)"),
            _ => return,
        };
        self.transient.vimdebug.set_mode(mode);
        let session = self.session.as_mut().expect("requires active session");
        crate::effects::messages::handle_show_message(
            session.host_mut().state_mut().globals_mut(),
            msg,
        );
    }

    /// Force-resolve a pending mapping after timeout, then drain expanded keys.
    pub(crate) fn resolve_mapping_timeout_impl(&mut self, editor: &mut Gd<CodeEdit>) {
        log::debug!("resolve_mapping_timeout: resolving pending mapping");
        self.transient.operations_this_cycle = 0;

        {
            let session = self
                .session
                .as_mut()
                .expect("resolve_mapping_timeout: requires active session");
            let engine_mode = session.engine().mode();
            let auto_pairs_active = session.engine().options().auto_pairs().is_some();
            let scrolloff = usize_to_i32(session.engine().options().scrolloff());
            session.host_mut().refresh_from_editor();
            session
                .host_mut()
                .set_auto_brace_eligible(engine_mode.is_insert());
            session
                .host_mut()
                .set_engine_auto_pairs_active(auto_pairs_active);
            session.host_mut().set_scrolloff(scrolloff);
            session.host_mut().set_current_mode(engine_mode);
        }

        {
            let session = self
                .session
                .as_mut()
                .expect("resolve_mapping_timeout: requires active session");
            session.engine_mut().resolve_timeout();
            // drain_and_process_one calls build_context -> process -> deliver_effects
            // for each pending key, so effects are applied by GodotHost.
            while session.drain_and_process_one() {
                self.transient.operations_this_cycle =
                    self.transient.operations_this_cycle.saturating_add(1);
            }
        }

        // Handle any deferred actions produced during drain.
        {
            let session = self
                .session
                .as_mut()
                .expect("resolve_mapping_timeout: requires active session");
            let pending_actions = session.host_mut().take_pending_ui_actions();
            for action in pending_actions {
                self.handle_host_pending_ui_action(action, editor);
            }
        }

        {
            let session = self
                .session
                .as_mut()
                .expect("resolve_mapping_timeout: requires active session");
            let mode = session.engine().mode();
            session.host_mut().ensure_undo_balanced(mode);
        }
    }

    // ── Key passthrough ──────────────────────────────────────────────
    //
    // Decision order: mappings beat everything, then user overrides,
    // then host policy (F-keys, Alt/Meta), then the engine's own judgment.

    fn should_passthrough_key(&self, key: KeyEvent) -> bool {
        // Normalize non-Latin keys for mapping and passthrough lookup so that e.g.
        // Alt+Cyrillic-o matches a user's <A-j> mapping or passthrough entry.
        let mapping_key = normalize_key_for_mapping(key);

        // Mappings always take priority -- never passthrough mid-sequence.
        if self.engine().has_pending_mapping() || self.engine().could_start_mapping(mapping_key) {
            return false;
        }

        // User overrides: check both raw and normalized keys so that a passthrough
        // entry for 'j' works on both Latin and non-Latin layouts.
        if self.passthrough_keys.contains(&key) || self.passthrough_keys.contains(&mapping_key) {
            return true;
        }

        if super::passthrough::is_always_passthrough(key) {
            return true;
        }

        // Final arbiter: does the engine's built-in command set handle this key?
        !self.engine().would_handle_key(key)
    }

    /// Vimdebug step-mode key handler: n=next, p=prev, c=continue, q=quit.
    /// All keys are consumed while stepping.
    fn process_step_key(&mut self, key: KeyEvent, editor: &mut Gd<CodeEdit>) -> bool {
        let scrolloff = usize_to_i32(self.engine().options().scrolloff());
        let editor_id = editor.instance_id();
        let ch = key.as_char();

        match ch {
            Some('n') => {
                if let Some(idx) = self.transient.vimdebug.step_next() {
                    if let Some(ref effects) = self.transient.pending_step_effects {
                        if idx < effects.len() {
                            let effect = effects[idx].clone();
                            apply_step_effect_to_host(
                                self.session.as_mut().expect("requires active session"),
                                effect,
                                editor,
                                editor_id,
                                scrolloff,
                            );
                        }
                    }
                }
                if !self.transient.vimdebug.has_pending_steps() {
                    self.transient.vimdebug.step_quit();
                    self.transient.pending_step_effects = None;
                }
            }
            Some('p') => {
                self.transient.vimdebug.step_prev();
            }
            Some('c') => {
                let remaining = self.transient.vimdebug.step_continue();
                let mut all_effects = self
                    .transient
                    .pending_step_effects
                    .take()
                    .unwrap_or_default();
                let remaining_set: std::collections::HashSet<usize> =
                    remaining.into_iter().collect();
                let to_apply: Vec<vim_core::effects::Effect> = all_effects
                    .drain(..)
                    .enumerate()
                    .filter_map(|(i, e)| remaining_set.contains(&i).then_some(e))
                    .collect();
                for effect in to_apply {
                    apply_step_effect_to_host(
                        self.session.as_mut().expect("requires active session"),
                        effect,
                        editor,
                        editor_id,
                        scrolloff,
                    );
                }
                self.transient.vimdebug.step_quit();
            }
            Some('q') => {
                self.transient.vimdebug.step_quit();
                self.transient.pending_step_effects = None;
            }
            _ => {} // Consume all other keys while stepping
        }
        // Step effects bypass GodotHost::apply_effects, so the text cache
        // may be stale. Force a rebuild on the next access.
        if matches!(ch, Some('n') | Some('c')) {
            if let Some(session) = self.session.as_mut() {
                session.host_mut().invalidate_cache();
            }
        }
        true
    }
}

/// Apply a single deferred pass-2 effect in step mode.
fn apply_step_effect_to_host(
    session: &mut vim_core::execution::VimSession<GodotHost>,
    effect: vim_core::effects::Effect,
    editor: &mut Gd<CodeEdit>,
    editor_id: InstanceId,
    scrolloff: i32,
) {
    use vim_core::effects::Effect;

    let text = editor.get_text().to_string();
    let li = crate::bridge::codec::LineIndex::new(&text);
    let doc = crate::bridge::codec::DocumentView::new(&text, &li);

    let (_, host) = session.engine_and_host_mut();

    match effect {
        Effect::SetSelection {
            anchor,
            head,
            shape,
        } => {
            let mut port = crate::bridge::port_impl::CodeEditPort(editor);
            crate::effects::cursor::handle_set_selection(
                &mut port,
                &doc,
                anchor.get(),
                head.get(),
                shape,
            );
            let head_pos = doc.line_index.byte_to_line_col(doc.text, head.get());
            host.state_mut()
                .buffer(editor_id)
                .update_visual_selection(anchor, head, head_pos);
        }
        Effect::ClearSelection => {
            let mut port = crate::bridge::port_impl::CodeEditPort(editor);
            crate::effects::cursor::handle_clear_selection(&mut port);
            host.state_mut().buffer(editor_id).clear_visual_selection();
        }
        other => {
            let mut compound_actions = Vec::new();
            let highlight_yank_ms = host.highlight_yank_duration_ms();
            {
                let state = host.state_mut();
                let mut port = crate::bridge::port_impl::CodeEditPort(editor);
                crate::effects::dispatch::dispatch_pass2_effect(
                    other,
                    &mut port,
                    state,
                    &doc,
                    &mut compound_actions,
                    scrolloff,
                    highlight_yank_ms,
                    &mut crate::bridge::clipboard::GodotClipboard,
                );
            }
        }
    }
}

/// Convert a VimSession [`WindowNavAction`] to the godot-vim effect-layer
/// [`crate::effects::WindowNavAction`] for the navigation module.
///
/// Returns `None` for actions not supported in the Godot editor, logging a
/// warning so they are visible in the log output.
pub(super) fn convert_window_nav_action(
    nav: WindowNavAction,
) -> Option<crate::effects::WindowNavAction> {
    use crate::effects::WindowNavAction as GodotNav;
    match nav {
        WindowNavAction::MoveLeft => Some(GodotNav::MoveLeft),
        WindowNavAction::MoveRight => Some(GodotNav::MoveRight),
        WindowNavAction::MoveUp => Some(GodotNav::MoveUp),
        WindowNavAction::MoveDown => Some(GodotNav::MoveDown),
        WindowNavAction::CycleNext => Some(GodotNav::CycleNext),
        WindowNavAction::CyclePrev => Some(GodotNav::CyclePrev),
        WindowNavAction::Close => Some(GodotNav::CloseTab),
        WindowNavAction::RotateDown => Some(GodotNav::CycleNext),
        WindowNavAction::RotateUp => Some(GodotNav::CyclePrev),
        unsupported => {
            log::warn!("Ctrl-W {:?}: not supported in Godot editor", unsupported);
            None
        }
    }
}

/// Normalize a non-Latin [`KeyEvent`] to its Latin equivalent for mapping/passthrough lookup.
///
/// If the event carries a `latin_key` (e.g. the physical `j` key on a Cyrillic layout), a new
/// [`KeyEvent`] is returned with that Latin key and the original modifiers intact.  If there is
/// no Latin override the event is returned unchanged.
fn normalize_key_for_mapping(key: KeyEvent) -> KeyEvent {
    if let Some(latin) = key.latin_key() {
        KeyEvent::new(latin, key.modifiers())
    } else {
        key
    }
}

/// Activate IME for text input modes (Insert/Replace).
///
/// Cancels any in-progress composition first, then enables the OS IME so that
/// CJK and other complex input methods work while the user is typing.
/// Targets the editor's actual window (not MAIN_WINDOW_ID) so floating
/// script editors get correct IME activation on Windows.
fn activate_ime(editor: &mut Gd<CodeEdit>) {
    editor.cancel_ime();
    let window_id = editor
        .get_window()
        .map(|w| w.get_window_id())
        .unwrap_or(DisplayServer::MAIN_WINDOW_ID);
    DisplayServer::singleton()
        .window_set_ime_active_ex(true)
        .window_id(window_id)
        .done();
    update_ime_position(editor);
    log::trace!("IME activated for insert mode (window_id={})", window_id);
}

/// Deactivate IME when leaving text input modes.
///
/// Cancels any in-progress composition and disables the OS IME so that Normal
/// mode keystrokes are not intercepted by the input method.
/// Targets the editor's actual window for floating script editor support.
fn deactivate_ime(editor: &mut Gd<CodeEdit>) {
    editor.cancel_ime();
    let window_id = editor
        .get_window()
        .map(|w| w.get_window_id())
        .unwrap_or(DisplayServer::MAIN_WINDOW_ID);
    DisplayServer::singleton()
        .window_set_ime_active_ex(false)
        .window_id(window_id)
        .done();
    log::trace!("IME deactivated (window_id={})", window_id);
}

/// Schedule IME deactivation to run AFTER the current frame's draw phase.
///
/// Godot's TextEdit unconditionally calls `window_set_ime_active(true)` during
/// `NOTIFICATION_DRAW` (via `_update_ime_window_position`). The immediate
/// `deactivate_ime` call runs during the input phase — before draw — so TextEdit
/// re-enables IME before the next frame's input arrives. This deferred call
/// runs after draw, ensuring `im_active=false` survives into the next frame.
fn deactivate_ime_deferred(editor: &Gd<CodeEdit>) {
    let window_id = editor
        .get_window()
        .map(|w| w.get_window_id())
        .unwrap_or(DisplayServer::MAIN_WINDOW_ID);
    DisplayServer::singleton().call_deferred(
        "window_set_ime_active",
        &[false.to_variant(), window_id.to_variant()],
    );
}

/// Update the IME candidate window position to track the text cursor.
///
/// Called after IME activation and after every keystroke in insert-like
/// modes so the CJK candidate window stays next to the caret in real time.
/// Uses `get_caret_draw_pos` (editor-local) + `get_global_position`
/// (window-relative) for correct positioning in both docked and floating
/// script editors.
fn update_ime_position(editor: &Gd<CodeEdit>) {
    let caret_pos = editor.get_caret_draw_pos();
    let global_pos = editor.get_global_position();
    let ime_pos = Vector2i::new(
        (global_pos.x + caret_pos.x) as i32,
        (global_pos.y + caret_pos.y) as i32,
    );
    let window_id = editor
        .get_window()
        .map(|w| w.get_window_id())
        .unwrap_or(DisplayServer::MAIN_WINDOW_ID);
    DisplayServer::singleton()
        .window_set_ime_position_ex(ime_pos)
        .window_id(window_id)
        .done();
    log::trace!(
        "IME position updated to ({}, {}) window_id={}",
        ime_pos.x,
        ime_pos.y,
        window_id
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use vim_core::keymap::{Key, KeyEvent, Modifiers};

    #[test]
    fn normalize_with_latin_key_returns_latin() {
        let cyrillic_event =
            KeyEvent::new(Key::Char('\u{043E}'), Modifiers::ALT).with_latin(Key::Char('j'));
        let normalized = normalize_key_for_mapping(cyrillic_event);
        assert_eq!(normalized.key(), Key::Char('j'));
        assert_eq!(normalized.modifiers(), Modifiers::ALT);
    }

    #[test]
    fn normalize_without_latin_key_returns_original() {
        let ascii_event = KeyEvent::new(Key::Char('j'), Modifiers::ALT);
        let normalized = normalize_key_for_mapping(ascii_event);
        assert_eq!(normalized.key(), Key::Char('j'));
        assert_eq!(normalized.modifiers(), Modifiers::ALT);
    }

    #[test]
    fn normalize_preserves_ctrl_modifier() {
        let event = KeyEvent::new(Key::Char('\u{043E}'), Modifiers::CTRL | Modifiers::ALT)
            .with_latin(Key::Char('j'));
        let normalized = normalize_key_for_mapping(event);
        assert_eq!(normalized.key(), Key::Char('j'));
        assert_eq!(normalized.modifiers(), Modifiers::CTRL | Modifiers::ALT);
    }

    #[test]
    fn normalize_no_modifiers() {
        let event =
            KeyEvent::new(Key::Char('\u{043E}'), Modifiers::NONE).with_latin(Key::Char('j'));
        let normalized = normalize_key_for_mapping(event);
        assert_eq!(normalized.key(), Key::Char('j'));
        assert_eq!(normalized.modifiers(), Modifiers::NONE);
    }

    #[test]
    fn normalize_special_key_unchanged() {
        let event = KeyEvent::new(Key::Escape, Modifiers::NONE);
        let normalized = normalize_key_for_mapping(event);
        assert_eq!(normalized.key(), Key::Escape);
    }
}
