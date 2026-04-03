//! Editor attachment and detachment: signal wiring, pipeline-driven mode exit,
//! per-buffer engine state save/restore, indent/commentstring sync, and UI
//! lifecycle management.

use godot::classes::CodeEdit;
use godot::prelude::*;

use crate::bridge::code_edit_ext::CodeEditExt;

use super::GodotVimCore;
use super::signals::{connect_deferred, connect_immediate, safe_disconnect};

const SIG_GUI_INPUT: &str = "gui_input";
const SIG_CARET_CHANGED: &str = "caret_changed";
const SIG_VALUE_CHANGED: &str = "value_changed";
const SIG_DRAW: &str = "draw";
const SIG_VISIBILITY_CHANGED: &str = "visibility_changed";
const SIG_MINIMUM_SIZE_CHANGED: &str = "minimum_size_changed";
const SIG_TEXT_CHANGED: &str = "text_changed";

impl GodotVimCore {
    pub(super) fn attach(&mut self, editor: Gd<CodeEdit>) {
        let new_id = editor.instance_id();

        if self.last_editor_id == Some(new_id) {
            log::trace!("attach: same editor #{}, skipping", new_id.to_i64());
            return;
        }

        // Evict buffer state for editors freed since last attach to prevent
        // unbounded growth of the per-editor buffer map.
        if let Some(controller) = &mut self.controller {
            log::trace!("attach: sweeping stale buffers before attaching #{}", new_id.to_i64());
            controller.sweep_stale_buffers();
        }

        self.detach();

        // Store the editor reference BEFORE signal connections so that
        // panic recovery can call detach() to disconnect orphaned signals.
        // Safe: no per-editor signal handler can fire until connections below.
        self.attached_editor = Some(editor.clone());
        self.last_editor_id = Some(new_id);

        let mut editor = editor;

        // gui_input MUST be immediate -- deferred delivery would miss the
        // `set_input_as_handled()` window, letting keystrokes leak to Godot.
        let gui_callable = self.base().callable("handle_gui_input");
        connect_immediate(&mut editor, SIG_GUI_INPUT, &gui_callable);

        // caret_changed fires mid-edit, so DEFERRED avoids re-entrancy into
        // the Vim engine while text mutations are in progress.
        let caret_callable = self.base().callable("on_caret_changed");
        connect_deferred(&mut editor, SIG_CARET_CHANGED, &caret_callable);

        // Catches external text changes (Find-and-Replace, plugins, auto-format)
        // that bypass the Vim keystroke pipeline. Vim-driven edits already
        // invalidate the cache inline, so this is harmless but necessary for
        // external edits.
        let text_changed_callable = self.base().callable("on_text_changed");
        connect_deferred(&mut editor, SIG_TEXT_CHANGED, &text_changed_callable);

        if let Some(controller) = &mut self.controller {
            controller.restore_buffer_engine_state(new_id);
        }

        // Seed the undo tree with the editor's current text so that the first
        // undo operation has a base snapshot to diff against.
        if let Some(controller) = &mut self.controller {
            let text = editor.get_text().to_string();
            controller.init_undo_tree(new_id, &text);
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

        log::debug!("Attached to editor #{}", new_id.to_i64());
    }

    pub(super) fn detach(&mut self) {
        let Some(mut editor) = self.attached_editor.take() else {
            return;
        };

        // Brace pair cache is per-language; stale pairs from the old editor's
        // language would produce wrong results in the new editor.
        crate::bridge::port_impl::invalidate_brace_pair_cache();

        // Reset so outstanding suppressions from this editor don't swallow
        // legitimate caret_changed events on the next editor.
        self.pending_caret_suppressions = 0;

        // Discard any unconsumed yank highlight so it doesn't flash on the
        // next editor's first ui_snapshot(). Matches the substitute_preview
        // pattern (cleared via pipeline exit / force_cleanup).
        if let Some(controller) = &mut self.controller {
            controller.clear_highlight_yank();
        }

        if !editor.is_instance_valid() {
            log::warn!("detach: editor no longer valid, skipping cleanup");
            if let Some(controller) = &mut self.controller {
                controller.force_cleanup_without_editor();
            }
            self.ui.reset_cached_state();
            return;
        }

        let editor_id = editor.instance_id();

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
                controller.force_cleanup_without_editor();
                self.ui.reset_cached_state();
                return;
            }

            // Exit non-Normal mode via the engine pipeline. This ensures
            // macro recording captures the Esc, visual marks (</>),
            // LastVisualInfo, insert-stop marks (^), and EndUndo effects
            // are all produced — identical to the user pressing Esc.
            controller.exit_mode_via_pipeline(&mut editor);

            // Defense-in-depth: clear Godot-side visual artifacts in case
            // the pipeline exit left stale selection highlights.
            controller.cleanup_visual_artifacts(editor_id, &mut editor);

            // Defense-in-depth: drain any remaining undo depth in case
            // the pipeline's EndUndo didn't close all open groups.
            controller.drain_remaining_undo_depth(&mut editor);

            // Clear parser state (pending operator like `d`) so it doesn't
            // leak to the next editor. Macro recording is NOT aborted here —
            // it is a session-level concept that survives buffer switches.
            controller.engine_reset_parser();

            // Reset transient shell state (vimdebug, pending effects, caches).
            controller.reset_transients();
        }

        self.ui.detach(&mut editor);

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

        if let Some(controller) = &mut self.controller {
            controller.save_buffer_engine_state(editor_id, &editor);
        }

        let gui_callable = self.base().callable("handle_gui_input");
        safe_disconnect(&mut editor, SIG_GUI_INPUT, &gui_callable);

        let caret_callable = self.base().callable("on_caret_changed");
        safe_disconnect(&mut editor, SIG_CARET_CHANGED, &caret_callable);

        let text_changed_callable = self.base().callable("on_text_changed");
        safe_disconnect(&mut editor, SIG_TEXT_CHANGED, &text_changed_callable);

        log::debug!("Detached from editor #{}", editor_id.to_i64());
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
        let Some(gstr) = delimiters.get(i) else { continue };
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
        log::debug!("sync_commentstring: '{}' for editor #{}", cs, editor.instance_id().to_i64());
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
        use_spaces, indent_size, tab_size, editor.instance_id().to_i64(),
    );
}
