//! Editor attachment and detachment: signal wiring, pipeline-driven mode exit,
//! per-buffer engine state save/restore, indent/commentstring sync, and UI
//! lifecycle management.

// Promote #[must_use] warnings to errors so that dropping an EngineOutcome
// without calling .apply_ui_update() or .discard() is a compile-time error.
#![deny(unused_must_use)]

use godot::classes::CodeEdit;
use godot::prelude::*;

use crate::bridge::code_edit_ext::CodeEditExt;

use super::outcome::EngineOutcome;
use super::signals::{connect_deferred, connect_immediate, safe_disconnect, SIG_TREE_EXITED};
use super::GodotVimCore;

const SIG_GUI_INPUT: &str = "gui_input";
const SIG_CARET_CHANGED: &str = "caret_changed";
const SIG_VALUE_CHANGED: &str = "value_changed";
const SIG_DRAW: &str = "draw";
const SIG_VISIBILITY_CHANGED: &str = "visibility_changed";
const SIG_MINIMUM_SIZE_CHANGED: &str = "minimum_size_changed";
const SIG_TEXT_CHANGED: &str = "text_changed";
const SIG_TEXT_SET: &str = "text_set";

impl GodotVimCore {
    pub(super) fn attach(&mut self, editor: Gd<CodeEdit>) {
        let new_id = editor.instance_id();

        if self.last_editor_id == Some(new_id) {
            log::trace!("attach: same editor #{}, skipping", new_id.to_i64());
            return;
        }

        // Evict buffer state for editors freed since last attach to prevent
        // unbounded growth of the per-editor buffer map. Safe to call in
        // either attached or detached state — no-ops when detached.
        if let Some(controller) = &mut self.controller {
            if controller.is_attached() {
                log::trace!(
                    "attach: sweeping stale buffers before attaching #{}",
                    new_id.to_i64()
                );
                controller.sweep_stale_buffers();
            }
        }

        self.detach();

        // Store the editor reference BEFORE signal connections so that
        // panic recovery can call detach() to disconnect orphaned signals.
        // Safe: no per-editor signal handler can fire until connections below.
        self.attached_editor = Some(editor.clone());
        self.last_editor_id = Some(new_id);

        let mut editor = editor;

        // Create the VimSession by pairing the detached engine with a new
        // GodotHost wrapping this editor. Must happen before any method
        // that accesses host state (restore_buffer, etc.).
        if let Some(controller) = &mut self.controller {
            controller.attach_session(editor.clone());
        }

        // gui_input MUST be immediate -- deferred delivery would miss the
        // `set_input_as_handled()` window, letting keystrokes leak to Godot.
        let gui_callable = self.base().callable("handle_gui_input");
        connect_immediate(&mut editor, SIG_GUI_INPUT, &gui_callable);

        // caret_changed fires mid-edit, so DEFERRED avoids re-entrancy into
        // the Vim engine while text mutations are in progress.
        let caret_callable = self.base().callable("on_caret_changed");
        connect_deferred(&mut editor, SIG_CARET_CHANGED, &caret_callable);

        // Godot requires the connected handler to exist; the handler is a
        // no-op (engine-handled insert keeps state in sync via effects).
        let text_changed_callable = self.base().callable("on_text_changed");
        connect_deferred(&mut editor, SIG_TEXT_CHANGED, &text_changed_callable);

        // text_set fires when CodeEdit.set_text() is called programmatically
        // (e.g. external script reload, VCS revert). Uses a dedicated handler
        // because set_text() has different semantics: Godot destroys its undo
        // stack, resets the caret to (0,0), and clears selections.
        let text_set_callable = self.base().callable("on_text_set");
        connect_deferred(&mut editor, SIG_TEXT_SET, &text_set_callable);

        if let Some(controller) = &mut self.controller {
            controller.restore_buffer_engine_state(new_id);
        }

        // Comment delimiters are language-specific (# for GDScript, // for C#/shaders).
        if let Some(controller) = &mut self.controller {
            sync_commentstring_from_editor(&editor, controller);
        }

        // Godot is the source of truth for tab/space mode and indent size.
        if let Some(controller) = &mut self.controller {
            sync_indent_from_editor(&editor, controller);
        }

        // Sync auto-brace pairs so the engine handles pairing during both
        // normal execution and shadow macro replay.
        if let Some(controller) = &mut self.controller {
            controller.sync_auto_pairs(&editor);
        }

        self.ui.attach(&mut editor);

        if let Some(ref snapshot) = self.settings {
            let mode = self
                .controller
                .as_ref()
                .map_or(vim_core::primitives::Mode::Normal, |c| c.mode());
            self.ui.apply_settings(snapshot, mode, &mut editor);
        }

        // ── Immediate UI content refresh ─────────────────────────────────
        // At this point the engine state is fully restored (marks, search
        // pattern, mode, hlsearch, etc.) and all overlay nodes exist.
        // Push a full snapshot so the UI is visually correct from frame 1,
        // eliminating the class of "stale UI until first keystroke" bugs
        // (search highlights, status bar mode label, cursor shape, etc.).
        if let Some(controller) = &mut self.controller {
            let snap = controller.ui_snapshot(new_id);
            EngineOutcome::with_snapshot(snap, crate::controller::PipelineOutcome::Passthrough)
                .apply_ui_update(&mut self.ui, &mut editor, &mut self.caret_reconciler);
        }

        // Scrollbar instances are stable across attach/detach -- Godot does not
        // recreate them. If theme changes cause scrollbar recreation, these
        // connections silently break (acceptable: cursor overlay just won't
        // follow scroll until the next full redraw signal).
        let scrollbar_callable = self.base().callable("on_scrollbar_changed");
        if let Some(mut vscroll) = editor.get_v_scroll_bar() {
            connect_deferred(&mut vscroll, SIG_VALUE_CHANGED, &scrollbar_callable);
        }
        if let Some(mut hscroll) = editor.get_h_scroll_bar() {
            connect_deferred(&mut hscroll, SIG_VALUE_CHANGED, &scrollbar_callable);
        }

        // Catch redraws missed by scroll/caret signals: fold/unfold, theme
        // changes, code completion popups, external text edits.
        let draw_callable = self.base().callable("on_editor_draw");
        for signal_name in &[SIG_DRAW, SIG_VISIBILITY_CHANGED, SIG_MINIMUM_SIZE_CHANGED] {
            connect_deferred(&mut editor, signal_name, &draw_callable);
        }

        // Eagerly detect foreign editor free so we don't hold a dangling handle
        // until the next focus event. tree_exited fires synchronously when the
        // editor leaves the scene tree (tab closed / editor freed).
        let tree_exited_callable = self.base().callable("on_attached_editor_tree_exited");
        connect_immediate(&mut editor, SIG_TREE_EXITED, &tree_exited_callable);

        log::debug!("Attached to editor #{}", new_id.to_i64());
    }

    pub(super) fn detach(&mut self) {
        let Some(mut editor) = self.attached_editor.take() else {
            return;
        };

        // Catch ANY panic after the take() so the teardown tail always runs.
        let teardown_ok = crate::safety::panic_guard(
            "detach:teardown",
            || {
                if !editor.is_instance_valid() {
                    log::warn!("detach: editor no longer valid at entry");
                    return false;
                }

                self.cancel_pending_tooltip();

                // Reset so outstanding expectations from this editor don't suppress
                // legitimate caret_changed events on the next editor.
                self.caret_reconciler.reset();

                // Discard any unconsumed yank highlight so it doesn't flash on the
                // next editor's first ui_snapshot(). Matches the substitute_preview
                // pattern (cleared via pipeline exit / force_cleanup).
                if let Some(controller) = &mut self.controller {
                    controller.clear_highlight_yank();
                }

                let editor_id = editor.instance_id();

                // ── Disconnect ALL per-editor signals FIRST ─────────────────────
                //
                // CRITICAL: Deferred signals (caret_changed, text_changed, draw,
                // scrollbar value_changed) must be disconnected BEFORE any operation
                // that could trigger them (exit_mode_via_pipeline, cleanup_visual_-
                // artifacts, etc.).
                //
                // Godot's DEFERRED connection flag enqueues callbacks into the
                // frame's deferred-call queue when the signal is emitted. Crucially,
                // disconnecting a signal does NOT revoke already-enqueued callbacks.
                // If we disconnect AFTER operations that move the cursor or modify
                // text, stale deferred callbacks survive in the queue and fire after
                // attach() has pointed `self.attached_editor` at the NEW editor --
                // causing on_caret_changed_impl to misread the new editor's state
                // (e.g. falsely entering Visual mode if the new editor has a Godot
                // selection).
                //
                // By disconnecting first, the signal emission still occurs
                // internally (Godot doesn't suppress that), but with no connected
                // callable, nothing is enqueued into the deferred queue.
                //
                // gui_input is IMMEDIATE (not deferred) so it has no queue-based
                // staleness issue. However, disconnecting it early is also safe --
                // no new keystrokes arrive during a synchronous detach() call --
                // and keeps all disconnect logic in one cohesive block.

                let gui_callable = self.base().callable("handle_gui_input");
                safe_disconnect(&mut editor, SIG_GUI_INPUT, &gui_callable);

                let caret_callable = self.base().callable("on_caret_changed");
                safe_disconnect(&mut editor, SIG_CARET_CHANGED, &caret_callable);

                let text_changed_callable = self.base().callable("on_text_changed");
                safe_disconnect(&mut editor, SIG_TEXT_CHANGED, &text_changed_callable);

                let text_set_callable = self.base().callable("on_text_set");
                safe_disconnect(&mut editor, SIG_TEXT_SET, &text_set_callable);

                let scrollbar_callable = self.base().callable("on_scrollbar_changed");
                if let Some(mut vscroll) = editor.get_v_scroll_bar() {
                    safe_disconnect(&mut vscroll, SIG_VALUE_CHANGED, &scrollbar_callable);
                }
                if let Some(mut hscroll) = editor.get_h_scroll_bar() {
                    safe_disconnect(&mut hscroll, SIG_VALUE_CHANGED, &scrollbar_callable);
                }

                let draw_callable = self.base().callable("on_editor_draw");
                for signal_name in &[SIG_DRAW, SIG_VISIBILITY_CHANGED, SIG_MINIMUM_SIZE_CHANGED] {
                    safe_disconnect(&mut editor, signal_name, &draw_callable);
                }

                let tree_exited_callable =
                    self.base().callable("on_attached_editor_tree_exited");
                safe_disconnect(&mut editor, SIG_TREE_EXITED, &tree_exited_callable);

                // ── Teardown operations (signal-safe: no callbacks can enqueue) ─

                // Prevent stale pending keys from leaking to the next editor.
                if let Some(timer) = &mut self.mapping_timer {
                    timer.stop();
                }

                // Resolve pending mapping keys BEFORE forcing Normal mode. Order matters:
                // a pending `j` (first key of `jk`->`<Esc>` in Insert mode) would be
                // misinterpreted as a Normal-mode motion if we forced Normal first.
                //
                // Re-validate after resolution because the keystroke pipeline dispatches
                // effects that could theoretically free the editor (tab-close race).
                if let Some(controller) = &mut self.controller {
                    controller.resolve_mapping_timeout(&mut editor);

                    if !editor.is_instance_valid() {
                        log::warn!("detach: editor freed during mapping timeout resolution");
                        controller.clear_multi_cursor_on_detach();
                        return false;
                    }

                    // Exit non-Normal mode via the engine pipeline. This ensures
                    // macro recording captures the Esc, visual marks (</>),
                    // LastVisualInfo, insert-stop marks (^), and EndUndo effects
                    // are all produced -- identical to the user pressing Esc.
                    controller.exit_mode_via_pipeline(&mut editor);

                    if !editor.is_instance_valid() {
                        log::warn!("detach: editor freed during exit_mode_via_pipeline");
                        controller.clear_multi_cursor_on_detach();
                        return false;
                    }

                    // Defense-in-depth: clear Godot-side visual artifacts in case
                    // the pipeline exit left stale selection highlights.
                    controller.cleanup_visual_artifacts(editor_id, &mut editor);

                    // Clear parser state (pending operator like `d`) so it doesn't
                    // leak to the next editor. Macro recording is NOT aborted here --
                    // it is a session-level concept that survives buffer switches.
                    controller.engine_reset_parser();

                    // Reset transient shell state (vimdebug, pending effects, caches).
                    controller.reset_transients();

                    // Clear multi-cursor on buffer leave — multi-cursor state is per-buffer,
                    // not persisted across buffer switches.
                    controller.clear_multi_cursor_on_detach();
                }
                true
            },
            false,
        );

        // ── Single teardown tail (runs unconditionally after the guard) ──
        let still_valid = editor.is_instance_valid();
        if still_valid {
            self.ui.detach(&mut editor);
            if let Some(controller) = &mut self.controller {
                // Decompose the session: drop the GodotHost, reclaim the engine
                // for re-use on the next attach.
                controller.save_buffer_engine_state(editor.instance_id(), &editor);
            }
        } else {
            self.ui.reset_cached_state();
        }
        if let Some(controller) = &mut self.controller {
            if !still_valid || !teardown_ok {
                controller.force_cleanup_without_editor();
            }
            controller.detach_session();
        }
        log::debug!("Detached (teardown_ok={teardown_ok})");
    }
}

/// Sync `commentstring` from CodeEdit's registered comment delimiters.
///
/// Godot returns delimiters as strings: line comments are single tokens (`#`),
/// block comments are space-separated pairs (`/* */`). We filter to line
/// comments only, then pick the **shortest** -- languages like GDScript
/// register both `#` and `##` (doc comment), and we want the regular prefix.
fn sync_commentstring_from_editor(
    editor: &Gd<CodeEdit>,
    controller: &mut crate::controller::VimController,
) {
    let delimiters = editor.get_comment_delimiters();
    let mut best: Option<String> = None;
    for i in 0..delimiters.len() {
        let Some(gstr) = delimiters.get(i) else {
            continue;
        };
        let s = gstr.to_string();
        // Block comments contain a space separator (e.g. "/* */") -- skip.
        if s.contains(' ') {
            continue;
        }
        if best.as_ref().is_none_or(|b| s.len() < b.len()) {
            best = Some(s);
        }
    }
    if let Some(delim) = best {
        let cs = format!("{delim} %s");
        log::debug!(
            "sync_commentstring: '{}' for editor #{}",
            cs,
            editor.instance_id().to_i64()
        );
        controller.set_commentstring(&cs);
    } else {
        log::trace!("sync_commentstring: no line comment delimiter found, keeping default");
    }
}

/// Sync indent settings (expandtab, shiftwidth, tabstop) from CodeEdit.
///
/// Godot is the source of truth for indentation -- overrides GodotVim defaults
/// so `>`, `<`, and auto-indent match the editor's actual behavior.
pub(super) fn sync_indent_from_editor(
    editor: &Gd<CodeEdit>,
    controller: &mut crate::controller::VimController,
) {
    let use_spaces = editor.is_indent_using_spaces();
    let indent_size = editor.safe_indent_size();
    let tab_size = editor.safe_tab_size();

    controller.sync_indent(use_spaces, indent_size, tab_size);

    log::debug!(
        "sync_indent: expandtab={} shiftwidth={} tabstop={} for editor #{}",
        use_spaces,
        indent_size,
        tab_size,
        editor.instance_id().to_i64(),
    );
}
