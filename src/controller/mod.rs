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
    /// Cross-drain runaway guard: catches `:norm` calling back into `drain_pending`.
    operations_this_cycle: u32,
    /// Avoids re-fetching the full document text from Godot (FFI round-trip)
    /// on successive keystrokes that don't mutate text. Tagged with
    /// `InstanceId` to self-invalidate on buffer switch.
    persistent_text: Option<(InstanceId, String)>,
    passthrough_keys: HashSet<KeyEvent>,
    /// Deferred for the plugin layer (scene tree owner) after `process_cycle`.
    pending_ui_action: Option<PendingUiAction>,
    security_policy: SecurityPolicy,
    perf: perf::PerfTracker,
    vimdebug: vimdebug::VimdebugState,
    /// Pass-2 effects deferred by vimdebug step-mode.
    pending_step_effects: Option<Vec<vim_core::effects::Effect>>,
    /// 0 = disabled.
    highlight_yank_duration_ms: u32,
}

impl VimController {
    #[must_use]
    pub(crate) fn new() -> Self {
        Self {
            engine: VimEngine::new(),
            state: ShellState::default(),
            undo_depth: crate::effects::UndoDepth::new(),
            operations_this_cycle: 0,
            persistent_text: None,
            passthrough_keys: HashSet::new(),
            pending_ui_action: None,
            security_policy: SecurityPolicy {
                shell_execution: ShellExecution::Disabled,
                file_access_scope: FileAccessScope::ProjectOnly,
                sandbox_sourced_configs: true,
            },
            perf: perf::PerfTracker::new(PERF_RING_CAPACITY, PERF_BUDGET_US),
            vimdebug: vimdebug::VimdebugState::default(),
            pending_step_effects: None,
            highlight_yank_duration_ms: u32::try_from(crate::settings::defaults::HIGHLIGHT_YANK_DURATION).unwrap_or(150),
        }
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
            sandbox_sourced_configs: snapshot.project_vimrc
                == crate::settings::ProjectVimrc::Sandbox,
        });
        self.set_highlight_yank_duration(snapshot.highlight_yank_duration);
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
            vimdebug: crate::types::VimdebugSnapshot {
                provenance: self.vimdebug.provenance().cloned(),
                effects: self.vimdebug.effects_summary().cloned(),
                range: self.vimdebug.range(),
                step_status: self.vimdebug.step_status_line(),
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

    /// Persist buffer-local mappings to shell state so they survive detach/reattach.
    pub(crate) fn save_buffer_mappings_to_state(&mut self, editor_id: InstanceId) {
        let mappings = self.engine.take_buffer_mappings();
        *self.state.buffer(editor_id).buffer_mappings_mut() = mappings;
    }

    /// Seed the undo tree on first attach (no-op if already initialized).
    pub(crate) fn init_undo_tree(&mut self, editor_id: InstanceId, text: &str) {
        let buf = self.state.buffer(editor_id);
        if buf.undo_tree().is_none() {
            buf.init_undo_tree(text);
        }
    }

    pub(crate) fn restore_buffer_mappings_from_state(&mut self, editor_id: InstanceId) {
        let mappings = self.state.buffer(editor_id).buffer_mappings().clone();
        self.engine.set_buffer_mappings(mappings);
    }

    /// Evict buffer state for editors freed since the last sweep, including
    /// global marks referencing evicted buffers.
    ///
    /// Called from `attach()` — a natural choke point since every editor
    /// switch passes through it. Uses Godot's ObjectDB to probe liveness.
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
        self.persistent_text = None;
    }

    /// Force Normal mode when the editor has been freed but engine mode is stale.
    pub(crate) fn reset_mode_to_normal(&mut self) {
        self.engine.set_mode(vim_core::primitives::Mode::Normal);
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

    /// Force-exit command-line mode on detach.
    ///
    /// Resets the engine to Normal mode and clears the command-line session
    /// so that the substitute preview is cleaned up and the next editor
    /// doesn't inherit a stale command-line prompt.
    pub(crate) fn force_exit_command_line(&mut self) {
        if self.engine.mode().is_command_line() {
            log::debug!("force_exit_command_line: mode={}", self.engine.mode());
            self.engine.set_mode(vim_core::primitives::Mode::Normal);
            // Clear substitute preview so the UI layer doesn't carry stale
            // highlights to the next editor.
            self.state.clear_substitute_preview();
        }
    }

    /// Force-exit insert/replace mode on detach, draining undo groups against
    /// the *departing* editor to prevent begin/end imbalance across tab switches.
    pub(crate) fn force_exit_insert_replace(&mut self, editor: &mut Gd<CodeEdit>) {
        let mode = self.engine.mode();
        if mode.is_insert() || mode.is_replace() {
            log::debug!("force_exit_insert_replace: mode={}", mode);
            self.engine.set_mode(vim_core::primitives::Mode::Normal);
            let godot_groups = self.undo_depth.drain();
            for _ in 0..godot_groups {
                editor.end_complex_operation();
            }
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
        self.operations_this_cycle
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
        self.pending_ui_action.take()
    }

    /// Reset transient state after a caught panic. Treats open undo groups
    /// as a transaction boundary: drain and roll back, so partial text
    /// mutations don't persist. The engine self-corrects on the next
    /// keystroke when it rebuilds `InputContext` from Godot's restored text.
    pub(crate) fn recover_from_panic(&mut self, editor: &mut Gd<CodeEdit>) {
        self.engine.set_mode(vim_core::primitives::Mode::Normal);
        self.persistent_text = None;
        self.pending_step_effects = None;
        self.pending_ui_action = None;
        let godot_groups = self.undo_depth.drain();
        for _ in 0..godot_groups {
            editor.end_complex_operation();
        }

        // One undo() reverses the entire closed undo group atomically.
        if godot_groups > 0 {
            log::warn!(
                "recover_from_panic: rolling back {} closed undo group(s) via editor.undo()",
                godot_groups,
            );
            editor.undo();
        }

        // Resolve and discard any half-consumed mapping (e.g., `j` of
        // `jk`→`<Esc>`) — cannot safely dispatch effects during recovery.
        self.engine.resolve_timeout();
        self.engine.abort_replay();
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

        let text = match self.persistent_text.take() {
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
            self.persistent_text = Some((editor_id, text));
            return false;
        }

        let host_requests = response.take_host_requests();
        {
            let mut cx = self.as_process_context();
            let line_index_hint = Some(doc.into_line_index());
            // Auto-brace ineligible: entering Visual, not Insert.
            cx.apply_effects(effects, editor, false, &text, line_index_hint);

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
            persistent_text: &mut self.persistent_text,
            vimdebug: &mut self.vimdebug,
            pending_step_effects: &mut self.pending_step_effects,
            operations_this_cycle: &mut self.operations_this_cycle,
            perf: &mut self.perf,
            pending_ui_action: &mut self.pending_ui_action,
            security_policy: &self.security_policy,
            highlight_yank_duration_ms: self.highlight_yank_duration_ms,
            passthrough_keys: &self.passthrough_keys,
        }
    }
}
