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
use crate::multi_cursor::keybindings::{self, MultiCursorAction};
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

#[allow(clippy::large_enum_variant)]
pub(crate) enum ControllerPhase {
    Attached { session: VimSession<GodotHost> },
    Detached { engine: VimEngine, state: ShellState },
}

pub(crate) struct ControllerContext {
    pub(crate) transient: TransientShellState,
    pub(crate) passthrough_keys: HashSet<KeyEvent>,
    pub(crate) multi_cursor_bindings: Vec<(KeyEvent, MultiCursorAction)>,
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
                multi_cursor_bindings: keybindings::default_bindings(),
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

    // ── Session accessors (only valid when attached) ─────────────────

    fn session_mut(&mut self) -> &mut VimSession<GodotHost> {
        match &mut self.phase {
            ControllerPhase::Attached { session } => session,
            ControllerPhase::Detached { .. } => panic!("VimController: not attached"),
        }
    }

    /// Number of active cursors (1 = single cursor, >1 = multi-cursor active).
    pub(crate) fn cursor_count(&self) -> usize {
        self.engine().state().multi_cursor().selections().len()
    }

    /// Scrolloff value from vim-core options.
    pub(crate) fn scrolloff(&self) -> i32 {
        crate::bridge::codec::usize_to_i32(self.engine().options().scrolloff())
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
        host.set_security_policy(self.ctx.security_policy.clone());
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
        let state = self.host_state_mut();
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
        let state = self
            .host_state_mut()
            .buffer(editor_id)
            .take_engine_state()
            .unwrap_or_default();
        self.engine_mut().on_buffer_enter(state);
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
            let editor_id = host.editor().instance_id();
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

    /// Destroy the active Ctrl+D match session. Called when secondary
    /// cursors are cleared (Escape, cursor count → 1) so the next Ctrl+D
    /// starts fresh from the primary cursor.
    pub(crate) fn clear_match_session(&mut self, editor_id: InstanceId) {
        self.host_state_mut()
            .buffer(editor_id)
            .clear_match_session();
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
        let mut response = self.engine_mut().source_config_text(text);
        let effects = response.take_effects();
        // When detached (no session), config effects are logged but cannot
        // be routed to shell state. This happens during initial enter_tree
        // before any editor is attached.
        let has_session = self.is_attached();
        for effect in effects {
            match effect {
                vim_core::effects::Effect::ShowInfo { info } => {
                    let msg = format!("{}", info);
                    if has_session {
                        crate::effects::messages::handle_show_message(
                            self.host_state_mut().globals_mut(),
                            &msg,
                        );
                    } else {
                        log::info!("reload_config (detached): {}", msg);
                    }
                }
                vim_core::effects::Effect::ShowError { error, .. } => {
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
        self.ctx.transient.pending_ui_action.take()
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
            let editor_id = session.host().editor().instance_id();
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

        // Refresh text_cache BEFORE syncing undo nodes — record_internal_undo_node
        // reads text_cache for the post-edit text (T1). Without this, text_cache
        // still holds the pre-edit text, producing an identity changeset.
        session.host_mut().invalidate_cache();

        // Sync internal undo nodes immediately with the correct T0 (old_text).
        // reconcile_external_text_change calls engine.apply_external_edit_with_recording()
        // which may create undo tree nodes that bypass the effect pipeline. Record
        // them in UndoStore now — deferring to the next process_key would lose the
        // correct pre-edit text (T0).
        {
            use vim_core::effects::Effect;
            let fc_node = session.engine_mut().take_last_force_committed_node();
            let ext_node = session.engine_mut().take_last_external_edit_node();
            if let Some(fc_id) = fc_node {
                session.host_mut().apply_effects(&[Effect::EndUndoGroup {
                    node_id: Some(fc_id),
                }]);
            }
            if let Some(ext_id) = ext_node {
                session
                    .host_mut()
                    .record_internal_undo_node(ext_id, &old_text_owned);
            }
            if fc_node.is_some() && session.engine().mode().is_insert() {
                session.host_mut().apply_effects(&[Effect::BeginUndoGroup {
                    cursor_strategy: vim_core::primitives::UndoCursorStrategy::FirstEdit,
                }]);
            }
        }

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

    // ── Processing entry points ────────────────────────────────────────

    /// Single entry point for keystroke processing from `gui_input`.
    pub(crate) fn process_cycle(
        &mut self,
        key: KeyEvent,
        editor: &mut Gd<CodeEdit>,
    ) -> PipelineOutcome {
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
        let ControllerPhase::Attached { ref mut session } = self.phase else {
            panic!("process_mouse_selection: requires active session");
        };

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

    // ── Multi-cursor action execution ──────────��──────────────────────

    /// Sync multi-cursor positions from the engine to Godot after a
    /// multi-cursor command executed outside the normal process_key pipeline
    /// (e.g., keybinding-triggered actions). Delegates to the same
    /// `sync_multi_cursors_to_godot` used by the process_cycle path.
    pub(crate) fn sync_multi_cursors_after_action(&mut self) {
        if let ControllerPhase::Attached { ref mut session } = self.phase {
            process::sync_multi_cursors_to_godot(session);
        }
    }

    /// Execute a VS Code-style multi-cursor action detected by the keybinding
    /// layer. Translates the action into the corresponding vim-core
    /// `MultiCursorCommand` and executes it through the engine.
    pub(crate) fn execute_multi_cursor_action(
        &mut self,
        action: crate::multi_cursor::keybindings::MultiCursorAction,
        editor: &Gd<CodeEdit>,
    ) {
        use crate::multi_cursor::keybindings::MultiCursorAction;
        use vim_core::execution::MultiCursorContext;
        use vim_core::primitives::Direction;
        use vim_core::state::MultiCursorCommand;

        let session = match &mut self.phase {
            ControllerPhase::Attached { session } => session,
            ControllerPhase::Detached { .. } => {
                log::warn!("execute_multi_cursor_action: no active session");
                return;
            }
        };

        // Sync cursor position from Godot before reading engine state —
        // the keybinding path bypasses process_cycle which normally does this.
        session.host_mut().refresh_from_editor();

        // Build context from the current document state.
        let text = editor.get_text().to_string();
        let line_count = editor.get_line_count() as usize;
        let search_pattern = session
            .engine()
            .state()
            .search()
            .pattern()
            .map(|s| s.to_owned());

        // Ctrl+D (AddNextMatch): find the next occurrence of the word under
        // the primary cursor and add a cursor there, keeping all existing
        // cursors. This matches VS Code's "Add Selection to Next Find Match".
        //
        // SkipAndAddNext is wrong here because with multiple cursors it
        // removes the primary before adding the next match.
        if matches!(action, MultiCursorAction::AddNextMatch) {
            self.execute_add_next_match(&text);
            return;
        }

        let cmd = match action {
            MultiCursorAction::AddNextMatch => unreachable!("handled above"),
            MultiCursorAction::AddCursorAbove => {
                MultiCursorCommand::AddCursorVertical(Direction::Backward)
            }
            MultiCursorAction::AddCursorBelow => {
                MultiCursorCommand::AddCursorVertical(Direction::Forward)
            }
            MultiCursorAction::SelectAllOccurrences => MultiCursorCommand::SelectAllOccurrences,
            MultiCursorAction::ClearSecondary => MultiCursorCommand::ClearSecondary,
        };

        let ctx = MultiCursorContext {
            text: &text,
            search_pattern: search_pattern.as_deref(),
            line_count,
        };

        match session.engine_mut().execute_multi_cursor(&cmd, &ctx) {
            Ok(effects) => {
                if !effects.is_empty() {
                    log::debug!(
                        "execute_multi_cursor_action: {:?} produced {} effects",
                        action,
                        effects.len()
                    );
                }
            }
            Err(e) => {
                log::warn!("execute_multi_cursor_action: {:?} failed: {}", action, e);
            }
        }
    }

    /// Ctrl+D: find the next occurrence of the word under the primary cursor
    /// and add a Godot caret there.
    ///
    /// Maintains a `MatchSession` in BufferState that locks the search word
    /// and tracks the Godot caret index of the last-added match. On each
    /// Ctrl+D, the search start is computed by reading the live caret
    /// position from Godot (immune to text edits between presses). The
    /// session auto-invalidates when the word under cursor changes.
    fn execute_add_next_match(&mut self, text: &str) {
        let session = match &mut self.phase {
            ControllerPhase::Attached { session } => session,
            ControllerPhase::Detached { .. } => return,
        };

        let primary_offset = {
            use vim_core::execution::VimHost;
            session.host().cursor_offset()
        };
        let word = match find_word_at_offset(text, primary_offset) {
            Some(w) => w.to_owned(),
            None => {
                log::debug!("execute_add_next_match: no word under cursor");
                return;
            }
        };

        let editor_id = session.host().editor().instance_id();
        let line_index = session.host().line_index().clone();
        let mut editor = session.host().editor().clone();

        // Determine search start from session state.
        // If an active session exists with the same word, read the live position
        // of the last-added caret from Godot (self-healing after text edits).
        // If the word changed or no session exists, start from primary cursor.
        let existing_session = session
            .host()
            .state()
            .buffer_ref(editor_id)
            .and_then(|b| b.match_session().cloned());

        let search_from = match &existing_session {
            Some(ms) if ms.word == word => {
                let idx = ms.last_caret_index;
                let caret_count = editor.get_caret_count();
                if idx >= 0 && idx < caret_count {
                    // Read live position — Godot adjusts caret positions after edits.
                    let line = editor.get_caret_line_ex().caret_index(idx).done();
                    let col = editor.get_caret_column_ex().caret_index(idx).done();
                    let byte_off = line_index.line_col_to_byte(text, line, col);
                    byte_off + word.len()
                } else {
                    // Caret index no longer valid (removed externally). Reset.
                    primary_offset + word.len()
                }
            }
            _ => {
                // No session or word changed — start fresh from primary cursor.
                primary_offset + word.len()
            }
        };

        // Search forward with whole-word matching (VS Code Ctrl+D semantics).
        // If the found match overlaps an existing caret, skip it and continue.
        // Two phases: forward from search_from, then wrap-around from 0.
        let mut start = search_from;
        while start < text.len() && !text.is_char_boundary(start) {
            start += 1;
        }
        let caret_count = editor.get_caret_count();
        let match_offset = 'search: {
            // Phase 1: forward search.
            let mut pos = start;
            while let Some(off) = find_whole_word(text, &word, pos) {
                if !caret_overlaps_match(&editor, &line_index, text, off, word.len(), caret_count) {
                    break 'search off;
                }
                pos = off + word.len();
            }
            // Phase 2: wrap-around from document start.
            let mut pos = 0;
            while let Some(off) = find_whole_word(text, &word, pos) {
                if off >= start {
                    break; // back to where phase 1 started — all matches checked
                }
                if !caret_overlaps_match(&editor, &line_index, text, off, word.len(), caret_count) {
                    break 'search off;
                }
                pos = off + word.len();
            }
            log::debug!("execute_add_next_match: all matches of '{}' selected", word);
            return;
        };

        let lc = line_index.byte_to_line_col(text, match_offset);
        let caret_idx = editor.add_caret(lc.line, lc.col);
        if caret_idx < 0 {
            log::warn!(
                "execute_add_next_match: add_caret({}, {}) failed",
                lc.line,
                lc.col
            );
            return;
        }

        session
            .host_mut()
            .state_mut()
            .buffer(editor_id)
            .set_match_session(word.clone(), caret_idx);
        log::debug!(
            "execute_add_next_match: caret {} at {}:{} for '{}'",
            caret_idx,
            lc.line,
            lc.col,
            word
        );
    }
}

/// Check if any existing Godot caret falls within [match_offset, match_offset + word_len).
fn caret_overlaps_match(
    editor: &Gd<CodeEdit>,
    line_index: &crate::bridge::codec::LineIndex,
    text: &str,
    match_offset: usize,
    word_len: usize,
    caret_count: i32,
) -> bool {
    let match_end = match_offset + word_len;
    for idx in 0..caret_count {
        let cl = editor.get_caret_line_ex().caret_index(idx).done();
        let cc = editor.get_caret_column_ex().caret_index(idx).done();
        let caret_byte = line_index.line_col_to_byte(text, cl, cc);
        if caret_byte >= match_offset && caret_byte < match_end {
            return true;
        }
    }
    false
}

/// Find the next whole-word occurrence of `word` in `text` starting from `from`.
/// Returns the byte offset of the match, or `None`.
/// A match is whole-word when the characters immediately before and after the
/// match are NOT word characters (alphanumeric or underscore).
fn find_whole_word(text: &str, word: &str, from: usize) -> Option<usize> {
    let haystack = text.get(from..)?;
    let mut search_from = 0;
    loop {
        let pos = haystack[search_from..].find(word)?;
        let abs_pos = from + search_from + pos;
        let match_start = search_from + pos;
        let match_end = match_start + word.len();

        let before_ok = if match_start == 0 && from == 0 {
            true
        } else {
            let abs_before = abs_pos;
            abs_before == 0
                || text[..abs_before]
                    .chars()
                    .next_back()
                    .map_or(true, |c| !c.is_alphanumeric() && c != '_')
        };
        let after_ok = if match_end >= haystack.len() {
            true
        } else {
            haystack[match_end..]
                .chars()
                .next()
                .map_or(true, |c| !c.is_alphanumeric() && c != '_')
        };

        if before_ok && after_ok {
            return Some(abs_pos);
        }
        // Advance past this non-whole-word match.
        search_from = match_start + word.len();
        if search_from >= haystack.len() {
            return None;
        }
    }
}

/// Extract the word at the given byte offset in the text.
///
/// Scans backward and forward from `offset` to find word boundaries using
/// Vim-style classification (alphanumeric + underscore = word character).
/// Returns `None` if the character at offset is not a word character.
fn find_word_at_offset(text: &str, offset: usize) -> Option<&str> {
    let mut clamped = offset.min(text.len().saturating_sub(1));
    while clamped > 0 && !text.is_char_boundary(clamped) {
        clamped -= 1;
    }

    let ch = text[clamped..].chars().next()?;
    if !ch.is_alphanumeric() && ch != '_' {
        return None;
    }

    // Scan backward to word start.
    let mut start = clamped;
    for (i, c) in text[..clamped].char_indices().rev() {
        if c.is_alphanumeric() || c == '_' {
            start = i;
        } else {
            break;
        }
    }

    // Scan forward to word end (exclusive).
    let mut end = clamped;
    for (i, c) in text[clamped..].char_indices() {
        if c.is_alphanumeric() || c == '_' {
            end = clamped + i + c.len_utf8();
        } else {
            break;
        }
    }

    text.get(start..end)
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
                multi_cursor_bindings: _,      // config
                security_policy: _,            // config
                perf: _,                       // persistent
                highlight_yank_duration_ms: _, // config
                code_complete_enabled: _,      // config
            } = ctx;
        }
    }
}
