//! Main keystroke processing pipeline: the path every key takes from Godot's
//! `gui_input` callback through the Vim engine and back out as editor mutations.
//!
//! Key flow:
//! ```text
//! gui_input → process_cycle_impl
//!   ├─ completion interception (pre-engine)
//!   ├─ passthrough check
//!   ├─ process_single_key
//!   │    ├─ Stage 1: build InputContext (text, cursor, fold/indent providers)
//!   │    ├─ Stage 2: engine.process(key, ctx) → Response
//!   │    ├─ Stage 3: apply_effects → Godot mutations
//!   │    └─ Stage 4: handle_host_requests → sub-effects → recurse
//!   ├─ drain_pending (macro replay / mapping expansion)
//!   ├─ ensure_undo_balanced
//!   └─ completion re-trigger (post-engine)
//! ```

use compact_str::CompactString;
use godot::classes::{CodeEdit, DisplayServer};
use godot::prelude::*;
use vim_core::document::Providers;
use vim_core::execution::MacroOutput;
use vim_core::keymap::KeyEvent;
use vim_core::primitives::SelectionRange;

use super::completion;
use super::context::ProcessContext;
use super::perf;
use super::MAX_DRAIN_ITERATIONS;
use crate::bridge;
use crate::bridge::document::GodotDocument;
use crate::bridge::port_impl::CodeEditPort;

impl ProcessContext<'_> {
    /// Single entry point from `gui_input`. Returns `true` if Vim consumed
    /// the key (Godot should not process it), `false` to pass through.
    ///
    /// Guarantees undo group balance on return via `ensure_undo_balanced`.
    pub(super) fn process_cycle_impl(&mut self, key: KeyEvent, editor: &mut Gd<CodeEdit>) -> bool {
        *self.operations_this_cycle = 0;

        // Messages are one-shot: displayed after the producing keystroke,
        // cleared on the next. Mirrors vim-core's clear_transient().
        self.state.globals_mut().clear_message();

        // Step mode intercepts all keys for the effect inspector (n/p/c/q).
        if self.vimdebug.is_step_mode() && self.pending_step_effects.is_some() {
            return self.process_step_key(key, editor);
        }

        if let Some(consumed) = completion::try_handle_completion(self.engine, key, editor) {
            log::debug!("process_cycle: completion intercepted key={} consumed={}", key, consumed);
            *self.persistent_text = None;
            self.ensure_undo_balanced(editor);
            return consumed;
        }

        if self.should_passthrough_key(key) {
            log::debug!("process_cycle: passthrough key={} mode={}", key, self.engine.mode());
            return false;
        }

        let consumed = self.process_single_key(key, editor);
        self.drain_pending(editor);
        self.ensure_undo_balanced(editor);

        completion::maybe_retrigger_completion(self.engine, key, editor, self.code_complete_enabled);

        consumed
    }

    /// Build context, run engine, dispatch effects for one keystroke.
    /// Called both from `process_cycle` (user input) and `drain_pending`
    /// (macro replay / mapping expansion).
    pub(super) fn process_single_key(&mut self, key: KeyEvent, editor: &mut Gd<CodeEdit>) -> bool {
        let total_start = std::time::Instant::now();
        *self.operations_this_cycle = self.operations_this_cycle.saturating_add(1);
        self.vimdebug.clear_captures();
        log::trace!("process_single_key: key={} operations_this_cycle={}", key, *self.operations_this_cycle);

        // Capture pre-processing state for the per-keystroke debug summary.
        let mode_before = self.engine.mode();
        let cursor_before = (editor.get_caret_line(), editor.get_caret_column());

        // ── Stage 1: Context build ───────────────────────────────────────
        let ctx_start = std::time::Instant::now();
        let editor_id = editor.instance_id();

        let text = match self.persistent_text.take() {
            Some((id, t)) if id == editor_id => t,
            _ => editor.get_text().to_string(),
        };
        let doc = GodotDocument::new(&text);

        // Providers must outlive the InputContext that borrows them.
        let fold_provider = bridge::context::GodotFoldProvider::new(editor);
        let indent_provider = bridge::context::GodotIndentProvider::new(editor);
        let providers = Providers::new()
            .with_fold(&fold_provider)
            .with_indent(&indent_provider);
        let mut ctx = bridge::context::build_context(editor, &doc, providers);

        // Use shell-owned selection instead of Godot's — Godot round-trips
        // lose collapsed selections, corrupt line-mode, and flatten block-mode.
        if let Some(vs) = self.state.buffer(editor_id).visual() {
            ctx = ctx.with_selection(SelectionRange::new(vs.anchor, vs.head));
        }

        let ctx_elapsed = ctx_start.elapsed();

        // ── Stage 2: Engine process ──────────────────────────────────────
        let eng_start = std::time::Instant::now();

        let mut response = self.engine.process(key, ctx);
        let consumed = response.consumed();

        // Capture command name for the debug summary before vimdebug consumes it.
        let cmd_name: CompactString = match response.provenance() {
            Some(p) => CompactString::from(p.command_name()),
            None => match response.kind() {
                vim_core::execution::ResponseKind::Pending => CompactString::new_inline("Pending"),
                vim_core::execution::ResponseKind::Ignored => CompactString::new_inline("Ignored"),
                vim_core::execution::ResponseKind::Consumed => CompactString::new_inline("(no provenance)"),
                _ => CompactString::new_inline("Unknown"),
            },
        };

        self.vimdebug.capture_provenance(
            response.provenance().map(|p| p.command_name()),
        );

        let eng_elapsed = eng_start.elapsed();

        // ── Stage 3: Effects dispatch ────────────────────────────────────
        let fx_start = std::time::Instant::now();

        let effects = response.take_effects();
        let effects_count = effects.len();
        let has_text_mutation = effects.iter().any(|e| e.is_text_mutation());

        self.vimdebug_capture(&effects, &text, &doc);

        let had_compound_actions =
            if self.vimdebug.is_step_mode() && !effects.is_empty() {
                match self.vimdebug_step_split(effects, &mut response, editor, &text) {
                    Some(result) => result,
                    None => {
                            return consumed;
                    }
                }
            } else {
                // Auto-brace is safe: dot-repeat completes within process() and
                // returns to Normal, so is_insert() is false during replay.
                let auto_brace = if self.engine.mode().is_insert() {
                    crate::effects::dispatch::AutoBraceMode::Eligible
                } else {
                    crate::effects::dispatch::AutoBraceMode::Ineligible
                };

                // Reuse the pre-built line index when no text mutations will
                // invalidate it; otherwise let dispatch rebuild after mutations.
                let line_index_hint = if has_text_mutation {
                    None
                } else {
                    Some(doc.into_line_index())
                };
                self.apply_effects(effects, editor, auto_brace, &text, line_index_hint)
            };

        let host_requests = response.take_host_requests();
        let has_host_requests = !host_requests.is_empty();
        self.handle_host_requests(host_requests, editor, 0);

        let fx_elapsed = fx_start.elapsed();

        let total_elapsed = total_start.elapsed();
        self.perf.record(perf::FrameMetrics {
            context_build_us: perf::Microseconds(u64::try_from(ctx_elapsed.as_micros()).unwrap_or(u64::MAX)),
            engine_process_us: perf::Microseconds(u64::try_from(eng_elapsed.as_micros()).unwrap_or(u64::MAX)),
            effects_dispatch_us: perf::Microseconds(u64::try_from(fx_elapsed.as_micros()).unwrap_or(u64::MAX)),
            ui_update_us: perf::Microseconds(0),
            total_us: perf::Microseconds(u64::try_from(total_elapsed.as_micros()).unwrap_or(u64::MAX)),
        });

        // Compound actions and host requests can mutate text outside the
        // original effect list, so they also invalidate the cache.
        let cache_restored = !has_text_mutation && !has_host_requests && !had_compound_actions;
        if cache_restored {
            *self.persistent_text = Some((editor_id, text));
        }

        // ── IME lifecycle ─────────────────────────────────────────────────
        //
        // Activate the OS input method when entering Insert/Replace so CJK
        // input works; deactivate it when returning to Normal so keystrokes
        // aren't intercepted by the IME.
        {
            let mode_after = self.engine.mode();
            if mode_after != mode_before {
                let was_insert_like = mode_before.is_insert() || mode_before.is_replace();
                let is_insert_like = mode_after.is_insert() || mode_after.is_replace();
                if !was_insert_like && is_insert_like {
                    activate_ime(editor);
                } else if was_insert_like && !is_insert_like {
                    deactivate_ime(editor);
                }
            }
        }

        // ── Per-keystroke DEBUG summary ──────────────────────────────────
        //
        // One line that tells the complete story of what happened:
        //   [key] k  Normal  cmd=Motion(Up)  cursor=10:0→9:0  effects=2  259µs
        if log::log_enabled!(target: "key", log::Level::Debug) {
            use std::fmt::Write;
            let mut summary = String::with_capacity(128);
            let _ = write!(summary, "{}  {}  cmd={}", key, mode_before, cmd_name);

            let cursor_after = (editor.get_caret_line(), editor.get_caret_column());
            if cursor_after != cursor_before {
                let _ = write!(
                    summary,
                    "  cursor={}:{}\u{2192}{}:{}",
                    cursor_before.0, cursor_before.1,
                    cursor_after.0, cursor_after.1,
                );
            }

            if has_text_mutation {
                summary.push_str("  text_mutated");
            }

            let mode_after = self.engine.mode();
            if mode_after != mode_before {
                let _ = write!(summary, "  mode\u{2192}{}", mode_after);
            }

            let _ = write!(
                summary,
                "  effects={}  {}\u{00b5}s",
                effects_count,
                total_elapsed.as_micros(),
            );

            log::debug!(target: "key", "{}", summary);
        }

        consumed
    }

    /// Build vimdebug annotations: up to 5 non-internal effect kinds as a
    /// summary string, plus the line/col range of the first Delete/Replace
    /// for the debug overlay highlight.
    fn vimdebug_capture(
        &mut self,
        effects: &[vim_core::effects::Effect],
        text: &str,
        doc: &GodotDocument,
    ) {
        if !self.vimdebug.is_enabled() || effects.is_empty() {
            return;
        }

        use std::fmt::Write;
        let mut summary = String::with_capacity(128);
        for (i, e) in effects
            .iter()
            .filter(|e| e.tier() != vim_core::effects::EffectTier::Internal)
            .take(5)
            .enumerate()
        {
            if i > 0 { summary.push_str(", "); }
            let _ = write!(summary, "{:?}", e.kind());
        }
        if !summary.is_empty() {
            self.vimdebug.capture_effects_summary(CompactString::from(summary));
        }

        let li = doc.line_index();
        let range = effects.iter().find_map(|e| {
            let r = match e {
                vim_core::effects::Effect::Delete { range } => range,
                vim_core::effects::Effect::Replace { range, .. } => range,
                _ => return None,
            };
            let start_lc = li.byte_to_line_col(text, r.start().get());
            let end_lc = li.byte_to_line_col(text, r.end().get());
            Some(crate::types::MatchRange::new(start_lc, end_lc))
        });
        self.vimdebug.capture_range(range);
    }

    /// Split effects for step-mode: apply text mutations (pass 1) immediately,
    /// defer cursor/UI effects (pass 2) for interactive inspection.
    ///
    /// Returns `Some(had_compounds)` if no effects were deferred, `None` if
    /// pass-2 effects are pending (caller should return early).
    fn vimdebug_step_split(
        &mut self,
        effects: Vec<vim_core::effects::Effect>,
        response: &mut vim_core::execution::Response,
        editor: &mut Gd<CodeEdit>,
        text_ref: &str,
    ) -> Option<bool> {
        let (pass1, pass2) = crate::effects::dispatch::split_effects_by_pass(effects);

        let pass1_had_compounds = if !pass1.is_empty() {
            let auto_brace = if self.engine.mode().is_insert() {
                crate::effects::dispatch::AutoBraceMode::Eligible
            } else {
                crate::effects::dispatch::AutoBraceMode::Ineligible
            };
            let result = self.apply_effects(pass1, editor, auto_brace, text_ref, None);
            *self.persistent_text = None;
            result
        } else {
            false
        };

        if !pass2.is_empty() {
            let descriptions: Vec<compact_str::CompactString> = pass2
                .iter()
                .map(|e| compact_str::format_compact!("{:?}", e.kind()))
                .collect();
            self.vimdebug.load_step_effects(descriptions);
            *self.pending_step_effects = Some(pass2);
            let host_requests = response.take_host_requests();
            if !host_requests.is_empty() {
                self.handle_host_requests(host_requests, editor, 0);
            }
            return None;
        }

        Some(pass1_had_compounds)
    }

    /// Vimdebug step-mode key handler: n=next, p=prev, c=continue, q=quit.
    /// All keys are consumed while stepping.
    fn process_step_key(
        &mut self,
        key: KeyEvent,
        editor: &mut Gd<CodeEdit>,
    ) -> bool {
        let scrolloff = self.scrolloff();
        let editor_id = editor.instance_id();
        let ch = key.as_char();

        match ch {
            Some('n') => {
                if let Some(idx) = self.vimdebug.step_next() {
                    if let Some(ref effects) = self.pending_step_effects {
                        if idx < effects.len() {
                            let effect = effects[idx].clone();
                            self.apply_step_effect(effect, editor, editor_id, scrolloff);
                        }
                    }
                }
                if !self.vimdebug.has_pending_steps() {
                    self.vimdebug.step_quit();
                    *self.pending_step_effects = None;
                }
            }
            Some('p') => {
                self.vimdebug.step_prev();
            }
            Some('c') => {
                let remaining = self.vimdebug.step_continue();
                let mut all_effects = self.pending_step_effects.take().unwrap_or_default();
                let remaining_set: std::collections::HashSet<usize> = remaining.into_iter().collect();
                let to_apply: Vec<vim_core::effects::Effect> = all_effects
                    .drain(..)
                    .enumerate()
                    .filter_map(|(i, e)| remaining_set.contains(&i).then_some(e))
                    .collect();
                for effect in to_apply {
                    self.apply_step_effect(effect, editor, editor_id, scrolloff);
                }
                self.vimdebug.step_quit();
            }
            Some('q') => {
                self.vimdebug.step_quit();
                *self.pending_step_effects = None;
            }
            _ => {} // Consume all other keys while stepping
        }
        true
    }

    /// Apply a single deferred pass-2 effect in step mode.
    ///
    /// `SetSelection`/`ClearSelection` are handled inline because step mode
    /// applies effects one at a time (no `SelectionPairing` state machine).
    /// Compound actions are intentionally discarded — step mode is a debug tool.
    fn apply_step_effect(
        &mut self,
        effect: vim_core::effects::Effect,
        editor: &mut Gd<CodeEdit>,
        editor_id: InstanceId,
        scrolloff: i32,
    ) {
        use vim_core::effects::Effect;

        let text = editor.get_text().to_string();
        let li = crate::bridge::codec::LineIndex::new(&text);
        let doc = crate::bridge::codec::DocumentView::new(&text, &li);

        match effect {
            Effect::SetSelection { anchor, head, shape } => {
                let mut port = CodeEditPort(editor);
                crate::effects::cursor::handle_set_selection(&mut port, &doc, anchor.get(), head.get(), shape);
                let head_pos = doc.line_index.byte_to_line_col(doc.text, head.get());
                self.state.buffer(editor_id).update_visual_selection(anchor, head, head_pos);
            }
            Effect::ClearSelection => {
                let mut port = CodeEditPort(editor);
                crate::effects::cursor::handle_clear_selection(&mut port);
                self.state.buffer(editor_id).clear_visual_selection();
            }
            other => {
                let mut compound_actions = Vec::new();
                {
                    let mut port = CodeEditPort(editor);
                    crate::effects::dispatch::dispatch_pass2_effect(
                        other,
                        &mut port,
                        self.state,
                        &doc,
                        &mut compound_actions,
                        scrolloff,
                        self.highlight_yank_duration_ms,
                        self.clipboard,
                    );
                }
            }
        }
    }

    pub(super) fn drain_pending(&mut self, editor: &mut Gd<CodeEdit>) {
        let mut iterations: u32 = 0;
        while let Some(output) = self.engine.drain_next_key() {
            // Defense-in-depth: abort if the editor was freed mid-replay.
            // Godot's single-threaded model makes this unreachable in normal
            // operation, but it provides an independent safety layer against
            // future changes or unexpected Godot behavior.
            if !editor.is_instance_valid() {
                log::warn!("drain_pending: editor freed mid-replay, aborting");
                self.engine.abort_replay();
                return;
            }
            iterations += 1;
            if iterations > MAX_DRAIN_ITERATIONS || *self.operations_this_cycle > MAX_DRAIN_ITERATIONS {
                log::error!(
                    "Drain exceeded {} iterations (local={}, cycle={}, last_output={:?}) — \
                     aborting replay to break potential infinite loop",
                    MAX_DRAIN_ITERATIONS,
                    iterations,
                    *self.operations_this_cycle,
                    output,
                );
                self.state.globals_mut().set_error(
                    "E223: Mapping/macro replay exceeded iteration limit — aborted",
                );
                self.engine.abort_replay();
                return;
            }
            match output {
                MacroOutput::Key(key) => {
                    self.process_single_key(key, editor);
                }
                MacroOutput::TextBlock { text, cursor_offset } => {
                    self.apply_text_block(&text, cursor_offset, editor);
                }
            }
        }
    }

    /// Insert a text block directly at the cursor, bypassing insert dispatch.
    /// Used for completion text, paste, and IME input during macro replay.
    fn apply_text_block(&mut self, text: &str, cursor_offset: usize, editor: &mut Gd<CodeEdit>) {
        let line = editor.get_caret_line();
        let col = editor.get_caret_column();

        // Insert the text at the caret. Godot's insert_text_at_caret moves the
        // caret to the end of the inserted text after the call.
        editor.insert_text_at_caret(&GString::from(text));

        // Position cursor at the correct offset within the inserted text.
        // cursor_offset is a byte offset relative to the insertion start.
        //
        // After insertion, the buffer contains the new text. We rebuild the
        // line index on the post-insertion text and compute the target position:
        //   target_byte = (caret_byte_after_insert - text.len()) + cursor_offset
        // because the caret is at the END of the inserted text.
        let full_text = editor.get_text().to_string();
        let line_index = crate::bridge::codec::LineIndex::new(&full_text);

        let caret_after = line_index.line_col_to_byte(
            &full_text,
            editor.get_caret_line(),
            editor.get_caret_column(),
        );
        let target_byte = caret_after.saturating_sub(text.len()) + cursor_offset;
        let target_pos = line_index.byte_to_line_col(&full_text, target_byte);

        editor.set_caret_line(target_pos.line);
        editor.set_caret_column(target_pos.col);

        // Invalidate the text cache since we mutated text.
        *self.persistent_text = None;
        *self.operations_this_cycle = self.operations_this_cycle.saturating_add(1);

        log::debug!(
            "apply_text_block: inserted {}b at ({},{}) cursor_offset={} -> ({},{})",
            text.len(), line, col, cursor_offset, target_pos.line, target_pos.col
        );
    }

    /// Returns `true` if compound actions were generated, which means text
    /// may have been mutated outside the original effect list (cache-invalidating).
    pub(super) fn apply_effects(
        &mut self,
        effects: Vec<vim_core::effects::Effect>,
        editor: &mut Gd<CodeEdit>,
        auto_brace: crate::effects::dispatch::AutoBraceMode,
        text_ref: &str,
        line_index_hint: Option<crate::bridge::codec::LineIndex>,
    ) -> bool {
        if effects.is_empty() {
            return false;
        }

        // Strip substitute preview effects during macro replay / dot-repeat.
        // Intermediate `:s` commands in a macro produce preview effects that
        // would flicker the overlay with no user benefit — the macro finishes
        // within the same drain_pending loop.
        let effects = if self.engine.has_pending_keys() || self.engine.is_repeating() {
            effects
                .into_iter()
                .filter(|e| {
                    !matches!(
                        e,
                        vim_core::effects::Effect::SubstitutePreview { .. }
                            | vim_core::effects::Effect::ClearSubstitutePreview
                    )
                })
                .collect()
        } else {
            effects
        };

        if effects.is_empty() {
            return false;
        }

        let editor_id = editor.instance_id();
        let auto_brace_eligible = matches!(auto_brace, crate::effects::dispatch::AutoBraceMode::Eligible);
        let mut auto_brace_snapshot = if auto_brace_eligible {
            bridge::AutoBraceSnapshot::from_editor(editor)
        } else {
            bridge::AutoBraceSnapshot::disabled()
        };
        // When vim-core owns single-char auto-pairs, remove them from the
        // host-side auto-brace pair list so the two systems operate on
        // disjoint sets and can never conflict (issue #20).
        if self.engine.options().auto_pairs().is_some() {
            auto_brace_snapshot.filter_engine_owned_pairs();
        }
        let compound_actions = {
            // Cheap Gd clone for the syntax closure; the original `editor` is
            // borrowed mutably by CodeEditPort simultaneously.
            let editor_for_syntax = editor.clone();
            let mut port = CodeEditPort(editor);
            crate::effects::dispatch(
                effects,
                &mut port,
                crate::effects::DispatchContext {
                    state: self.state,
                    editor_id,
                    undo_depth: self.undo_depth,
                    auto_brace,
                    auto_brace_snapshot,
                    line_index_hint,
                    scrolloff: crate::bridge::codec::usize_to_i32(self.engine.options().scrolloff()),
                    highlight_yank_duration_ms: self.highlight_yank_duration_ms,
                    syntax_query: Box::new(move |line, col| {
                        bridge::SyntaxRegion::from_editor(&editor_for_syntax, line, col)
                    }),
                    clipboard: self.clipboard,
                },
                text_ref,
            )
        };

        let had_compounds = !compound_actions.is_empty();
        for action in compound_actions {
            self.process_compound_action(action, editor);
        }

        had_compounds
    }

    // ── Undo safety ───────────────────────────────────────────────────

    /// Close any orphaned `begin_complex_operation` calls left open by
    /// a bug or panic. Insert/Replace legitimately hold depth=1 across
    /// keystrokes (opened on mode entry, closed on Esc); depth>1 is a bug.
    pub(super) fn ensure_undo_balanced(&mut self, editor: &mut Gd<CodeEdit>) {
        let mode = self.engine.mode();

        if mode.is_insert() || mode.is_replace() {
            let depth = self.undo_depth.depth();
            if depth > 1 {
                log::error!(
                    "Abnormal undo depth {} in {} mode (expected 1) editor=#{} — engine bug?",
                    depth, mode, editor.instance_id().to_i64(),
                );
            }
            return;
        }

        let godot_groups = self.undo_depth.drain();
        if godot_groups > 0 {
            self.state.globals_mut().set_error(
                "Internal: orphaned undo group(s) recovered — undo may be inconsistent",
            );
        }
        for i in 0..godot_groups {
            log::warn!("Closing orphaned undo group ({}/{})", i + 1, godot_groups);
            editor.end_complex_operation();
        }
    }

    // ── Key passthrough ──────────────────────────────────────────────
    //
    // Decision order: mappings beat everything, then user overrides,
    // then host policy (F-keys, Alt/Meta), then the engine's own judgment.

    fn should_passthrough_key(&self, key: KeyEvent) -> bool {
        // Mappings always take priority — never passthrough mid-sequence.
        if self.engine.has_pending_mapping() || self.engine.could_start_mapping(key) {
            return false;
        }

        if self.passthrough_keys.contains(&key) {
            return true;
        }

        if super::passthrough::is_always_passthrough(key) {
            return true;
        }

        // Final arbiter: does the engine's built-in command set handle this key?
        !self.engine.would_handle_key(key)
    }

    // ── Helpers ──────────────────────────────────────────────────────

    fn scrolloff(&self) -> i32 {
        crate::bridge::codec::usize_to_i32(self.engine.options().scrolloff())
    }

    pub(super) fn resolve_mapping_timeout_impl(&mut self, editor: &mut Gd<CodeEdit>) {
        log::debug!("resolve_mapping_timeout: resolving pending mapping");
        // Reset cycle counter so stale values from the previous process_cycle
        // don't trip the runaway guard during this timeout-driven drain.
        *self.operations_this_cycle = 0;
        *self.persistent_text = None;
        self.engine.resolve_timeout();
        self.drain_pending(editor);
        self.ensure_undo_balanced(editor);
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
