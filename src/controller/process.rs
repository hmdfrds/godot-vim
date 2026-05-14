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

use crate::bridge::port::TextEditorPort;

use super::completion;
use super::perf;
use super::{ControllerContext, PipelineOutcome};
use crate::bridge::codec::usize_to_i32;
use crate::bridge::godot_host::GodotHost;

pub(super) fn process_cycle_impl(
    session: &mut vim_core::execution::VimSession<GodotHost>,
    ctx: &mut ControllerContext,
    key: KeyEvent,
    editor: &mut Gd<CodeEdit>,
) -> PipelineOutcome {
    ctx.transient.operations_this_cycle = 0;

    // Messages are one-shot: displayed after the producing keystroke,
    // cleared on the next. Mirrors vim-core's clear_transient().
    session.host_mut().state_mut().globals_mut().clear_message();

    if ctx.transient.vimdebug.is_step_mode()
        && ctx.transient.pending_step_effects.is_some()
    {
        process_step_key(session, ctx, key, editor);
        return PipelineOutcome::VimdebugStep;
    }

    if let Some(consumed) =
        completion::try_handle_completion(session.engine_mut(), key, editor)
    {
        log::debug!(
            "process_cycle: completion intercepted key={} consumed={}",
            key,
            consumed
        );
        if consumed {
            session.host_mut().invalidate_cache();
        }
        let mode = session.engine().mode();
        session.host_mut().ensure_undo_balanced(mode);
        return if consumed {
            PipelineOutcome::CompletionConsumed
        } else {
            PipelineOutcome::CompletionDeferred
        };
    }

    if should_passthrough_key(session.engine(), &ctx.passthrough_keys, key) {
        log::debug!(
            "process_cycle: passthrough key={} mode={}",
            key,
            session.engine().mode()
        );
        return PipelineOutcome::Passthrough;
    }

    // Capture pre-processing state for debug summary and IME lifecycle.
    let mode_before = session.engine().mode();
    let cursor_before = (editor.get_caret_line(), editor.get_caret_column());
    let total_start = std::time::Instant::now();

    // ── Pre-processing setup ────────────────────────────────────────
    let engine_mode = session.engine().mode();
    let auto_pairs_active = session.engine().options().auto_pairs().is_some();
    let scrolloff = usize_to_i32(session.engine().options().scrolloff());
    session.host_mut().refresh_from_editor();

    // ── Gap 2: Import mouse-added Ctrl+Click carets before process_key ──
    import_godot_carets_into_engine(session);

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
        .set_vimdebug_enabled(ctx.transient.vimdebug.is_enabled());

    // ── Vimdebug: capture provenance before engine process ──────────
    ctx.transient.vimdebug.clear_captures();

    // ── Clipboard sync: pre-populate + register for clipboard=unnamedplus ──
    let opts = session.engine().resolved_options();
    if opts.clipboard_has_unnamedplus() || opts.clipboard_has_unnamed() {
        let text = godot::classes::DisplayServer::singleton()
            .clipboard_get()
            .to_string();
        session.sync_clipboard(&text);
    }

    // ── CORE: session.process_key(key) ──────────────────────────────
    let result = session.process_key(key);
    let consumed = result.consumed;

    ctx.transient.operations_this_cycle =
        ctx.transient.operations_this_cycle.saturating_add(1);

    // ── Gap 1 & 5: Sync multi-cursor positions to Godot ────────────
    sync_multi_cursors_to_godot(session);

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
    let pending_actions = session.host_mut().take_pending_ui_actions();
    for action in pending_actions {
        handle_host_pending_ui_action(session, ctx, action, editor);
    }

    // ── Post-processing: ensure undo balanced ───────────────────────
    let mode = session.engine().mode();
    session.host_mut().ensure_undo_balanced(mode);

    // ── Post-processing: completion re-trigger ──────────────────────
    completion::maybe_retrigger_completion(
        session.engine(),
        key,
        editor,
        ctx.code_complete_enabled,
    );

    let total_elapsed = total_start.elapsed();

    let mode_after = session.engine().mode();
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
        deactivate_ime(editor);
        deactivate_ime_deferred(editor);
    }

    // ── Perf metrics recording ──────────────────────────────────────
    ctx.perf.record(perf::FrameMetrics {
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

    if consumed {
        PipelineOutcome::EngineConsumed(result)
    } else {
        PipelineOutcome::EngineIgnored(result)
    }
}

/// Handle pending UI actions from GodotHost (deferred commands that need
/// controller-level access).
///
/// Actions that the plugin layer handles (OpenMappingDialog, SourceConfigFile)
/// are stored in `transient.pending_ui_action` for the plugin to consume.
/// Actions the controller handles directly (ShowUndoTree, Perf*, Vimdebug)
/// are executed inline.
fn handle_host_pending_ui_action(
    session: &mut vim_core::execution::VimSession<GodotHost>,
    ctx: &mut ControllerContext,
    action: crate::bridge::godot_host::PendingUiAction,
    _editor: &mut Gd<CodeEdit>,
) {
    use crate::bridge::godot_host::PendingUiAction;
    match action {
        PendingUiAction::OpenMappingDialog
        | PendingUiAction::SourceConfigFile
        | PendingUiAction::ShowTooltip { .. } => {
            ctx.transient.pending_ui_action = Some(action);
        }
        PendingUiAction::ShowUndoTree => {
            // The undo tree is now engine-owned. Use `:undotree` which
            // triggers vim-core's Effect::UndoTreeSnapshot, handled by
            // dispatch (formatted by undo_format::format_undo_tree_snapshot).
            crate::effects::messages::handle_show_message(
                session.host_mut().state_mut().globals_mut(),
                "Use :undotree to display the undo tree",
            );
        }
        PendingUiAction::PerfReport => {
            let msg = ctx.perf.format_report();
            crate::effects::messages::handle_show_message(
                session.host_mut().state_mut().globals_mut(),
                &msg,
            );
        }
        PendingUiAction::PerfReset => {
            ctx.perf.reset();
            crate::effects::messages::handle_show_message(
                session.host_mut().state_mut().globals_mut(),
                ":perf reset",
            );
        }
        PendingUiAction::Vimdebug(cmd) => {
            handle_vimdebug_command(session, ctx, &cmd);
        }
    }
}

/// Handle :vimdebug commands routed from GodotHost.
fn handle_vimdebug_command(
    session: &mut vim_core::execution::VimSession<GodotHost>,
    ctx: &mut ControllerContext,
    cmd: &str,
) {
    use super::vimdebug::VimdebugMode;

    let (mode, msg) = match cmd.trim() {
        "vimdebug" | "vimdebug on" => {
            if ctx.transient.vimdebug.mode() == VimdebugMode::Off {
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
    ctx.transient.vimdebug.set_mode(mode);
    crate::effects::messages::handle_show_message(
        session.host_mut().state_mut().globals_mut(),
        msg,
    );
}

/// Force-resolve a pending mapping after timeout, then drain expanded keys.
pub(super) fn resolve_mapping_timeout_impl(
    session: &mut vim_core::execution::VimSession<GodotHost>,
    ctx: &mut ControllerContext,
    editor: &mut Gd<CodeEdit>,
) {
    log::debug!("resolve_mapping_timeout: resolving pending mapping");
    ctx.transient.operations_this_cycle = 0;

    let engine_mode = session.engine().mode();
    let auto_pairs_active = session.engine().options().auto_pairs().is_some();
    let scrolloff = usize_to_i32(session.engine().options().scrolloff());
    session.host_mut().refresh_from_editor();

    // Gap 2: Import mouse-added carets before processing.
    import_godot_carets_into_engine(session);

    session
        .host_mut()
        .set_auto_brace_eligible(engine_mode.is_insert());
    session
        .host_mut()
        .set_engine_auto_pairs_active(auto_pairs_active);
    session.host_mut().set_scrolloff(scrolloff);
    session.host_mut().set_current_mode(engine_mode);

    session.engine_mut().resolve_timeout();
    // drain_and_process_one calls build_context -> process -> deliver_effects
    // for each pending key, so effects are applied by GodotHost.
    while session.drain_and_process_one() {
        ctx.transient.operations_this_cycle =
            ctx.transient.operations_this_cycle.saturating_add(1);
    }

    // Gaps 1 & 5: Sync multi-cursor positions after drain.
    sync_multi_cursors_to_godot(session);

    // Handle any deferred actions produced during drain.
    let pending_actions = session.host_mut().take_pending_ui_actions();
    for action in pending_actions {
        handle_host_pending_ui_action(session, ctx, action, editor);
    }

    let mode = session.engine().mode();
    session.host_mut().ensure_undo_balanced(mode);
}

// ── Key passthrough ──────────────────────────────────────────────
//
// Chain-of-responsibility: MappingPriority -> UserOverride -> HostPolicy -> EngineQuery.
// See `controller/passthrough.rs` for filter definitions and chain runner.

fn should_passthrough_key(
    engine: &vim_core::execution::VimEngine,
    passthrough_keys: &std::collections::HashSet<KeyEvent>,
    key: KeyEvent,
) -> bool {
    use super::passthrough::{run_passthrough_chain, FilterContext};

    // Normalize non-Latin keys for mapping and passthrough lookup so that e.g.
    // Alt+Cyrillic-o matches a user's <A-j> mapping or passthrough entry.
    let normalized_key = normalize_key_for_mapping(key);

    let ctx = FilterContext {
        key,
        normalized_key,
        engine,
        user_overrides: passthrough_keys,
    };

    run_passthrough_chain(&ctx)
}

fn process_step_key(
    session: &mut vim_core::execution::VimSession<GodotHost>,
    ctx: &mut ControllerContext,
    key: KeyEvent,
    editor: &mut Gd<CodeEdit>,
) {
    let scrolloff = usize_to_i32(session.engine().options().scrolloff());
    let editor_id = editor.instance_id();
    let ch = key.as_char();

    match ch {
        Some('n') => {
            if let Some(idx) = ctx.transient.vimdebug.step_next() {
                if let Some(ref effects) = ctx.transient.pending_step_effects {
                    if idx < effects.len() {
                        let effect = effects[idx].clone();
                        apply_step_effect_to_host(
                            session,
                            effect,
                            editor,
                            editor_id,
                            scrolloff,
                        );
                    }
                }
            }
            if !ctx.transient.vimdebug.has_pending_steps() {
                ctx.transient.vimdebug.step_quit();
                ctx.transient.pending_step_effects = None;
            }
        }
        Some('p') => {
            ctx.transient.vimdebug.step_prev();
        }
        Some('c') => {
            let remaining = ctx.transient.vimdebug.step_continue();
            let mut all_effects = ctx
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
                    session,
                    effect,
                    editor,
                    editor_id,
                    scrolloff,
                );
            }
            ctx.transient.vimdebug.step_quit();
        }
        Some('q') => {
            ctx.transient.vimdebug.step_quit();
            ctx.transient.pending_step_effects = None;
        }
        _ => {} // Consume all other keys while stepping
    }
    if matches!(ch, Some('n') | Some('c')) {
        session.host_mut().invalidate_cache();
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
    let highlight_yank_ms = host.highlight_yank_duration_ms();
    let (state, pending_ui_actions) = host.state_and_pending_ui_actions_mut();

    match effect {
        Effect::SetSelection {
            anchor,
            head,
            shape,
        } => {
            let mut port = crate::bridge::port_impl::CodeEditPort(editor, pending_ui_actions);
            crate::effects::cursor::handle_set_selection(
                &mut port,
                &doc,
                anchor.get(),
                head.get(),
                shape,
            );
            let head_pos = doc.line_index.byte_to_line_col(doc.text, head.get());
            let anchor_pos = doc.line_index.byte_to_line_col(doc.text, anchor.get());
            state
                .buffer(editor_id)
                .update_visual_selection(anchor, head, head_pos, anchor_pos);
        }
        Effect::ClearSelection => {
            let mut port = crate::bridge::port_impl::CodeEditPort(editor, pending_ui_actions);
            crate::effects::cursor::handle_clear_selection(&mut port);
            state.buffer(editor_id).clear_visual_selection();
        }
        other => {
            let mut compound_actions = Vec::new();
            {
                let mut port = crate::bridge::port_impl::CodeEditPort(editor, pending_ui_actions);
                let env = crate::effects::dispatch::DispatchEnv {
                    doc: &doc,
                    scrolloff,
                    highlight_yank_duration_ms: highlight_yank_ms,
                    editor_id,
                };
                crate::effects::dispatch::dispatch_pass2_effect(
                    other,
                    &mut port,
                    state,
                    &env,
                    &mut compound_actions,
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

// ═══════════════════════════════════════════════════════════════════════════════
// Multi-cursor pipeline helpers
// ═══════════════════════════════════════════════════════════════════════════════

/// Gap 2: Import mouse-added Ctrl+Click carets from Godot into vim-core.
///
/// Called before `process_key` so the engine sees any carets the user added
/// via mouse interaction since the last keystroke.
///
/// Delegates to `compute_import_action` (in `multi_cursor::sync`) for the
/// algorithm, making that function the single source of truth for the
/// Godot→Engine import decision logic.
fn import_godot_carets_into_engine(session: &mut vim_core::execution::VimSession<GodotHost>) {
    use crate::multi_cursor::sync::{compute_import_action, ImportAction};
    use vim_core::execution::MultiCursorContext;
    use vim_core::primitives::Offset;
    use vim_core::state::MultiCursorCommand;

    let editor_id = session.host().editor().instance_id();
    let last_count = session
        .host()
        .state()
        .buffer_ref(editor_id)
        .map_or(1, |b| b.last_caret_count());

    // Fast path: if caret count hasn't changed, skip the text allocation entirely.
    let current_count = session.host().editor().get_caret_count() as usize;
    if current_count == last_count {
        return;
    }

    // Only allocate text/line_index now that we know an import is needed.
    let text = session.host().text_cache().to_owned();
    let line_index = session.host().line_index().clone();

    // Compute the import action using the shared algorithm.
    // Scope the mutable editor borrow so it's released before engine access.
    let action = {
        let (editor, _state, pending_ui_actions) = session.host_mut().editor_and_state_mut();
        let port = crate::bridge::port_impl::CodeEditPort(editor, pending_ui_actions);
        compute_import_action(&port, last_count, &line_index, &text)
    };

    match action {
        ImportAction::NoChange => return,
        ImportAction::AddCursors(offsets) => {
            let line_count = line_index.line_count();
            let host_cursor = {
                use vim_core::execution::VimHost;
                session.host().cursor_offset()
            };
            let (engine, _host) = session.engine_and_host_mut();
            // Fix the stale primary cursor before adding secondaries.
            // The keybinding path bypasses process_key, so the engine's
            // primary is wherever the last command left it — not where
            // Godot's caret actually is. sync_primary_cursor fixes this.
            engine.sync_primary_cursor(host_cursor);
            let ctx = MultiCursorContext {
                text: &text,
                search_pattern: None,
                line_count,
            };
            for offset in &offsets {
                if let Err(e) = engine.execute_multi_cursor(
                    &MultiCursorCommand::AddCursor(Offset::new(*offset)),
                    &ctx,
                ) {
                    log::debug!("multi-cursor import AddCursor: {e}");
                }
            }
        }
        ImportAction::FullResync(new_secondary_offsets) => {
            let line_count = line_index.line_count();
            let host_cursor = {
                use vim_core::execution::VimHost;
                session.host().cursor_offset()
            };
            let (engine, _host) = session.engine_and_host_mut();
            engine.sync_primary_cursor(host_cursor);
            let ctx = MultiCursorContext {
                text: &text,
                search_pattern: None,
                line_count,
            };
            if let Err(e) = engine.execute_multi_cursor(&MultiCursorCommand::ClearSecondary, &ctx) {
                log::debug!("multi-cursor import ClearSecondary: {e}");
            }
            for offset in &new_secondary_offsets {
                if let Err(e) = engine.execute_multi_cursor(
                    &MultiCursorCommand::AddCursor(Offset::new(*offset)),
                    &ctx,
                ) {
                    log::debug!("multi-cursor import AddCursor: {e}");
                }
            }
        }
    }

    // Update buffer state's last_caret_count.
    let current_count = session.host().editor().get_caret_count() as usize;
    let buf = session.host_mut().state_mut().buffer(editor_id);
    buf.set_last_caret_count(current_count);
}

/// Gaps 1 & 5: Sync multi-cursor positions (and visual selections) to Godot
/// after process_key completes.
///
/// Only activates when cursor_count > 1 (multi-cursor is active). When only
/// one cursor exists, the normal single-cursor path handles positioning.
pub(super) fn sync_multi_cursors_to_godot(
    session: &mut vim_core::execution::VimSession<GodotHost>,
) {
    let cursor_count = session.engine().state().multi_cursor().selections().len();

    // Single cursor: let the normal path handle it.
    // Multi-cursor→single-cursor cleanup is handled by the dispatch layer
    // (dispatch.rs caret cleanup), not here. See dispatch.rs cursor_effect_index logic.
    if cursor_count <= 1 {
        let editor_id = session.host().editor().instance_id();
        let was_multi = session
            .host()
            .state()
            .buffer_ref(editor_id)
            .map_or(false, |b| b.last_caret_count() > 1);

        // MC→single transition: dispatch may have skipped SetCursor effects
        // (it used Godot's stale pre-command cursor_count > 1). Reposition the
        // primary from the engine's authoritative selection state.
        let transition_pos = if was_multi {
            let selections = session.engine().state().multi_cursor().selections();
            let text = session.host().text_cache();
            let line_index = session.host().line_index();
            let offset = selections.primary().head().get();
            let lc = line_index.byte_to_line_col(text, offset);
            let scrolloff = usize_to_i32(session.engine().options().scrolloff());
            Some((lc.line as i32, lc.col as i32, scrolloff))
        } else {
            None
        };

        let host = session.host_mut();
        let (editor, state, pending_ui_actions) = host.editor_and_state_mut();
        let mut port = crate::bridge::port_impl::CodeEditPort(editor, pending_ui_actions);
        port.remove_secondary_carets();

        if let Some((line, col, scrolloff)) = transition_pos {
            port.set_caret_line_unfold(line, crate::bridge::port::ViewportAdjust::Adjust);
            port.set_caret_column(col);
            crate::effects::cursor::enforce_scrolloff(&mut port, line, scrolloff);
        }

        let buf = state.buffer(editor_id);
        buf.set_last_caret_count(1);
        return;
    }

    // Collect all data from immutable borrows into owned vecs before mutating.
    let (positions, visual_selections, editor_id, scrolloff) = {
        let selections = session.engine().state().multi_cursor().selections();
        let text = session.host().text_cache();
        let line_index = session.host().line_index();
        let mode = session.engine().mode();
        let eid = session.host().editor().instance_id();
        let scrolloff = usize_to_i32(session.engine().options().scrolloff());

        let positions: Vec<(usize, usize, usize)> = selections
            .ranges()
            .iter()
            .map(|r| {
                let offset = r.head().get();
                let lc = line_index.byte_to_line_col(text, offset);
                (lc.line as usize, lc.col as usize, offset)
            })
            .collect();

        // Gap 5: Compute per-cursor visual selections in Godot-ready coordinates.
        // Converts Vim-inclusive selections to Godot-exclusive [from, to) format
        // with +1 on the far end, matching handle_set_selection in cursor.rs.
        let visual_selections: Option<Vec<(usize, usize, usize, usize)>> =
            if let Some(vt) = mode.visual_type() {
                use vim_core::primitives::VisualType;
                Some(
                    selections
                        .ranges()
                        .iter()
                        .map(|r| {
                            let anchor_offset = r.anchor().get();
                            let head_offset = r.head().get();
                            let anchor_lc =
                                line_index.byte_to_line_col(text, anchor_offset);
                            let head_lc = line_index.byte_to_line_col(text, head_offset);
                            let (al, ac) =
                                (anchor_lc.line as usize, anchor_lc.col as usize);
                            let (hl, hc) =
                                (head_lc.line as usize, head_lc.col as usize);

                            match vt {
                                VisualType::Char => {
                                    if head_offset >= anchor_offset {
                                        let line_len =
                                            line_index.line_char_count(text, hl);
                                        (al, ac, hl, (hc + 1).min(line_len))
                                    } else {
                                        let line_len =
                                            line_index.line_char_count(text, al);
                                        (al, (ac + 1).min(line_len), hl, hc)
                                    }
                                }
                                VisualType::Line => {
                                    let top = al.min(hl);
                                    let bot = al.max(hl);
                                    let bot_len =
                                        line_index.line_char_count(text, bot);
                                    if hl >= al {
                                        (top, 0, bot, bot_len)
                                    } else {
                                        (bot, bot_len, top, 0)
                                    }
                                }
                                VisualType::Block => {
                                    let min_col = ac.min(hc);
                                    let max_col = ac.max(hc);
                                    let line_len =
                                        line_index.line_char_count(text, hl);
                                    if hc <= ac {
                                        (hl, (max_col + 1).min(line_len), hl, min_col)
                                    } else {
                                        (hl, min_col, hl, (max_col + 1).min(line_len))
                                    }
                                }
                                _ => {
                                    if head_offset >= anchor_offset {
                                        let line_len =
                                            line_index.line_char_count(text, hl);
                                        (al, ac, hl, (hc + 1).min(line_len))
                                    } else {
                                        let line_len =
                                            line_index.line_char_count(text, al);
                                        (al, (ac + 1).min(line_len), hl, hc)
                                    }
                                }
                            }
                        })
                        .collect(),
                )
            } else {
                None
            };

        (positions, visual_selections, eid, scrolloff)
    };

    // Mutably borrow host to sync — use split borrow helper.
    let host = session.host_mut();
    let (editor, state, pending_ui_actions) = host.editor_and_state_mut();
    let mut port = crate::bridge::port_impl::CodeEditPort(editor, pending_ui_actions);
    let buffer_state = state.buffer(editor_id);

    // Gap 1: Sync cursor positions to editor.
    crate::multi_cursor::sync::sync_cursors_to_editor(&positions, &mut port, buffer_state);

    // Gap 5: Sync visual selections if applicable.
    if let Some(ref sels) = visual_selections {
        crate::multi_cursor::sync::sync_selections_to_editor(sels, &mut port);
    }

    if !positions.is_empty() {
        let primary_line = positions[0].0 as i32;
        port.set_caret_line_unfold(
            primary_line,
            crate::bridge::port::ViewportAdjust::Adjust,
        );
        port.adjust_viewport_to_caret();
        crate::effects::cursor::enforce_scrolloff(&mut port, primary_line, scrolloff);
    }
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

#[cfg(test)]
const HANDLED_VISUAL_TYPES: &[vim_core::primitives::VisualType] = &[
    vim_core::primitives::VisualType::Char,
    vim_core::primitives::VisualType::Line,
    vim_core::primitives::VisualType::Block,
];

#[cfg(test)]
mod visual_type_coverage_tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn visual_type_dispatch_covers_all_variants() {
        let handled: HashSet<_> = HANDLED_VISUAL_TYPES.iter().copied().collect();
        let all: HashSet<_> = vim_core::primitives::VisualType::ALL.iter().copied().collect();
        let missing: Vec<_> = all.difference(&handled).collect();
        assert!(
            missing.is_empty(),
            "Unhandled VisualType variants: {:?}",
            missing
        );
    }

    #[test]
    fn handled_visual_types_has_no_duplicates() {
        let mut seen = HashSet::new();
        for kind in HANDLED_VISUAL_TYPES {
            assert!(
                seen.insert(kind),
                "Duplicate in HANDLED_VISUAL_TYPES: {:?}",
                kind
            );
        }
    }
}

#[cfg(test)]
const HANDLED_DEFERRED_ACTIONS: &[vim_core::execution::host_api::DeferredActionKind] = &[
    vim_core::execution::host_api::DeferredActionKind::WindowNav,
];

#[cfg(test)]
mod deferred_action_coverage_tests {
    use super::*;
    use std::collections::HashSet;
    use vim_core::execution::host_api::DeferredActionKind;

    #[test]
    fn deferred_action_dispatch_covers_all_variants() {
        let handled: HashSet<_> = HANDLED_DEFERRED_ACTIONS.iter().copied().collect();
        let all: HashSet<_> = DeferredActionKind::ALL.iter().copied().collect();
        let missing: Vec<_> = all.difference(&handled).collect();
        assert!(
            missing.is_empty(),
            "Unhandled DeferredActionKind variants: {:?}",
            missing
        );
    }

    #[test]
    fn handled_deferred_actions_has_no_duplicates() {
        let mut seen = HashSet::new();
        for kind in HANDLED_DEFERRED_ACTIONS {
            assert!(
                seen.insert(kind),
                "Duplicate in HANDLED_DEFERRED_ACTIONS: {:?}",
                kind
            );
        }
    }
}
