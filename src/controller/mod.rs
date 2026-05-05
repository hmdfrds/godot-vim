//! Per-editor controller: mediates between Godot's event-driven input model
//! and vim-core's synchronous command model.
//!
//! Manages a [`VimSession<GodotHost>`] (when attached to an editor) or a bare
//! [`VimEngine`] (when detached). Each keystroke flows through
//! `VimSession::process_key()` with pre/post hooks for completion, passthrough,
//! vimdebug, IME, and perf. The controller never touches the Godot scene tree
//! directly -- UI actions that require tree access are deferred via
//! [`crate::bridge::godot_host::PendingUiAction`] for the plugin layer to execute.
//!
//! Sub-modules:
//! - [`process`] -- keystroke pipeline (`process_cycle`, `resolve_mapping_timeout`)
//! - [`completion`] -- CodeEdit autocomplete popup interception
//! - [`passthrough`] -- key bypass classification (F-keys, Alt/Meta, user overrides)
//! - [`perf`] -- per-keystroke latency tracking (`:perf`)
//! - [`vimdebug`] -- effect inspector (`:vimdebug watch/step`)

mod completion;
mod passthrough;
pub(crate) mod perf;
mod process;
pub(crate) mod reconcile;
pub(crate) mod vimdebug;

use std::collections::HashSet;

use compact_str::CompactString;
use godot::classes::CodeEdit;
use godot::prelude::*;
use vim_core::document::Document;
use vim_core::execution::{VimEngine, VimSession};
use vim_core::keymap::KeyEvent;
use vim_core::primitives::Direction;

use crate::bridge::godot_host::{GodotHost, PendingUiAction};
use crate::host::SecurityPolicy;
use crate::settings::{FileAccessScope, ShellExecution};
use crate::state::ShellState;

const PERF_RING_CAPACITY: usize = 1000;
/// Per-keystroke budget; exceeding this logs a warning with phase breakdown.
const PERF_BUDGET_US: perf::Microseconds = perf::Microseconds(2000);

/// Transient per-cycle / per-keystroke state that must be reset on every
/// cleanup path (dead editor, panic recovery, tab switch).
///
/// Grouping these fields into a single struct with a [`reset()`](Self::reset)
/// method ensures that all cleanup call-sites stay in sync — adding a new
/// transient field automatically gets cleaned up everywhere.
struct TransientShellState {
    /// Cross-drain runaway guard: catches `:norm` calling back into `drain_pending`.
    operations_this_cycle: u32,
    /// Deferred for the plugin layer (scene tree owner) after `process_cycle`.
    pending_ui_action: Option<PendingUiAction>,
    /// Effect inspector state (`:vimdebug watch/step`).
    vimdebug: vimdebug::VimdebugState,
    /// Pass-2 effects deferred by vimdebug step-mode.
    pending_step_effects: Option<Vec<vim_core::effects::Effect>>,
}

impl TransientShellState {
    fn new() -> Self {
        Self {
            operations_this_cycle: 0,
            pending_ui_action: None,
            vimdebug: vimdebug::VimdebugState::default(),
            pending_step_effects: None,
        }
    }

    /// Reset all transient fields to their clean defaults.
    ///
    /// Called by every cleanup path (dead editor, panic recovery, tab switch)
    /// to guarantee no stale state leaks across boundaries.
    fn reset(&mut self) {
        let Self {
            operations_this_cycle,
            pending_ui_action,
            vimdebug,
            pending_step_effects,
        } = self;
        *operations_this_cycle = 0;
        *pending_ui_action = None;
        vimdebug.set_mode(vimdebug::VimdebugMode::Off);
        *pending_step_effects = None;
    }
}

/// Per-editor orchestrator that bridges Godot's event-driven input to
/// vim-core's synchronous command model.
///
/// Created once in `enter_tree`, shared across all editor tabs. The engine
/// persists across attach/detach cycles; the host (`GodotHost`) is created
/// on attach and destroyed on detach. Between attach and detach, the engine
/// and host live together in a [`VimSession`]. When detached, the engine is
/// stored bare in `detached_engine`.
///
/// Invariant: exactly one of `session` or `detached_engine` is `Some` at all
/// times. Helper methods [`engine()`](Self::engine) / [`engine_mut()`](Self::engine_mut)
/// abstract over this.
pub(crate) struct VimController {
    /// Active session (engine + host). `Some` when an editor is attached.
    session: Option<VimSession<GodotHost>>,
    /// Bare engine stored between detach and the next attach.
    /// `Some` when no editor is attached.
    detached_engine: Option<VimEngine>,
    /// Per-cycle / per-keystroke state that must be reset on every cleanup path.
    /// See [`TransientShellState::reset()`] for the canonical reset logic.
    transient: TransientShellState,
    passthrough_keys: HashSet<KeyEvent>,
    security_policy: SecurityPolicy,
    perf: perf::PerfTracker,
    /// 0 = disabled.
    highlight_yank_duration_ms: u32,
    /// Whether Godot's native code completion should auto-trigger on typing.
    /// Mirrors `text_editor/completion/code_complete_enabled` from EditorSettings.
    code_complete_enabled: bool,
}

impl VimController {
    #[must_use]
    pub(crate) fn new() -> Self {
        let mut engine = VimEngine::new();
        engine.set_shadow_execution(true);
        Self {
            session: None,
            detached_engine: Some(engine),
            transient: TransientShellState::new(),
            passthrough_keys: HashSet::new(),
            security_policy: SecurityPolicy {
                shell_execution: ShellExecution::Disabled,
                file_access_scope: FileAccessScope::ProjectOnly,
                project_vimrc: crate::settings::ProjectVimrc::Sandbox,
            },
            perf: perf::PerfTracker::new(PERF_RING_CAPACITY, PERF_BUDGET_US),
            highlight_yank_duration_ms: u32::try_from(
                crate::settings::defaults::HIGHLIGHT_YANK_DURATION,
            )
            .unwrap_or(150),
            code_complete_enabled: true,
        }
    }

    // ── Engine accessors (work in both attached and detached state) ───

    /// Immutable access to the engine, regardless of attach state.
    fn engine(&self) -> &VimEngine {
        self.session
            .as_ref()
            .map(|s| s.engine())
            .or(self.detached_engine.as_ref())
            .expect("VimController invariant: engine always available")
    }

    /// Mutable access to the engine, regardless of attach state.
    fn engine_mut(&mut self) -> &mut VimEngine {
        if let Some(ref mut session) = self.session {
            return session.engine_mut();
        }
        self.detached_engine
            .as_mut()
            .expect("VimController invariant: engine always available")
    }

    // ── Session accessors (only valid when attached) ─────────────────

    fn session_mut(&mut self) -> &mut VimSession<GodotHost> {
        self.session.as_mut().expect("VimController: not attached")
    }

    /// Mutable access to the host's shell state.
    ///
    /// Only valid when a session is active (editor attached).
    fn host_state_mut(&mut self) -> &mut ShellState {
        self.session_mut().host_mut().state_mut()
    }

    // ── Attach / detach lifecycle ────────────────────────────────────

    /// Create a `VimSession<GodotHost>` by taking the detached engine and
    /// pairing it with a new `GodotHost` wrapping the given editor.
    ///
    /// Must only be called when detached (i.e., `detached_engine` is `Some`).
    /// Syncs controller-level config (security policy, highlight yank duration)
    /// into the new host.
    pub(crate) fn attach_session(&mut self, editor: Gd<CodeEdit>) {
        let engine = self
            .detached_engine
            .take()
            .expect("attach_session: must be in detached state");
        let mut host = GodotHost::new(editor);
        host.set_security_policy(self.security_policy.clone());
        host.set_highlight_yank_duration_ms(self.highlight_yank_duration_ms);
        let mut session = VimSession::from_parts(engine, host);
        let initial_text = session.host().text().to_owned();
        session.engine_mut().set_shadow_text(initial_text);
        self.session = Some(session);
    }

    /// Decompose the active session: drop the host, reclaim the engine.
    ///
    /// Returns the `GodotHost` for any final cleanup the caller needs.
    /// No-ops if already detached.
    pub(crate) fn detach_session(&mut self) -> Option<GodotHost> {
        let session = self.session.take()?;
        let (engine, host) = session.into_parts();
        self.detached_engine = Some(engine);
        Some(host)
    }

    /// Whether a session is currently active (editor attached).
    #[must_use]
    pub(crate) fn is_attached(&self) -> bool {
        self.session.is_some()
    }

    // ── Configuration setters ─────────────────────────────────────────

    pub(crate) fn set_passthrough_keys(&mut self, keys: &[KeyEvent]) {
        self.passthrough_keys = keys.iter().copied().collect();
    }

    pub(crate) fn set_security_policy(&mut self, policy: SecurityPolicy) {
        self.security_policy = policy;
    }

    pub(crate) fn set_highlight_yank_duration(&mut self, ms: u32) {
        self.highlight_yank_duration_ms = ms;
    }

    pub(crate) fn apply_settings(&mut self, snapshot: &crate::settings::SettingsSnapshot) {
        snapshot.apply_to_options(self.engine_mut().options_mut());
        self.set_passthrough_keys(&snapshot.passthrough_keys);
        self.set_security_policy(crate::host::SecurityPolicy {
            shell_execution: snapshot.shell_execution,
            file_access_scope: snapshot.file_access_scope,
            project_vimrc: snapshot.project_vimrc,
        });
        self.set_highlight_yank_duration(snapshot.highlight_yank_duration);
        self.code_complete_enabled = snapshot.code_complete_enabled;
    }

    // ── Public accessors ─────────────────────────────────────────────

    /// Build a snapshot of UI-relevant state for the rendering layer.
    ///
    /// **Side effect:** Drains one-shot events (`substitute_preview`,
    /// `highlight_yank`) from shell state. Safe to call once per cycle
    /// only — Godot's single-threaded signal model guarantees this.
    pub(crate) fn ui_snapshot(&mut self, editor_id: InstanceId) -> crate::types::UiSnapshot {
        // Extract engine-owned values first (immutable borrow of engine).
        let mode = self.engine().mode();
        let cmdline_prompt = if mode.is_command_line() {
            Some(self.engine().state().command_line().prompt())
        } else {
            None
        };
        let cmdline_input = CompactString::from(self.engine().state().command_line().input());
        let cmdline_cursor = self.engine().state().command_line().cursor();
        let recording_register = self
            .engine()
            .state()
            .macros()
            .recording_register()
            .map(|r| r.char());
        let search_pattern = self.engine().state().search().pattern().map(|p| {
            (
                CompactString::from(p),
                Direction::from(self.engine().state().search().direction()),
            )
        });
        let pending_keys = self.engine().pending_mapping_display();
        let pending_command = self.engine().pending_command_display();

        // Now borrow host state mutably for shell-state fields.
        let state = self.host_state_mut();
        let message = state.globals().message_status().clone();
        let hlsearch_enabled = state.globals().hlsearch_enabled();
        let visual_head = state
            .buffer_ref(editor_id)
            .and_then(|b| b.visual().map(|v| v.head_pos));
        let substitute_preview = state.take_substitute_preview();
        let highlight_yank = state.take_highlight_yank();

        let vimdebug = match (
            self.transient.vimdebug.provenance().cloned(),
            self.transient.vimdebug.effects_summary().cloned(),
        ) {
            (Some(provenance), Some(effects)) => match self.transient.vimdebug.step_status_line() {
                Some(step_status) => crate::types::VimdebugSnapshot::Step {
                    provenance,
                    effects,
                    range: self.transient.vimdebug.range(),
                    step_status,
                },
                None => crate::types::VimdebugSnapshot::Watch {
                    provenance,
                    effects,
                    range: self.transient.vimdebug.range(),
                },
            },
            _ => crate::types::VimdebugSnapshot::Inactive,
        };

        crate::types::UiSnapshot {
            mode,
            message,
            cmdline: crate::types::CommandLineState {
                prompt: cmdline_prompt,
                input: cmdline_input,
                cursor: cmdline_cursor,
            },
            recording_register,
            search_pattern,
            hlsearch_enabled,
            visual_head,
            pending_keys,
            pending_command,
            substitute_preview,
            vimdebug,
            highlight_yank,
        }
    }

    // ── Intent-revealing methods ─────────────────────────────────────
    //
    // Narrow, purpose-specific operations that limit the surface area
    // external callers can touch (vs. raw engine_mut()/state_mut()).

    /// Sync indent settings from CodeEdit on attach. CodeEdit is the source
    /// of truth for tab/space and indent widths — not plugin settings.
    pub(crate) fn sync_indent(&mut self, expandtab: bool, shiftwidth: usize, tabstop: usize) {
        let opts = self.engine_mut().options_mut();
        opts.set_expandtab(expandtab);
        opts.set_shiftwidth(shiftwidth);
        opts.set_tabstop(tabstop);
    }

    /// Sync from CodeEdit's language-specific comment delimiters (e.g., `"# %s"`
    /// for GDScript) so the `gc` commentary operator uses the right format.
    pub(crate) fn set_commentstring(&mut self, cs: &str) {
        self.engine_mut().options_mut().set_commentstring(cs);
    }

    /// Sync auto-brace pairs from CodeEdit so the engine handles auto-pairing
    /// during both normal execution and shadow macro replay.
    pub(crate) fn sync_auto_pairs(&mut self, editor: &Gd<godot::classes::CodeEdit>) {
        use vim_core::primitives::{AutoPairs, Pair};

        if !editor.is_auto_brace_completion_enabled() {
            self.engine_mut().options_mut().set_auto_pairs(None);
            return;
        }

        let dict = editor.get_auto_brace_completion_pairs();
        let mut pairs = Vec::new();
        for (k, v) in dict.iter_shared() {
            let open_str = k.to_string();
            let close_str = v.to_string();
            let mut open_chars = open_str.chars();
            let mut close_chars = close_str.chars();
            if let (Some(open), None, Some(close), None) = (
                open_chars.next(),
                open_chars.next(),
                close_chars.next(),
                close_chars.next(),
            ) {
                pairs.push(Pair { open, close });
            }
        }

        self.engine_mut()
            .options_mut()
            .set_auto_pairs(Some(AutoPairs {
                pairs: pairs.into(),
            }));
    }

    /// Restore per-buffer engine state for the given editor.
    ///
    /// Retrieves the saved `BufferLocalState` from `BufferState` (or uses
    /// `Default` for first-visit buffers) and calls `engine.on_buffer_enter()`
    /// to restore marks, changelist, last_visual, sticky_column, buffer_overrides,
    /// buffer_mappings, and exchange.
    pub(crate) fn restore_buffer_engine_state(&mut self, editor_id: InstanceId) {
        let state = self
            .host_state_mut()
            .buffer(editor_id)
            .take_engine_state()
            .unwrap_or_default();
        self.engine_mut().on_buffer_enter(state);
    }

    /// Seed the undo tree on first attach (no-op if already initialized).
    pub(crate) fn init_undo_tree(&mut self, editor_id: InstanceId, text: &str) {
        let buf = self.host_state_mut().buffer(editor_id);
        if buf.undo_tree().is_none() {
            buf.init_undo_tree(text);
        }
    }

    /// Evict buffer state for editors freed since the last sweep.
    ///
    /// Called from `attach()` and `perform_detach()` — natural choke points
    /// since every editor lifecycle transition passes through them. Uses
    /// Godot's ObjectDB to probe liveness.
    pub(crate) fn sweep_stale_buffers(&mut self) {
        let Some(ref mut session) = self.session else {
            return;
        };
        let removed = session.host_mut().state_mut().sweep_invalid_buffers(|id| {
            Gd::<godot::classes::Object>::try_from_instance_id(id).is_ok()
        });
        if !removed.is_empty() {
            log::debug!(
                "sweep_stale_buffers: evicted {} stale buffer(s)",
                removed.len()
            );
        }
        for id in removed {
            log::debug!("Evicted stale buffer state for editor #{}", id.to_i64());
        }
    }

    #[must_use]
    pub(crate) fn mode(&self) -> vim_core::primitives::Mode {
        self.engine().mode()
    }

    /// Canonical Tier 1 cleanup: comprehensive internal reset when no editor
    /// is available (dead editor, panic recovery, tab switch).
    ///
    /// Delegates to [`VimEngine::emergency_reset`] which clears parser, mode,
    /// state, typeahead, host pending, recording, command-line session,
    /// cmd_buffer, is_repeating, and changelist in one call. Then drains
    /// orphaned undo groups, clears substitute preview, and resets all
    /// transient shell state.
    ///
    /// Returns the number of Godot undo groups that were drained — callers
    /// with a live editor should close that many `end_complex_operation`
    /// calls and optionally `undo()` to roll back partial mutations.
    pub(crate) fn force_cleanup_without_editor(&mut self) -> u32 {
        log::debug!("force_cleanup_without_editor: canonical Tier 1 reset");
        self.engine_mut().emergency_reset();
        let godot_groups = if let Some(ref mut session) = self.session {
            let host = session.host_mut();
            let groups = host.undo_depth_mut().drain();
            host.state_mut().clear_substitute_preview();
            host.state_mut().take_highlight_yank();
            groups
        } else {
            0
        };
        self.transient.reset();
        godot_groups
    }

    /// Reset parser state — clears pending operator (e.g. `d` waiting for
    /// motion) so it doesn't leak to the next editor.
    ///
    /// Does NOT abort macro recording. Recording is a session-level concept
    /// that survives buffer switches, matching Vim's behavior where `qa...`
    /// continues across `:edit` commands. Recording is only aborted by
    /// emergency paths (`force_cleanup_without_editor` → `emergency_reset`).
    pub(crate) fn engine_reset_parser(&mut self) {
        self.engine_mut().reset_parser();
    }

    /// Convenience wrapper to reset transient shell state without touching
    /// the engine or undo depth. Used by cleanup paths that handle engine
    /// reset separately (e.g., the normal detach path).
    pub(crate) fn reset_transients(&mut self) {
        self.transient.reset();
    }

    /// Discard any unconsumed yank highlight to prevent cross-editor flash.
    pub(crate) fn clear_highlight_yank(&mut self) {
        if let Some(ref mut session) = self.session {
            session.host_mut().state_mut().take_highlight_yank();
        }
    }

    /// Save all per-buffer engine state for the current editor.
    ///
    /// Computes the cursor byte offset, calls `engine.on_buffer_leave()` to
    /// extract all per-buffer state (marks, changelist, last_visual, sticky_column,
    /// buffer_overrides, buffer_mappings, exchange), and stores the result in
    /// `BufferState` for later restoration.
    pub(crate) fn save_buffer_engine_state(
        &mut self,
        editor_id: InstanceId,
        editor: &Gd<CodeEdit>,
    ) {
        let line = editor.get_caret_line();
        let col = editor.get_caret_column();
        let text = editor.get_text().to_string();
        let line_index = crate::bridge::codec::LineIndex::new(&text);
        let offset = line_index.line_col_to_byte(&text, line, col);
        let engine_state = self.engine_mut().on_buffer_leave(offset);
        self.host_state_mut()
            .buffer(editor_id)
            .set_engine_state(engine_state);
    }

    /// Exit non-Normal mode by sending synthetic Escapes through the session
    /// pipeline until Normal mode is reached. This ensures macro recording
    /// captures the exit, visual marks (`<`/`>`), `LastVisualInfo`, insert-stop
    /// marks (`^`), and `EndUndo` effects are all produced -- identical to the
    /// user pressing Esc.
    ///
    /// Handles mode nesting (e.g., visual -> command-line -> visual -> normal)
    /// by looping. Safety limit of 5 prevents infinite loops if a bug causes
    /// Esc to not change mode.
    ///
    /// Returns `true` if the engine was in a non-Normal mode and Esc(s) were processed.
    pub(crate) fn exit_mode_via_pipeline(&mut self, _editor: &mut Gd<CodeEdit>) -> bool {
        if self.engine().mode().is_normal() {
            return false;
        }
        const MAX_ESC: usize = 5;

        // Pre-process setup for session.process_key().
        if let Some(ref mut session) = self.session {
            let engine_mode = session.engine().mode();
            let auto_pairs_active = session.engine().options().auto_pairs().is_some();
            let scrolloff =
                crate::bridge::codec::usize_to_i32(session.engine().options().scrolloff());
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

        for i in 0..MAX_ESC {
            if self.engine().mode().is_normal() {
                break;
            }
            log::debug!(
                "exit_mode_via_pipeline: pass {}, mode={}",
                i + 1,
                self.engine().mode()
            );
            if let Some(ref mut session) = self.session {
                let _ = session.process_key(KeyEvent::escape());
            }
        }
        if !self.engine().mode().is_normal() {
            log::error!(
                "exit_mode_via_pipeline: still in {} after {} Escapes",
                self.engine().mode(),
                MAX_ESC
            );
        }
        true
    }

    /// Clear Godot-side visual artifacts after the engine has already exited
    /// visual mode via the pipeline. Defense-in-depth: ensures no stale
    /// selection highlights remain even if the pipeline exit was a no-op.
    pub(crate) fn cleanup_visual_artifacts(
        &mut self,
        editor_id: InstanceId,
        editor: &mut Gd<CodeEdit>,
    ) {
        self.host_state_mut()
            .buffer(editor_id)
            .clear_visual_selection();
        editor.remove_secondary_carets();
        editor.deselect();
    }

    /// Drain any remaining undo depth as defense-in-depth after pipeline exit.
    /// The pipeline's `EndUndo` effect handles the normal case; this catches
    /// edge cases where undo groups are still open.
    pub(crate) fn drain_remaining_undo_depth(&mut self, editor: &mut Gd<CodeEdit>) {
        let remaining = if let Some(ref mut session) = self.session {
            session.host_mut().undo_depth_mut().drain()
        } else {
            0
        };
        for _ in 0..remaining {
            editor.end_complex_operation();
        }
    }

    /// Sync sticky column from a mouse click or external caret move.
    /// This is the only shell-side write path; the engine owns the column.
    pub(crate) fn set_engine_sticky_column(&mut self, col: usize) {
        self.engine_mut().set_sticky_column(col);
    }

    #[must_use]
    pub(crate) fn has_pending_mapping(&self) -> bool {
        self.engine().has_pending_mapping()
    }

    #[must_use]
    pub(crate) fn could_start_mapping(&self, key: vim_core::keymap::KeyEvent) -> bool {
        self.engine().could_start_mapping(key)
    }

    #[must_use]
    pub(crate) fn timeoutlen(&self) -> u32 {
        self.engine().timeoutlen()
    }

    /// Used by `on_mapping_timeout_impl` to detect whether timeout
    /// resolution produced any effects this cycle.
    #[must_use]
    pub(crate) fn operations_this_cycle(&self) -> u32 {
        self.transient.operations_this_cycle
    }

    // ── Config file sourcing ─────────────────────────────────────────

    /// Clear all user mappings and re-source config text. Clears first so
    /// that removed lines don't leave stale mapping entries.
    ///
    /// The engine currently produces only `ShowMessage`, `ShowError`, and
    /// `ClearHighlights` from config sourcing. The catch-all arm trips a
    /// debug assertion if a new effect type appears.
    pub(crate) fn reload_config(&mut self, text: &str) {
        self.engine_mut().clear_mappings();
        let mut response = self.engine_mut().source_config_text(text);
        let effects = response.take_effects();
        // When detached (no session), config effects are logged but cannot
        // be routed to shell state. This happens during initial enter_tree
        // before any editor is attached.
        let has_session = self.session.is_some();
        for effect in effects {
            match effect {
                vim_core::effects::Effect::ShowMessage { text: msg } => {
                    if has_session {
                        crate::effects::messages::handle_show_message(
                            self.host_state_mut().globals_mut(),
                            &msg,
                        );
                    } else {
                        log::info!("reload_config (detached): {}", msg);
                    }
                }
                vim_core::effects::Effect::ShowError { error } => {
                    if has_session {
                        crate::effects::messages::handle_show_error(
                            self.host_state_mut().globals_mut(),
                            &error,
                        );
                    } else {
                        log::warn!("reload_config (detached): {}", error);
                    }
                }
                vim_core::effects::Effect::ClearHighlights => {
                    if has_session {
                        crate::effects::search::handle_clear_highlights(
                            self.host_state_mut().globals_mut(),
                        );
                    }
                }
                other => {
                    debug_assert!(
                        false,
                        "reload_config: unexpected effect from config sourcing: {:?}. \
                         Add handling or add to the known-skip list.",
                        other.kind()
                    );
                    log::warn!(
                        "reload_config: unhandled effect {:?} from config sourcing (skipped)",
                        other.kind()
                    );
                }
            }
        }
    }

    // ── Pending UI actions ───────────────────────────────────────────

    pub(crate) fn take_pending_ui_action(&mut self) -> Option<PendingUiAction> {
        self.transient.pending_ui_action.take()
    }

    /// Panic recovery composed from the canonical Tier 1 cleanup.
    ///
    /// Delegates to [`force_cleanup_without_editor`](Self::force_cleanup_without_editor)
    /// for engine + shell reset, then uses the returned undo group count to
    /// close orphaned Godot undo groups, roll back partial text mutations,
    /// and restore the editor to a clean visual state.
    pub(crate) fn recover_from_panic(&mut self, editor: &mut Gd<CodeEdit>) {
        let godot_groups = self.force_cleanup_without_editor();
        for _ in 0..godot_groups {
            editor.end_complex_operation();
        }
        if godot_groups > 0 {
            log::warn!(
                "recover_from_panic: rolling back {} closed undo group(s) via editor.undo()",
                godot_groups,
            );
            editor.undo();
        }
        editor.deselect();
        editor.remove_secondary_carets();
        let editor_id = editor.instance_id();
        if let Some(ref mut session) = self.session {
            session
                .host_mut()
                .state_mut()
                .buffer(editor_id)
                .clear_visual_selection();
            session
                .host_mut()
                .state_mut()
                .globals_mut()
                .set_error("Recovered from internal error \u{2014} state reset to Normal mode");
        }
    }

    // ── External text change reconciliation ─────────────────────────────

    /// Detect and reconcile an external text change (e.g. Find-and-Replace,
    /// external formatter, Godot refactoring) by diffing the host's cached
    /// text against the live editor text.
    ///
    /// Returns `true` if a change was detected and reconciled.
    /// No-ops when detached (no session).
    pub(crate) fn reconcile_external_edit(&mut self, editor: &Gd<CodeEdit>) -> bool {
        use vim_core::document::Document;

        let session = match self.session.as_mut() {
            Some(s) => s,
            None => return false,
        };

        let new_text = editor.get_text().to_string();
        let old_text = session.host().text();

        if old_text == new_text {
            return false;
        }

        // Compute cursor byte offset in the new text.
        let new_index = crate::bridge::codec::LineIndex::new(&new_text);
        let cursor_byte = new_index.line_col_to_byte(
            &new_text,
            editor.get_caret_line(),
            editor.get_caret_column(),
        );

        // Clone old text before mutably borrowing the session for the engine.
        let old_text_owned = old_text.to_owned();

        reconcile::reconcile_external_text_change(
            session.engine_mut(),
            &old_text_owned,
            &new_text,
            cursor_byte,
            vim_core::execution::ExternalEditKind::HostNotified,
        );

        // Update the host's cache so the next call doesn't re-detect.
        session.host_mut().invalidate_cache();

        true
    }

    // ── Processing entry points ────────────────────────────────────────

    /// Single entry point for keystroke processing from `gui_input`.
    pub(crate) fn process_cycle(&mut self, key: KeyEvent, editor: &mut Gd<CodeEdit>) -> bool {
        self.process_cycle_impl(key, editor)
    }

    /// Force-resolve a pending mapping after timeout, then drain expanded keys.
    pub(crate) fn resolve_mapping_timeout(&mut self, editor: &mut Gd<CodeEdit>) {
        self.resolve_mapping_timeout_impl(editor)
    }

    /// Process a mouse drag selection detected by `on_caret_changed`.
    /// Returns `true` if any effects were produced.
    pub(crate) fn process_mouse_selection(
        &mut self,
        editor: &mut Gd<CodeEdit>,
        anchor_line: i32,
        anchor_col: i32,
        head_line: i32,
        head_col: i32,
        shape: vim_core::primitives::SelectionShape,
    ) -> bool {
        let session = self
            .session
            .as_mut()
            .expect("process_mouse_selection: requires active session");

        // Compute byte offsets from line/col using the host's document.
        let text = editor.get_text().to_string();
        let li = crate::bridge::codec::LineIndex::new(&text);
        let anchor_offset = li.line_col_to_byte(&text, anchor_line, anchor_col);
        let head_offset = li.line_col_to_byte(&text, head_line, head_col);

        // Sync host state before processing.
        let current_mode = session.engine().mode();
        session.host_mut().refresh_from_editor();
        session.host_mut().set_auto_brace_eligible(false); // entering Visual, not Insert
        session.host_mut().set_current_mode(current_mode);

        let result = session.process_mouse_selection(anchor_offset, head_offset, shape);

        // Handle any deferred actions (window nav from mouse selection is unlikely but safe).
        for action in &result.deferred_actions {
            match action {
                vim_core::execution::host_api::DeferredAction::WindowNav(nav) => {
                    if let Some(nav_action) = process::convert_window_nav_action(*nav) {
                        let control: Gd<godot::classes::Control> = editor.clone().upcast();
                        crate::navigation::handle_window_nav_action(&control, nav_action);
                    }
                }
                _ => {
                    log::warn!(
                        "process_mouse_selection: unhandled deferred action {:?}",
                        action
                    );
                }
            }
        }

        // Ensure undo groups are balanced after mouse selection processing.
        let mode = session.engine().mode();
        session.host_mut().ensure_undo_balanced(mode);

        result.consumed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Exhaustive field inventory for [`VimController`].
    ///
    /// Adding a new field causes a compile error until it is categorized here.
    /// This is the compile-time guarantee that cleanup paths stay complete.
    ///
    /// Categories:
    ///   engine     — cleaned by `emergency_reset()` inside `force_cleanup_without_editor`
    ///   shell      — cleaned selectively by `force_cleanup_without_editor`
    ///   undo       — cleaned by `undo_depth.drain()` inside `force_cleanup_without_editor`
    ///   transient  — in `TransientShellState`, cleaned by `transient.reset()`
    ///   config     — set via `apply_settings()`, never reset on cleanup
    ///   persistent — survives all cleanups
    #[test]
    fn cleanup_field_inventory() {
        #[allow(unused, unreachable_code)]
        fn check(c: VimController) {
            let VimController {
                session: _,                    // engine+host: emergency_reset() + host cleanup
                detached_engine: _,            // engine: emergency_reset() when detached
                transient: _,                  // transient: .reset()
                passthrough_keys: _,           // config
                security_policy: _,            // config
                perf: _,                       // persistent
                highlight_yank_duration_ms: _, // config
                code_complete_enabled: _,      // config
            } = c;
        }
    }
}
