//! Per-editor controller: mediates between Godot's event-driven input model
//! and vim-core's synchronous command model.
//!
//! Owns the [`VimEngine`] and orchestrates each keystroke through a four-stage
//! pipeline: context build → engine process → effect dispatch → host request
//! handling. The controller never touches the Godot scene tree directly —
//! UI actions that require tree access are deferred via [`PendingUiAction`]
//! for the plugin layer to execute.
//!
//! Sub-modules:
//! - [`process`] — keystroke pipeline (`process_cycle`, `process_single_key`, `drain_pending`)
//! - [`host_bridge`] — host request dispatch and controller-command interception
//! - [`completion`] — CodeEdit autocomplete popup interception
//! - [`norm`] — `:norm` compound execution across line ranges
//! - [`passthrough`] — key bypass classification (F-keys, Alt/Meta, user overrides)
//! - [`perf`] — per-keystroke latency tracking (`:perf`)
//! - [`vimdebug`] — effect inspector (`:vimdebug watch/step`)

mod completion;
mod context;
mod host_bridge;
mod norm;
mod passthrough;
pub(crate) mod perf;
mod process;
pub(crate) mod vimdebug;

use std::collections::HashSet;

use compact_str::CompactString;
use godot::classes::CodeEdit;
use godot::prelude::*;
use vim_core::execution::VimEngine;
use vim_core::keymap::KeyEvent;
use vim_core::primitives::Direction;

use crate::host::SecurityPolicy;
use crate::settings::{FileAccessScope, ShellExecution};
use crate::state::ShellState;

/// Actions deferred for the plugin layer (which owns the scene tree) to
/// execute after `process_cycle`. The controller has no scene tree access.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PendingUiAction {
    OpenMappingDialog,
    SourceConfigFile,
}

/// Runaway guard for macro/mapping replay within a single `process_cycle`.
/// Sized for `999@q` worst case: 999 replays * 20 keys = ~20k, with 5x margin.
const MAX_DRAIN_ITERATIONS: u32 = 100_000;

const PERF_RING_CAPACITY: usize = 1000;
/// Per-keystroke budget; exceeding this logs a warning with phase breakdown.
const PERF_BUDGET_US: perf::Microseconds = perf::Microseconds(2000);

/// Host request recursion depth limit. Typical depth is 1-2;
/// `:source` chains can reach 3. Five allows headroom without risk.
const MAX_HOST_DEPTH: u32 = 5;

/// Transient per-cycle / per-keystroke state that must be reset on every
/// cleanup path (dead editor, panic recovery, tab switch).
///
/// Grouping these fields into a single struct with a [`reset()`](Self::reset)
/// method ensures that all cleanup call-sites stay in sync — adding a new
/// transient field automatically gets cleaned up everywhere.
struct TransientShellState {
    /// Cross-drain runaway guard: catches `:norm` calling back into `drain_pending`.
    operations_this_cycle: u32,
    /// Avoids re-fetching the full document text from Godot (FFI round-trip)
    /// on successive keystrokes that don't mutate text. Tagged with
    /// `InstanceId` to self-invalidate on buffer switch.
    persistent_text: Option<(InstanceId, String)>,
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
            persistent_text: None,
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
            persistent_text,
            pending_ui_action,
            vimdebug,
            pending_step_effects,
        } = self;
        *operations_this_cycle = 0;
        *persistent_text = None;
        *pending_ui_action = None;
        vimdebug.set_mode(vimdebug::VimdebugMode::Off);
        *pending_step_effects = None;
    }
}

/// Per-editor orchestrator that owns the [`VimEngine`] and bridges Godot's
/// event-driven input to vim-core's synchronous command model.
///
/// Created once in `enter_tree`, shared across all editor tabs. Per-buffer
/// state is keyed by `InstanceId` in [`ShellState`]; the engine itself is
/// tab-agnostic. Lifetime spans `enter_tree` to `exit_tree`.
pub(crate) struct VimController {
    engine: VimEngine,
    state: ShellState,
    /// Ensures Godot's `begin/end_complex_operation` calls are always balanced,
    /// even after panics or abnormal mode transitions.
    undo_depth: crate::effects::UndoDepth,
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
        let mut ctrl = Self {
            engine: VimEngine::new(),
            state: ShellState::default(),
            undo_depth: crate::effects::UndoDepth::new(),
            transient: TransientShellState::new(),
            passthrough_keys: HashSet::new(),
            security_policy: SecurityPolicy {
                shell_execution: ShellExecution::Disabled,
                file_access_scope: FileAccessScope::ProjectOnly,
                project_vimrc: crate::settings::ProjectVimrc::Sandbox,
            },
            perf: perf::PerfTracker::new(PERF_RING_CAPACITY, PERF_BUDGET_US),
            highlight_yank_duration_ms: u32::try_from(crate::settings::defaults::HIGHLIGHT_YANK_DURATION).unwrap_or(150),
            code_complete_enabled: true,
        };
        ctrl.engine.set_shadow_execution(true);
        ctrl
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
        snapshot.apply_to_options(self.engine.options_mut());
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
        let mode = self.engine.mode();
        let vs = self.engine.state();
        crate::types::UiSnapshot {
            mode,
            message: self.state.globals().message_status().clone(),
            cmdline: crate::types::CommandLineState {
                prompt: if mode.is_command_line() {
                    Some(vs.command_line().prompt())
                } else {
                    None
                },
                input: CompactString::from(vs.command_line().input()),
                cursor: vs.command_line().cursor(),
            },
            recording_register: vs.macros().recording_register().map(|r| r.char()),
            search_pattern: vs.search().pattern().map(|p| {
                (CompactString::from(p), Direction::from(vs.search().direction()))
            }),
            hlsearch_enabled: self.state.globals().hlsearch_enabled(),
            visual_head: self.state.buffer_ref(editor_id).and_then(|b| b.visual().map(|v| v.head_pos)),
            pending_keys: self.engine.pending_mapping_display(),
            pending_command: self.engine.pending_command_display(),
            substitute_preview: self.state.take_substitute_preview(),
            vimdebug: match (self.transient.vimdebug.provenance().cloned(), self.transient.vimdebug.effects_summary().cloned()) {
                (Some(provenance), Some(effects)) => {
                    match self.transient.vimdebug.step_status_line() {
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
                    }
                }
                // Either field alone (or both absent) means vimdebug is inactive
                // or has not captured anything yet this cycle.
                _ => crate::types::VimdebugSnapshot::Inactive,
            },
            highlight_yank: self.state.take_highlight_yank(),
        }
    }

    // ── Intent-revealing methods ─────────────────────────────────────
    //
    // Narrow, purpose-specific operations that limit the surface area
    // external callers can touch (vs. raw engine_mut()/state_mut()).

    /// Sync indent settings from CodeEdit on attach. CodeEdit is the source
    /// of truth for tab/space and indent widths — not plugin settings.
    pub(crate) fn sync_indent(&mut self, expandtab: bool, shiftwidth: usize, tabstop: usize) {
        let opts = self.engine.options_mut();
        opts.set_expandtab(expandtab);
        opts.set_shiftwidth(shiftwidth);
        opts.set_tabstop(tabstop);
    }

    /// Sync from CodeEdit's language-specific comment delimiters (e.g., `"# %s"`
    /// for GDScript) so the `gc` commentary operator uses the right format.
    pub(crate) fn set_commentstring(&mut self, cs: &str) {
        self.engine.options_mut().set_commentstring(cs);
    }

    /// Sync auto-brace pairs from CodeEdit so the engine handles auto-pairing
    /// during both normal execution and shadow macro replay.
    pub(crate) fn sync_auto_pairs(&mut self, editor: &Gd<godot::classes::CodeEdit>) {
        use vim_core::primitives::{AutoPairs, Pair};

        if !editor.is_auto_brace_completion_enabled() {
            self.engine.options_mut().set_auto_pairs(None);
            return;
        }

        let dict = editor.get_auto_brace_completion_pairs();
        let mut pairs = Vec::new();
        for (k, v) in dict.iter_shared() {
            let open_str = k.to_string();
            let close_str = v.to_string();
            // Engine auto-pairs only supports single-char pairs; skip multi-char
            // (e.g., `/*` / `*/`). Host-side auto-brace handles those.
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

        self.engine
            .options_mut()
            .set_auto_pairs(Some(AutoPairs { pairs: pairs.into() }));
    }

    /// Restore per-buffer engine state for the given editor.
    ///
    /// Retrieves the saved `BufferLocalState` from `BufferState` (or uses
    /// `Default` for first-visit buffers) and calls `engine.on_buffer_enter()`
    /// to restore marks, changelist, last_visual, sticky_column, buffer_overrides,
    /// buffer_mappings, and exchange.
    pub(crate) fn restore_buffer_engine_state(&mut self, editor_id: InstanceId) {
        let state = self.state.buffer(editor_id).take_engine_state()
            .unwrap_or_default();
        self.engine.on_buffer_enter(state);
    }

    /// Seed the undo tree on first attach (no-op if already initialized).
    pub(crate) fn init_undo_tree(&mut self, editor_id: InstanceId, text: &str) {
        let buf = self.state.buffer(editor_id);
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
        let removed = self.state.sweep_invalid_buffers(|id| {
            Gd::<godot::classes::Object>::try_from_instance_id(id).is_ok()
        });
        if !removed.is_empty() {
            log::debug!("sweep_stale_buffers: evicted {} stale buffer(s)", removed.len());
        }
        for id in removed {
            log::debug!("Evicted stale buffer state for editor #{}", id.to_i64());
        }
    }

    #[must_use]
    pub(crate) fn mode(&self) -> vim_core::primitives::Mode {
        self.engine.mode()
    }

    /// Called on `text_changed` signal so the next keystroke fetches fresh
    /// editor text instead of using a stale cache.
    pub(crate) fn invalidate_text_cache(&mut self) {
        self.transient.persistent_text = None;
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
        self.engine.emergency_reset();
        let godot_groups = self.undo_depth.drain();
        self.state.clear_substitute_preview();
        self.state.take_highlight_yank();
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
        self.engine.reset_parser();
    }

    /// Convenience wrapper to reset transient shell state without touching
    /// the engine or undo depth. Used by cleanup paths that handle engine
    /// reset separately (e.g., the normal detach path).
    pub(crate) fn reset_transients(&mut self) {
        self.transient.reset();
    }

    /// Discard any unconsumed yank highlight to prevent cross-editor flash.
    pub(crate) fn clear_highlight_yank(&mut self) {
        self.state.take_highlight_yank();
    }

    /// Save all per-buffer engine state for the current editor.
    ///
    /// Computes the cursor byte offset, calls `engine.on_buffer_leave()` to
    /// extract all per-buffer state (marks, changelist, last_visual, sticky_column,
    /// buffer_overrides, buffer_mappings, exchange), and stores the result in
    /// `BufferState` for later restoration.
    pub(crate) fn save_buffer_engine_state(&mut self, editor_id: InstanceId, editor: &Gd<CodeEdit>) {
        let line = editor.get_caret_line();
        let col = editor.get_caret_column();
        let text = editor.get_text().to_string();
        let line_index = crate::bridge::codec::LineIndex::new(&text);
        let offset = line_index.line_col_to_byte(&text, line, col);
        let engine_state = self.engine.on_buffer_leave(offset);
        self.state.buffer(editor_id).set_engine_state(engine_state);
    }

    /// Exit non-Normal mode by sending a synthetic Escape through the engine
    /// pipeline. This ensures macro recording captures the exit, visual marks
    /// (`<`/`>`), `LastVisualInfo`, insert-stop marks (`^`), and `EndUndo`
    /// effects are all produced — identical to the user pressing Esc.
    ///
    /// Returns `true` if the engine was in a non-Normal mode and Esc was processed.
    pub(crate) fn exit_mode_via_pipeline(&mut self, editor: &mut Gd<CodeEdit>) -> bool {
        if self.engine.mode().is_normal() {
            return false;
        }
        log::debug!("exit_mode_via_pipeline: mode={}", self.engine.mode());
        let mut cx = self.as_process_context();
        cx.process_single_key(KeyEvent::escape(), editor);
        true
    }

    /// Clear Godot-side visual artifacts after the engine has already exited
    /// visual mode via the pipeline. Defense-in-depth: ensures no stale
    /// selection highlights remain even if the pipeline exit was a no-op.
    pub(crate) fn cleanup_visual_artifacts(&mut self, editor_id: InstanceId, editor: &mut Gd<CodeEdit>) {
        self.state.buffer(editor_id).clear_visual_selection();
        editor.remove_secondary_carets();
        editor.deselect();
    }

    /// Drain any remaining undo depth as defense-in-depth after pipeline exit.
    /// The pipeline's `EndUndo` effect handles the normal case; this catches
    /// edge cases where undo groups are still open.
    pub(crate) fn drain_remaining_undo_depth(&mut self, editor: &mut Gd<CodeEdit>) {
        let remaining = self.undo_depth.drain();
        for _ in 0..remaining {
            editor.end_complex_operation();
        }
    }

    /// Force-exit visual/select mode on detach, clearing both engine and
    /// Godot-side selection state to prevent stale highlights.
    pub(crate) fn force_exit_visual(&mut self, editor_id: InstanceId, editor: &mut Gd<CodeEdit>) {
        // Callers must drain pending mapping keys first — a half-consumed
        // multi-key sequence would reference a now-dead selection.
        debug_assert!(
            !self.engine.has_pending_mapping(),
            "force_exit_visual called with pending mapping keys — drain first"
        );

        let mode = self.engine.mode();
        if mode.is_visual() || mode.is_select() {
            log::debug!("force_exit_visual: editor=#{} mode={}", editor_id.to_i64(), mode);

            if self.engine.has_pending_keys() {
                log::warn!("force_exit_visual: aborting pending replay before clearing selection");
                self.engine.abort_replay();
            }

            self.engine.set_mode(vim_core::primitives::Mode::Normal);
            self.state.buffer(editor_id).clear_visual_selection();
            editor.remove_secondary_carets();
            editor.deselect();
        }
    }

    /// Sync sticky column from a mouse click or external caret move.
    /// This is the only shell-side write path; the engine owns the column.
    pub(crate) fn set_engine_sticky_column(&mut self, col: usize) {
        self.engine.set_sticky_column(col);
    }

    #[must_use]
    pub(crate) fn has_pending_mapping(&self) -> bool {
        self.engine.has_pending_mapping()
    }

    #[must_use]
    pub(crate) fn timeoutlen(&self) -> u32 {
        self.engine.timeoutlen()
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
        self.engine.clear_mappings();
        let mut response = self.engine.source_config_text(text);
        let effects = response.take_effects();
        for effect in effects {
            match effect {
                vim_core::effects::Effect::ShowMessage { text: msg } => {
                    crate::effects::messages::handle_show_message(self.state.globals_mut(), &msg);
                }
                vim_core::effects::Effect::ShowError { error } => {
                    crate::effects::messages::handle_show_error(self.state.globals_mut(), &error);
                }
                vim_core::effects::Effect::ClearHighlights => {
                    crate::effects::search::handle_clear_highlights(self.state.globals_mut());
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
        self.state.buffer(editor_id).clear_visual_selection();
        self.state.globals_mut().set_error(
            "Recovered from internal error \u{2014} state reset to Normal mode",
        );
    }

    // ── Processing entry points (delegate to ProcessContext) ─────────

    /// Single entry point for keystroke processing from `gui_input`.
    pub(crate) fn process_cycle(&mut self, key: KeyEvent, editor: &mut Gd<CodeEdit>) -> bool {
        let mut cx = self.as_process_context();
        cx.process_cycle_impl(key, editor)
    }

    /// Force-resolve a pending mapping after timeout, then drain expanded keys.
    pub(crate) fn resolve_mapping_timeout(&mut self, editor: &mut Gd<CodeEdit>) {
        let mut cx = self.as_process_context();
        cx.resolve_mapping_timeout_impl(editor)
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
        let editor_id = editor.instance_id();

        let text = match self.transient.persistent_text.take() {
            Some((id, t)) if id == editor_id => t,
            _ => editor.get_text().to_string(),
        };
        let doc = crate::bridge::document::GodotDocument::new(&text);

        let anchor_offset = doc.line_index().line_col_to_byte(
            doc.text(), anchor_line, anchor_col,
        );
        let head_offset = doc.line_index().line_col_to_byte(
            doc.text(), head_line, head_col,
        );

        let fold_provider = crate::bridge::context::GodotFoldProvider::new(editor);
        let indent_provider = crate::bridge::context::GodotIndentProvider::new(editor);
        let providers = vim_core::document::Providers::new()
            .with_fold(&fold_provider)
            .with_indent(&indent_provider);
        let ctx = crate::bridge::context::build_context(editor, &doc, providers);

        let mut response = self.engine.process_mouse_selection(
            anchor_offset, head_offset, shape, &ctx,
        );

        let effects = response.take_effects();
        if effects.is_empty() {
            self.transient.persistent_text = Some((editor_id, text));
            return false;
        }

        let host_requests = response.take_host_requests();
        {
            let mut cx = self.as_process_context();
            let line_index_hint = Some(doc.into_line_index());
            // Auto-brace ineligible: entering Visual, not Insert.
            cx.apply_effects(effects, editor, crate::effects::dispatch::AutoBraceMode::Ineligible, &text, line_index_hint);

            if !host_requests.is_empty() {
                cx.handle_host_requests(host_requests, editor, 0);
            }
        }

        true
    }

    // ── Process context factory ──────────────────────────────────────

    /// Split `&mut self` into individually-borrowed fields for the processing
    /// pipeline. See [`context::ProcessContext`] for why this is needed.
    fn as_process_context(&mut self) -> context::ProcessContext<'_> {
        context::ProcessContext {
            engine: &mut self.engine,
            state: &mut self.state,
            undo_depth: &mut self.undo_depth,
            persistent_text: &mut self.transient.persistent_text,
            vimdebug: &mut self.transient.vimdebug,
            pending_step_effects: &mut self.transient.pending_step_effects,
            operations_this_cycle: &mut self.transient.operations_this_cycle,
            perf: &mut self.perf,
            pending_ui_action: &mut self.transient.pending_ui_action,
            security_policy: &self.security_policy,
            highlight_yank_duration_ms: self.highlight_yank_duration_ms,
            passthrough_keys: &self.passthrough_keys,
            code_complete_enabled: self.code_complete_enabled,
        }
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
                engine: _,                     // engine: emergency_reset()
                state: _,                      // shell: selective clears
                undo_depth: _,                 // undo: drain()
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
