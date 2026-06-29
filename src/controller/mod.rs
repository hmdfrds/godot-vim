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
//! - [`passthrough`] -- key bypass chain-of-responsibility (F-keys, Meta, user overrides, engine query)
//! - [`perf`] -- per-keystroke latency tracking (`:perf`)
//! - [`vimdebug`] -- effect inspector (`:vimdebug watch/step`)

mod completion;
mod passthrough;
pub(crate) mod perf;
mod pipeline_outcome;
mod process;
pub(crate) mod reconcile;
pub(crate) mod vimdebug;

pub(crate) use pipeline_outcome::PipelineOutcome;

use std::collections::HashSet;

use compact_str::CompactString;
use godot::classes::CodeEdit;
use godot::prelude::*;
use vim_core::document::Document;
use vim_core::execution::{VimEngine, VimHost, VimSession};
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
pub(crate) struct TransientShellState {
    /// Cross-drain runaway guard: catches `:norm` calling back into `drain_pending`.
    operations_this_cycle: u32,
    /// Deferred for the plugin layer (scene tree owner) after `process_cycle`.
    pending_ui_actions: Vec<PendingUiAction>,
    /// Effect inspector state (`:vimdebug watch/step`).
    vimdebug: vimdebug::VimdebugState,
    /// Pass-2 effects deferred by vimdebug step-mode.
    pending_step_effects: Option<Vec<vim_core::effects::Effect>>,
}

impl TransientShellState {
    fn new() -> Self {
        Self {
            operations_this_cycle: 0,
            pending_ui_actions: Vec::new(),
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
            pending_ui_actions,
            vimdebug,
            pending_step_effects,
        } = self;
        *operations_this_cycle = 0;
        pending_ui_actions.clear();
        vimdebug.set_mode(vimdebug::VimdebugMode::Off);
        *pending_step_effects = None;
    }
}

#[allow(clippy::large_enum_variant)]
pub(crate) enum ControllerPhase {
    Attached { session: VimSession<GodotHost> },
    Detached { engine: VimEngine, state: ShellState },
}

pub(crate) struct ControllerContext {
    pub(crate) transient: TransientShellState,
    pub(crate) passthrough_keys: HashSet<KeyEvent>,
    pub(crate) security_policy: SecurityPolicy,
    pub(crate) perf: perf::PerfTracker,
    /// 0 = disabled.
    pub(crate) highlight_yank_duration_ms: u32,
    /// Whether Godot's native code completion should auto-trigger on typing.
    /// Mirrors `text_editor/completion/code_complete_enabled` from EditorSettings.
    pub(crate) code_complete_enabled: bool,
}

/// Per-editor orchestrator that bridges Godot's event-driven input to
/// vim-core's synchronous command model.
///
/// Created once in `enter_tree`, shared across all editor tabs. The engine
/// persists across attach/detach cycles; the host (`GodotHost`) is created
/// on attach and destroyed on detach. Between attach and detach, the engine
/// and host live together in a [`VimSession`]. When detached, the engine is
/// stored bare in [`ControllerPhase::Detached`].
///
/// Split into `phase` + `ctx` so that methods can borrow the session (via
/// `phase`) and transient/config state (via `ctx`) simultaneously without
/// conflicting `&mut self` borrows.
pub(crate) struct VimController {
    pub(crate) phase: ControllerPhase,
    pub(crate) ctx: ControllerContext,
}

impl VimController {
    #[must_use]
    pub(crate) fn new() -> Self {
        let mut engine = VimEngine::new();
        engine.set_shadow_execution(true);
        Self {
            phase: ControllerPhase::Detached {
                engine,
                state: ShellState::default(),
            },
            ctx: ControllerContext {
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
            },
        }
    }

    // ── Engine accessors (work in both attached and detached state) ───

    pub(crate) fn engine(&self) -> &VimEngine {
        match &self.phase {
            ControllerPhase::Attached { session } => session.engine(),
            ControllerPhase::Detached { engine, .. } => engine,
        }
    }

    pub(crate) fn engine_mut(&mut self) -> &mut VimEngine {
        match &mut self.phase {
            ControllerPhase::Attached { session } => session.engine_mut(),
            ControllerPhase::Detached { engine, .. } => engine,
        }
    }

    /// Number of active cursors (1 = single cursor, >1 = multi-cursor active).
    pub(crate) fn cursor_count(&self) -> usize {
        self.engine().state().multi_cursor().selections().len()
    }

    /// Mutable access to the host's shell state.
    ///
    /// Returns `None` when detached (no active session).
    fn host_state_mut(&mut self) -> Option<&mut ShellState> {
        let ControllerPhase::Attached { ref mut session } = self.phase else {
            return None;
        };
        Some(session.host_mut().state_mut())
    }

    // ── Attach / detach lifecycle ────────────────────────────────────

    /// Create a `VimSession<GodotHost>` by taking the detached engine and
    /// pairing it with a new `GodotHost` wrapping the given editor.
    ///
    /// Must only be called when detached.
    /// Syncs controller-level config (security policy, highlight yank duration)
    /// into the new host.
    pub(crate) fn attach_session(&mut self, editor: Gd<CodeEdit>) {
        let old_phase = std::mem::replace(
            &mut self.phase,
            // Temporary placeholder; overwritten below.
            ControllerPhase::Detached {
                engine: VimEngine::new(),
                state: ShellState::default(),
            },
        );
        let ControllerPhase::Detached { engine, state } = old_phase else {
            panic!("attach_session: must be in detached state");
        };
        let mut host = GodotHost::new(editor);
        host.set_state(state);
        host.set_security_policy(self.ctx.security_policy);
        host.set_highlight_yank_duration_ms(self.ctx.highlight_yank_duration_ms);
        let mut session = VimSession::from_parts(engine, host);
        let initial_text = session.host().text().to_owned();
        session.engine_mut().set_shadow_text(initial_text);
        self.phase = ControllerPhase::Attached { session };
    }

    /// Decompose the active session: drop the host, reclaim the engine.
    ///
    /// Returns the `GodotHost` for any final cleanup the caller needs.
    /// No-ops if already detached.
    pub(crate) fn detach_session(&mut self) -> Option<GodotHost> {
        let old_phase = std::mem::replace(
            &mut self.phase,
            ControllerPhase::Detached {
                engine: VimEngine::new(),
                state: ShellState::default(),
            },
        );
        match old_phase {
            ControllerPhase::Attached { session } => {
                let (engine, mut host) = session.into_parts();
                let state = host.take_state();
                self.phase = ControllerPhase::Detached { engine, state };
                Some(host)
            }
            detached @ ControllerPhase::Detached { .. } => {
                self.phase = detached;
                None
            }
        }
    }

    /// Whether a session is currently active (editor attached).
    #[must_use]
    pub(crate) fn is_attached(&self) -> bool {
        matches!(self.phase, ControllerPhase::Attached { .. })
    }

    // ── Configuration setters ─────────────────────────────────────────

    pub(crate) fn set_passthrough_keys(&mut self, keys: &[KeyEvent]) {
        self.ctx.passthrough_keys = keys.iter().copied().collect();
    }

    pub(crate) fn set_security_policy(&mut self, policy: SecurityPolicy) {
        self.ctx.security_policy = policy;
    }

    pub(crate) fn set_highlight_yank_duration(&mut self, ms: u32) {
        self.ctx.highlight_yank_duration_ms = ms;
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
        self.ctx.code_complete_enabled = snapshot.code_complete_enabled;
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
        let cursor_count = self.cursor_count();

        // Now borrow host state mutably for shell-state fields.
        let (message, hlsearch_enabled, cursor_style, visual_head, block_visual, substitute_preview, highlight_yank) =
            if let ControllerPhase::Attached { ref mut session } = self.phase {
                let state = session.host_mut().state_mut();
                let message = state.globals().message_status().clone();
                let hlsearch_enabled = state.globals().hlsearch_enabled();
                let cursor_style = state.cursor_style();
                let visual_head = state
                    .buffer_ref(editor_id)
                    .and_then(|b| b.visual().map(|v| v.head_pos));
                let block_visual = if matches!(
                    mode,
                    vim_core::primitives::Mode::Visual(vim_core::primitives::VisualType::Block)
                ) {
                    state.buffer_ref(editor_id).and_then(|b| {
                        b.visual().map(|v| crate::types::BlockVisualGeometry {
                            anchor_line: v.anchor_pos.line,
                            anchor_col: v.anchor_pos.col,
                            head_line: v.head_pos.line,
                            head_col: v.head_pos.col,
                        })
                    })
                } else {
                    None
                };
                let substitute_preview = state.take_substitute_preview();
                let highlight_yank = state.take_highlight_yank();
                (message, hlsearch_enabled, cursor_style, visual_head, block_visual, substitute_preview, highlight_yank)
            } else {
                use crate::types::StatusMessage;
                use vim_core::primitives::CursorStyle;
                (StatusMessage::default(), false, CursorStyle::for_mode(mode), None, None, None, None)
            };

        let vimdebug = match (
            self.ctx.transient.vimdebug.provenance().cloned(),
            self.ctx.transient.vimdebug.effects_summary().cloned(),
        ) {
            (Some(provenance), Some(effects)) => match self.ctx.transient.vimdebug.step_status_line() {
                Some(step_status) => crate::types::VimdebugSnapshot::Step {
                    provenance,
                    effects,
                    range: self.ctx.transient.vimdebug.range(),
                    step_status,
                },
                None => crate::types::VimdebugSnapshot::Watch {
                    provenance,
                    effects,
                    range: self.ctx.transient.vimdebug.range(),
                },
            },
            _ => crate::types::VimdebugSnapshot::Inactive,
        };

        crate::types::UiSnapshot {
            mode,
            cursor_style,
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
            block_visual,
            cursor_count,
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
        let engine_state = self
            .host_state_mut()
            .and_then(|s| s.buffer(editor_id).take_engine_state())
            .unwrap_or_default();
        self.engine_mut().on_buffer_enter(engine_state);
    }

    /// Evict buffer state for editors freed since the last sweep.
    ///
    /// Called from `attach()` and `perform_detach()` — natural choke points
    /// since every editor lifecycle transition passes through them. Uses
    /// Godot's ObjectDB to probe liveness.
    pub(crate) fn sweep_stale_buffers(&mut self) {
        let ControllerPhase::Attached { ref mut session } = self.phase else {
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
    /// cmd_buffer, is_repeating, and changelist in one call. Then discards
    /// any orphaned undo group pending text, clears substitute preview, and
    /// resets all transient shell state.
    pub(crate) fn force_cleanup_without_editor(&mut self) {
        log::debug!("force_cleanup_without_editor: canonical Tier 1 reset");
        self.engine_mut().emergency_reset();
        if let ControllerPhase::Attached { ref mut session } = self.phase {
            let host = session.host_mut();
            let editor_id = host.editor_id();
            // Discard any pending undo group text (orphaned begin_group).
            host.state_mut()
                .buffer(editor_id)
                .undo_store_mut()
                .take_pending_text();
            host.state_mut().clear_substitute_preview();
            host.state_mut().take_highlight_yank();
        }
        self.ctx.transient.reset();
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
        self.ctx.transient.reset();
    }

    /// Clear multi-cursor state on buffer leave. Multi-cursor is per-buffer
    /// and must not persist across buffer switches.
    pub(crate) fn clear_multi_cursor_on_detach(&mut self) {
        use vim_core::execution::MultiCursorContext;
        use vim_core::state::MultiCursorCommand;

        if self.engine().state().multi_cursor().selections().len() <= 1 {
            return;
        }
        // ClearSecondary doesn't use the context fields, but the API requires one.
        let ctx = MultiCursorContext {
            text: "",
            search_pattern: None,
            line_count: 0,
        };
        if let Err(e) = self
            .engine_mut()
            .execute_multi_cursor(&MultiCursorCommand::ClearSecondary, &ctx)
        {
            log::warn!("clear_multi_cursor_on_detach: {}", e);
        }
    }

    /// Discard any unconsumed yank highlight to prevent cross-editor flash.
    pub(crate) fn clear_highlight_yank(&mut self) {
        if let ControllerPhase::Attached { ref mut session } = self.phase {
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
        if let Some(state) = self.host_state_mut() {
            state.buffer(editor_id).set_engine_state(engine_state);
        }
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
        if let ControllerPhase::Attached { ref mut session } = self.phase {
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
            if let ControllerPhase::Attached { ref mut session } = self.phase {
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

        // Sync multi-cursor positions and ensure undo balance after
        // pipeline-driven mode exit. Without this, multi-cursor Godot
        // carets can be stale and orphaned undo groups can leak.
        if let ControllerPhase::Attached { ref mut session } = self.phase {
            process::sync_multi_cursors_to_godot(session);
            let mode = session.engine().mode();
            session.host_mut().ensure_undo_balanced(mode);
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
        if let Some(state) = self.host_state_mut() {
            state.buffer(editor_id).clear_visual_selection();
        }
        editor.remove_secondary_carets();
        editor.deselect();
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
        self.ctx.transient.operations_this_cycle
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

        // Seed editor-idiom multi-cursor bindings as defaults.
        // These are sourced BEFORE the user's .godot-vimrc, so user config
        // overrides them (later `nmap` on the same key wins).
        let _ = self.engine_mut().source_config_text(
            "nmap <C-S-Down> :addcursor below<CR>\n\
             vmap <C-S-Down> :addcursor below<CR>\n\
             nmap <C-S-Up> :addcursor above<CR>\n\
             vmap <C-S-Up> :addcursor above<CR>\n\
             nmap <C-S-l> :selectall<CR>\n\
             vmap <C-S-l> :selectall<CR>\n\
             nmap <leader>mn :addnext<CR>\n\
             nmap <leader>mN :addprev<CR>\n\
             nmap <leader>ma :selectall<CR>\n\
             nmap <leader>ms :cursorsplit<CR>\n\
             nmap <leader>mx :cursorremove<CR>\n\
             nmap <leader>mp :cursorprimary next<CR>\n\
             nmap <leader>mf :cursorfilter \n\
             nmap <leader>mF :cursorfilter! ",
        );

        let mut response = self.engine_mut().source_config_text(text);
        let effects = response.take_effects();
        // When detached (no session), config effects are logged but cannot
        // be routed to shell state. This happens during initial enter_tree
        // before any editor is attached.
        for effect in effects {
            match effect {
                vim_core::effects::Effect::ShowInfo { info } => {
                    let msg = format!("{}", info);
                    if let Some(state) = self.host_state_mut() {
                        crate::effects::messages::handle_show_message(
                            state.globals_mut(),
                            &msg,
                        );
                    } else {
                        log::info!("reload_config (detached): {}", msg);
                    }
                }
                vim_core::effects::Effect::ShowError { error, .. } => {
                    if let Some(state) = self.host_state_mut() {
                        crate::effects::messages::handle_show_error(
                            state.globals_mut(),
                            &error,
                        );
                    } else {
                        log::warn!("reload_config (detached): {}", error);
                    }
                }
                vim_core::effects::Effect::ClearHighlights => {
                    if let Some(state) = self.host_state_mut() {
                        crate::effects::search::handle_clear_highlights(
                            state.globals_mut(),
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

    pub(crate) fn take_pending_ui_actions(&mut self) -> Vec<PendingUiAction> {
        std::mem::take(&mut self.ctx.transient.pending_ui_actions)
    }

    /// Panic recovery composed from the canonical Tier 1 cleanup.
    ///
    /// Delegates to [`force_cleanup_without_editor`](Self::force_cleanup_without_editor)
    /// for engine + shell reset. If an undo group was pending (partial text
    /// mutation), restores the editor text from the pending snapshot.
    /// Restores the editor to a clean visual state.
    pub(crate) fn recover_from_panic(&mut self, editor: &mut Gd<CodeEdit>) {
        // Capture pending text BEFORE force_cleanup discards it.
        let pending_text = if let ControllerPhase::Attached { ref mut session } = self.phase {
            let editor_id = session.host().editor_id();
            session
                .host_mut()
                .state_mut()
                .buffer(editor_id)
                .undo_store_mut()
                .take_pending_text()
        } else {
            None
        };

        self.force_cleanup_without_editor();

        // Restore pre-mutation text if we had a pending group.
        if let Some(restore_text) = pending_text {
            log::warn!(
                "recover_from_panic: restoring editor text from pending undo group snapshot"
            );
            editor.set_text(&godot::prelude::GString::from(&restore_text));
        }

        editor.deselect();
        editor.remove_secondary_carets();
        let editor_id = editor.instance_id();
        if let ControllerPhase::Attached { ref mut session } = self.phase {
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

    /// Reconcile a `text_set` signal: `CodeEdit.set_text()` replaced the
    /// buffer wholesale (file reload, VCS revert).
    ///
    /// Semantics differ from `reconcile_external_edit`:
    /// - Godot destroys its own undo stack on `set_text()`.
    /// - The caret is reset to (0,0) by Godot.
    /// - We fence (reset) the engine undo tree and clear the UndoStore,
    ///   since old undo entries reference pre-reload text.
    /// - Positions (marks, jumplist) are remapped through the diff so they
    ///   survive across the reload when possible.
    ///
    /// Returns `true` if a change was detected and reconciled.
    /// No-ops when detached (no session).
    pub(crate) fn reconcile_text_set(&mut self, editor: &Gd<CodeEdit>) -> bool {
        let session = match &mut self.phase {
            ControllerPhase::Attached { session } => session,
            ControllerPhase::Detached { .. } => return false,
        };

        let new_text = editor.get_text().to_string();
        let old_text = session.host().text();

        if old_text == new_text {
            return false;
        }

        // ① Save the old cursor byte offset BEFORE any engine mutation.
        //    The host's cached cursor_offset was set during the last
        //    refresh_from_editor() call — still the pre-set_text() value
        //    since no process_key() has run in this cycle. Godot has already
        //    reset the live caret to (0,0) which would be useless for diff
        //    quality.
        let old_cursor_offset = session.host().cursor_offset();

        // Clone old text before mutably borrowing the session.
        let old_text_owned = old_text.to_owned();

        // ② Force Normal mode if not already.
        if !session.engine().mode().is_normal() {
            session.engine_mut().emergency_reset();
        }

        // ③ Remap marks, jumplist, changelist through the diff.
        //    Use the old cursor offset for better diff quality (Godot's 0,0
        //    would clamp the common-prefix to zero, defeating suffix detection).
        reconcile::reconcile_external_text_change(
            session.engine_mut(),
            &old_text_owned,
            &new_text,
            old_cursor_offset,
            vim_core::execution::ExternalEditKind::HostNotified,
        );

        // ④ Clear changelist — entries reference pre-reload positions.
        session.engine_mut().changelist_mut().clear();

        // ⑤ Fence the engine undo tree — all existing nodes reference
        //    pre-reload text and would produce garbage if replayed.
        session.engine_mut().undo_tree_mut().fence();

        // ⑥ Discard any force-committed or external-edit undo nodes that
        //    reconcile created (they were just fenced away).
        let _ = session.engine_mut().take_last_force_committed_node();
        let _ = session.engine_mut().take_last_external_edit_node();

        // ⑦ Clear UndoStore for this buffer.
        {
            let editor_id = session.host().editor().instance_id();
            session
                .host_mut()
                .state_mut()
                .buffer(editor_id)
                .undo_store_mut()
                .clear();
        }

        // ⑧ Update shadow document directly to new text.
        session.engine_mut().set_shadow_text(&new_text);

        // ⑨ Refresh the host text cache so it reflects the new text.
        session.host_mut().invalidate_cache();

        // ⑩ Restore cursor: clamp old line/col to new text bounds.
        {
            let old_index = crate::bridge::codec::LineIndex::new(&old_text_owned);
            let old_lc = old_index.byte_to_line_col(&old_text_owned, old_cursor_offset);
            let new_index = crate::bridge::codec::LineIndex::new(&new_text);

            let max_line = new_index.line_count().saturating_sub(1);
            let clamped_line = (old_lc.line as usize).min(max_line);
            let clamped_line_i32 = crate::bridge::codec::usize_to_i32(clamped_line);

            // Clamp column to the length of the new line.
            let new_line_len = {
                let line_byte_start = new_index.line_col_to_byte(
                    &new_text,
                    clamped_line_i32,
                    0,
                );
                // Find end of line (next newline or end of text).
                let rest = &new_text[line_byte_start..];
                let line_byte_len = rest.find('\n').unwrap_or(rest.len());
                // Count chars in the line for the column bound.
                new_text[line_byte_start..line_byte_start + line_byte_len]
                    .chars()
                    .count()
            };
            let clamped_col = (old_lc.col as usize).min(new_line_len.saturating_sub(1));
            let clamped_col_i32 = crate::bridge::codec::usize_to_i32(clamped_col);

            editor.clone().set_caret_line(clamped_line_i32);
            editor.clone().set_caret_column(clamped_col_i32);
        }

        // ⑪ Clear multi-cursor — positions are invalidated by full reload.
        if session.engine().state().multi_cursor().is_active() {
            use vim_core::execution::MultiCursorContext;
            use vim_core::state::MultiCursorCommand;

            let ctx = MultiCursorContext {
                text: &new_text,
                search_pattern: None,
                line_count: 0,
            };
            let _ = session
                .engine_mut()
                .execute_multi_cursor(&MultiCursorCommand::ClearSecondary, &ctx);
            editor.clone().remove_secondary_carets();
        }

        true
    }

    /// Detect and reconcile an external text change (e.g. Find-and-Replace,
    /// external formatter, Godot refactoring) by diffing the host's cached
    /// text against the live editor text.
    ///
    /// Returns `true` if a change was detected and reconciled.
    /// No-ops when detached (no session).
    pub(crate) fn reconcile_external_edit(&mut self, editor: &Gd<CodeEdit>) -> bool {
        let session = match &mut self.phase {
            ControllerPhase::Attached { session } => session,
            ControllerPhase::Detached { .. } => return false,
        };

        let new_text = editor.get_text().to_string();
        let old_text = session.host().text();

        if old_text == new_text {
            return false;
        }

        // Compute cursor byte offset in the new text.
        // When multi-cursor is active, use usize::MAX so decompose_multi_site_diff
        // doesn't clamp the common-prefix at a single caret position — the diff
        // envelope must cover all replacement sites.
        let new_index = crate::bridge::codec::LineIndex::new(&new_text);
        let cursor_byte = if session.engine().state().multi_cursor().is_active() {
            usize::MAX
        } else {
            new_index.line_col_to_byte(
                &new_text,
                editor.get_caret_line(),
                editor.get_caret_column(),
            )
        };

        // Clone old text before mutably borrowing the session for the engine.
        let old_text_owned = old_text.to_owned();

        reconcile::reconcile_external_text_change(
            session.engine_mut(),
            &old_text_owned,
            &new_text,
            cursor_byte,
            vim_core::execution::ExternalEditKind::HostNotified,
        );

        // Refresh text_cache BEFORE syncing undo nodes — record_internal_undo_node
        // reads text_cache for the post-edit text (T1). Without this, text_cache
        // still holds the pre-edit text, producing an identity changeset.
        session.host_mut().invalidate_cache();
        sync_undo_nodes_after_external_edit(session, &old_text_owned);

        // H1: Clear secondary cursors on external edit. External text changes
        // (Find-Replace, formatter, etc.) invalidate secondary cursor positions
        // because reconciliation only syncs the primary cursor. This matches
        // VS Code behavior where external edits collapse multi-cursor.
        if session.engine().state().multi_cursor().is_active() {
            use vim_core::state::MultiCursorCommand;
            let ctx = vim_core::execution::MultiCursorContext {
                text: &new_text,
                search_pattern: None,
                line_count: 0,
            };
            let _ = session
                .engine_mut()
                .execute_multi_cursor(&MultiCursorCommand::ClearSecondary, &ctx);
            editor.clone().remove_secondary_carets();
        }

        true
    }
}

/// Sync internal undo nodes created by engine operations that bypass the
/// effect pipeline (force-committed groups, external-edit nodes). Must be
/// called after any reconciliation that invokes apply_external_edit.
pub(crate) fn sync_undo_nodes_after_external_edit(
    session: &mut vim_core::execution::VimSession<GodotHost>,
    pre_edit_text: &str,
) {
    use vim_core::effects::Effect;
    let fc_node = session.engine_mut().take_last_force_committed_node();
    let ext_node = session.engine_mut().take_last_external_edit_node();
    if let Some(fc_id) = fc_node {
        let editor_id = session.host().editor().instance_id();
        let has_pending = session
            .host()
            .state()
            .buffer_ref(editor_id)
            .is_some_and(|b| b.undo_store().has_pending());
        if has_pending {
            session.host_mut().apply_effects(&[Effect::EndUndoGroup {
                node_id: Some(fc_id),
            }]);
        }
    }
    if let Some(ext_id) = ext_node {
        session
            .host_mut()
            .record_internal_undo_node(ext_id, pre_edit_text);
    }
    if fc_node.is_some() && session.engine().mode().is_insert() {
        session.host_mut().apply_effects(&[Effect::BeginUndoGroup {
            cursor_strategy: vim_core::primitives::UndoCursorStrategy::FirstEdit,
        }]);
    }

    #[cfg(debug_assertions)]
    {
        if let Some(shadow_text) = session.engine().shadow_text() {
            let host_text = session.host().text();
            debug_assert_eq!(
                shadow_text, host_text,
                "shadow document out of sync after undo node sync"
            );
        }
    }
}

impl VimController {
    // ── Processing entry points ────────────────────────────────────────

    /// Single entry point for keystroke processing from `gui_input`.
    pub(crate) fn process_cycle(
        &mut self,
        key: KeyEvent,
        editor: &mut Gd<CodeEdit>,
    ) -> PipelineOutcome {
        let ControllerPhase::Attached { ref mut session } = self.phase else {
            log::warn!("process_cycle: not attached");
            return PipelineOutcome::Passthrough;
        };
        process::process_cycle_impl(session, &mut self.ctx, key, editor)
    }

    /// Force-resolve a pending mapping after timeout, then drain expanded keys.
    pub(crate) fn resolve_mapping_timeout(&mut self, editor: &mut Gd<CodeEdit>) {
        let ControllerPhase::Attached { ref mut session } = self.phase else {
            log::warn!("resolve_mapping_timeout: not attached");
            return;
        };
        process::resolve_mapping_timeout_impl(session, &mut self.ctx, editor);
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
        let ControllerPhase::Attached { ref mut session } = self.phase else {
            return false; // Not attached — nothing to process
        };

        // Compute byte offsets from line/col using the host's document.
        let text = editor.get_text().to_string();
        let li = crate::bridge::codec::LineIndex::new(&text);
        let anchor_offset = li.line_col_to_byte(&text, anchor_line, anchor_col);
        let head_offset = li.line_col_to_byte(&text, head_line, head_col);

        // Sync host state before processing.
        let current_mode = session.engine().mode();
        session.host_mut().refresh_from_editor();
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

    /// Exhaustive field inventory for [`VimController`], [`ControllerPhase`],
    /// and [`ControllerContext`].
    ///
    /// Adding a new field causes a compile error until it is categorized here.
    /// This is the compile-time guarantee that cleanup paths stay complete.
    ///
    /// Categories:
    ///   engine     — cleaned by `emergency_reset()` inside `force_cleanup_without_editor`
    ///   shell      — cleaned selectively by `force_cleanup_without_editor`
    ///   transient  — in `TransientShellState`, cleaned by `transient.reset()`
    ///   config     — set via `apply_settings()`, never reset on cleanup
    ///   persistent — survives all cleanups
    #[test]
    fn cleanup_field_inventory() {
        #[allow(unused, unreachable_code)]
        fn check(c: VimController) {
            let VimController {
                phase,  // engine+host lifecycle (see phase inventory below)
                ctx,    // config + transient (see ctx inventory below)
            } = c;

            match phase {
                ControllerPhase::Attached { session: _ } => {
                    // engine+host: emergency_reset() + host cleanup
                }
                ControllerPhase::Detached {
                    engine: _, // engine: emergency_reset() when detached
                    state: _,  // persistent: transferred to/from host on attach/detach
                } => {}
            }

            let ControllerContext {
                transient: _,                  // transient: .reset()
                passthrough_keys: _,           // config
                security_policy: _,            // config
                perf: _,                       // persistent
                highlight_yank_duration_ms: _, // config
                code_complete_enabled: _,      // config
            } = ctx;
        }
    }
}
