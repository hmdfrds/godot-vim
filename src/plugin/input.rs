//! Input handler implementations for the two Godot entry points: global
//! `input()` (cross-panel/dock navigation) and per-editor `gui_input()`
//! (keystroke processing through the Vim engine).

// Promote #[must_use] warnings to errors so that dropping an EngineOutcome
// without calling .apply_ui_update() or .discard() is a compile-time error.
#![deny(unused_must_use)]

use godot::classes::{CodeEdit, EditorInterface, InputEvent, InputEventKey};
use godot::global::Key;
use godot::prelude::*;

use crate::bridge;
use crate::controller::VimController;
use crate::navigation::{self, classify_focus, FocusContext};
use crate::ui::UiCoordinator;

use super::outcome::EngineOutcome;
use super::processing_guard::ProcessingKeyGuard;
use super::GodotVimCore;

impl GodotVimCore {
    /// Global `input()` handler (Godot stage 1 -- fires before `gui_input`).
    ///
    /// Intercepts two categories before they reach the Vim engine or native controls:
    /// 1. **Cross-panel navigation** (Ctrl+hjkl) -- consumed from all contexts
    ///    except Foreign text input, so Godot never handles these keys natively.
    /// 2. **Dock/search navigation** (j/k/h/l/Enter/Esc) -- only when focus is
    ///    on a navigable dock or search box.
    ///
    /// Editor-context keys fall through to the per-editor `gui_input` pipeline.
    pub(super) fn handle_input_impl(&mut self, event: Gd<InputEvent>) {
        if !self.enabled {
            return;
        }
        let Ok(key_event) = event.try_cast::<InputEventKey>() else {
            return;
        };
        if !key_event.is_pressed() {
            return;
        }

        let keycode = key_event.get_keycode();
        if matches!(
            keycode,
            Key::SHIFT
                | Key::CTRL
                | Key::ALT
                | Key::META
                | Key::CAPSLOCK
                | Key::NUMLOCK
                | Key::SCROLLLOCK
        ) {
            return;
        }

        let Some(base_control) = EditorInterface::singleton().get_base_control() else {
            return;
        };
        let Some(mut viewport) = base_control.get_viewport() else {
            return;
        };

        // Phase 1: decide, holding only an immutable borrow of self.attached_editor.
        let (attached_id, stale) = match self.attached_editor.as_ref() {
            Some(e) if e.is_instance_valid() => (Some(e.instance_id()), false),
            Some(_) => (None, true), // editor freed externally (e.g. a foreign addon closed its view)
            None => (None, false),
        };
        // Phase 2: borrow released — now safe to take &mut self.
        if stale {
            // Deref-free self-heal: drop the stale handle. detach() is self-completing (Task 3).
            self.detach();
            self.last_editor_id = None;
        }
        let context = classify_focus(&viewport, attached_id);

        // Consume Ctrl+hjkl for cross-panel navigation, with mode awareness:
        // - Foreign: never intercept (user is typing in a non-Vim text input)
        // - Editor in Insert/Replace/CommandLine/Select: don't intercept —
        //   Ctrl+H=backspace, Ctrl+J=newline, Ctrl+K=digraph are Vim bindings
        // - Editor in Normal/Visual/OP: intercept — no Vim Ctrl+hjkl bindings
        // - Dock/Search/Unknown: always intercept
        let is_ctrl_only = key_event.is_ctrl_pressed()
            && !key_event.is_alt_pressed()
            && !key_event.is_meta_pressed()
            && !key_event.is_shift_pressed();
        let should_intercept_hjkl = is_ctrl_only
            && match context {
                FocusContext::Foreign => false,
                FocusContext::Editor => {
                    self.controller.as_ref().is_none_or(|c| {
                        let mode = c.mode();
                        let is_nav_mode = matches!(
                            mode,
                            vim_core::primitives::Mode::Normal
                                | vim_core::primitives::Mode::Visual(_)
                                | vim_core::primitives::Mode::OperatorPending(_)
                        );
                        // Select mode intentionally excluded — it's insert-like,
                        // so Ctrl+H/J/K/L should reach the engine (backspace,
                        // newline, etc.), not navigate panels.
                        if !is_nav_mode {
                            return false;
                        }
                        // User mappings take priority over panel navigation.
                        // If the mapping trie has an entry for this Ctrl+hjkl key,
                        // let it flow through to gui_input where the mapping system
                        // handles it.
                        let vim_key = vim_core::keymap::KeyEvent::ctrl(match keycode {
                            Key::H => 'h',
                            Key::J => 'j',
                            Key::K => 'k',
                            Key::L => 'l',
                            _ => return false, // not hjkl — don't intercept
                        });
                        !c.could_start_mapping(vim_key)
                    })
                }
                FocusContext::Dock(..) | FocusContext::SearchBox(..) | FocusContext::Unknown => {
                    true
                }
            };
        if should_intercept_hjkl {
            let physical = key_event.get_physical_keycode();
            if let Some(direction) = navigation::window::direction_from_hjkl(keycode, physical) {
                if let Some(focus_owner) = viewport.gui_get_focus_owner() {
                    let control: Gd<godot::classes::Control> = focus_owner.upcast();
                    let _ = navigation::handle_window_nav(&control, direction);
                }
                log::trace!("input: Ctrl+hjkl consumed key={:?}", keycode);
                viewport.set_input_as_handled();
                return;
            }
        }

        let consumed = match context {
            FocusContext::Editor | FocusContext::Foreign | FocusContext::Unknown => false,
            FocusContext::Dock(kind, control) => {
                let result = if navigation::is_in_filesystem_dock(&control) {
                    let fs_result = self.fs_explorer.handle_key(&key_event, &control, kind);
                    if fs_result.is_consumed() {
                        log::trace!("input: filesystem explorer consumed key={:?}", keycode);
                        fs_result
                    } else {
                        navigation::handle_dock_input(control, &key_event, kind)
                    }
                } else {
                    navigation::handle_dock_input(control, &key_event, kind)
                };
                if result.is_consumed() {
                    log::trace!("input: dock navigation consumed key={:?}", keycode);
                }
                result.is_consumed()
            }
            FocusContext::SearchBox(line_edit) => {
                if self.fs_explorer.is_prompt_active(&line_edit) {
                    false
                } else {
                    let result = navigation::handle_search_input(&line_edit, &key_event);
                    if result.is_consumed() {
                        log::trace!("input: search box consumed key={:?}", keycode);
                    }
                    result.is_consumed()
                }
            }
        };

        if consumed {
            viewport.set_input_as_handled();
        }
    }

    /// Per-editor keystroke handler. Connected to `gui_input` on the attached CodeEdit.
    pub(super) fn handle_gui_input_impl(&mut self, event: Gd<InputEvent>) {
        let Some(editor) = &self.attached_editor else {
            return;
        };
        // has_focus() guards against deferred delivery edge cases where gui_input
        // arrives after focus has moved away.
        if !editor.is_instance_valid() || !editor.has_focus() {
            return;
        }

        // ── Mouse wheel interception ─────────────────────────────────────
        // Route mouse wheel through the vim engine as Ctrl-Y (scroll up) /
        let Ok(key_event) = event.try_cast::<InputEventKey>() else {
            return;
        };
        // Accept both press and echo (key-repeat) -- held-key repeat is
        // correct Vim semantics (e.g. holding `j` to scroll down).
        if !key_event.is_pressed() {
            return;
        }
        let Some(key) = bridge::input::parse_godot_key(&key_event) else {
            return;
        };

        // IME compose guard: when TextEdit is actively composing (CJK input,
        // dead keys, alt-code unicode), don't consume the key — let it flow
        // through to TextEdit's native IME handling. Guards text-input modes
        // (Insert/Replace/CommandLine) where IME composition is meaningful.
        //
        // Escape-class keys (Escape, Ctrl+C, Ctrl+[) force-cancel the IME
        // composition so the user can always exit — even if the IME framework
        // doesn't cancel on Escape by itself.
        if editor.has_ime_text() {
            if let Some(controller) = &self.controller {
                let mode = controller.mode();
                if matches!(
                    mode,
                    vim_core::primitives::Mode::Insert
                        | vim_core::primitives::Mode::Replace
                        | vim_core::primitives::Mode::VirtualReplace
                        | vim_core::primitives::Mode::CommandLine
                ) {
                    let is_escape_key = matches!(key.key(), vim_core::keymap::Key::Escape)
                        || key == vim_core::keymap::KeyEvent::ctrl('c')
                        || key == vim_core::keymap::KeyEvent::ctrl('[');
                    if is_escape_key {
                        log::debug!("gui_input: force-cancelling IME for escape key={}", key);
                        let mut ed = editor.clone();
                        ed.cancel_ime();
                        // Fall through — Escape reaches the engine
                    } else {
                        log::trace!(
                            "gui_input: IME compose active in {:?}, passing through key={}",
                            mode,
                            key
                        );
                        return;
                    }
                } else {
                    // Stale IME composition in a non-insert mode — cancel it.
                    // This shouldn't happen (deactivate_ime cancels on mode exit),
                    // but some platforms/IMEs can leave stale state.
                    log::debug!("gui_input: cancelling stale IME in {:?}", mode);
                    let mut ed = editor.clone();
                    ed.cancel_ime();
                    // Fall through — key reaches the engine normally
                }
            }
        }

        // Cancel any pending tooltip — a new keystroke supersedes the hover.
        // Done before cloning `editor` to avoid borrow conflicts with `&mut self`.
        self.cancel_pending_tooltip();

        // Re-borrow after the mutable cancel call.
        let Some(editor) = &self.attached_editor else {
            return;
        };
        let mut ed = editor.clone();

        let outcome = {
            let _guard = ProcessingKeyGuard::new(&mut self.processing_key);
            let Some(controller) = &mut self.controller else {
                return;
            };
            controller.process_cycle(key, &mut ed)
        };

        let snap = {
            let Some(controller) = &mut self.controller else {
                return;
            };
            controller.ui_snapshot(ed.instance_id())
        };

        let applied = EngineOutcome::with_snapshot(snap, outcome)
            .apply_ui_update(&mut self.ui, &mut ed, &mut self.caret_reconciler);

        log::trace!(
            "gui_input: key={} outcome={}",
            key,
            applied.pipeline.log_label()
        );
        if applied.pipeline.should_mark_handled() {
            if let Some(mut vp) = editor.get_viewport() {
                vp.set_input_as_handled();
            }
        }

        if let Some(controller) = &mut self.controller {
            for action in controller.take_pending_ui_actions() {
                self.handle_pending_ui_action(action);
            }
        }

        // Start/restart the mapping timer if keys are buffered, stop if not.
        if let Some(controller) = &self.controller {
            if controller.has_pending_mapping() {
                if let Some(timer) = &mut self.mapping_timer {
                    let timeout_sec = controller.timeoutlen() as f64 / 1000.0;
                    timer.set_wait_time(timeout_sec);
                    timer.start();
                    log::trace!(
                        "gui_input: mapping timer started ({}ms)",
                        controller.timeoutlen()
                    );
                }
            } else if let Some(timer) = &mut self.mapping_timer {
                timer.stop();
            }
        }
    }

    /// Fired by the mapping timer after `timeoutlen` ms without further input.
    /// Flushes buffered keys as literals (or expands an exact match).
    pub(super) fn on_mapping_timeout_impl(&mut self) {
        let Some(editor) = &self.attached_editor else {
            return;
        };
        if !editor.is_instance_valid() {
            return;
        }
        let mut ed = editor.clone();

        let had_operations = {
            let _guard = ProcessingKeyGuard::new(&mut self.processing_key);
            if let Some(controller) = &mut self.controller {
                controller.resolve_mapping_timeout(&mut ed);
                controller.operations_this_cycle() > 0
            } else {
                false
            }
        };

        let editor_id = ed.instance_id();
        if let Some(controller) = &mut self.controller {
            let snap = controller.ui_snapshot(editor_id);
            // Use EngineConsumed when operations happened so apply_ui_update
            // sets a caret expectation; Passthrough otherwise so it does not.
            let pipeline = if had_operations {
                // Dummy ProcessResult -- only may_have_moved_cursor() matters,
                // which is true for EngineConsumed regardless of the payload.
                crate::controller::PipelineOutcome::EngineConsumed(
                    vim_core::execution::host_api::ProcessResult {
                        consumed: true,
                        host_requests: Vec::new(),
                        deferred_actions: Vec::new(),
                    },
                )
            } else {
                crate::controller::PipelineOutcome::Passthrough
            };
            EngineOutcome::with_snapshot(snap, pipeline)
                .apply_ui_update(&mut self.ui, &mut ed, &mut self.caret_reconciler);
        }

        if let Some(controller) = &mut self.controller {
            for action in controller.take_pending_ui_actions() {
                self.handle_pending_ui_action(action);
            }
        }

        // Timeout resolution may produce new pending keys (e.g. partial
        // match of a longer mapping) -- restart the timer so they resolve.
        if let Some(controller) = &self.controller {
            if controller.has_pending_mapping() {
                if let Some(timer) = &mut self.mapping_timer {
                    let timeout_sec = controller.timeoutlen() as f64 / 1000.0;
                    timer.set_wait_time(timeout_sec);
                    timer.start();
                }
            }
        }
    }

    /// Route a mouse-wheel event through the vim engine.
    ///
    /// Feeds Ctrl-Y (scroll up) or Ctrl-E (scroll down) through the normal
    /// `process_cycle` pipeline, repeating 3 times to match Godot's default
    /// scroll speed. Consumes the event so Godot's native scroll doesn't fire.
    /// Reconcile external cursor/selection changes with Vim engine state.
    /// Connected DEFERRED to avoid re-entrancy during text edits.
    ///
    /// Four cases based on (has_selection, vim_mode):
    /// 1. Selection + Normal  -- mouse drag entered Visual
    /// 2. No selection + Normal -- mouse click; sync sticky column
    /// 3. No selection + Visual -- click deselected; exit Visual
    /// 4. Selection + Visual  -- mouse extending; update Visual extents
    pub(super) fn on_caret_changed_impl(&mut self) {
        // Read caret position and check reconciler BEFORE any mutable borrows.
        // Uses a block to drop the immutable borrow of attached_editor before
        // cancel_pending_tooltip borrows &mut self.
        let (line, col) = {
            let Some(editor) = &self.attached_editor else {
                return;
            };
            if !editor.is_instance_valid() {
                return;
            }
            (editor.get_caret_line(), editor.get_caret_column())
        };

        match self.caret_reconciler.check_and_consume(line, col) {
            super::caret_reconcile::CaretOrigin::VimDriven => return,
            super::caret_reconcile::CaretOrigin::External => {}
        }

        self.cancel_pending_tooltip();

        let Some(controller) = &mut self.controller else {
            return;
        };

        let Some(editor) = &self.attached_editor else {
            return;
        };
        if !editor.is_instance_valid() {
            return;
        }
        let mut ed = editor.clone();
        let has_selection = ed.has_selection();
        let mode = controller.mode();

        if has_selection && !mode.is_visual_or_select() {
            log::debug!("on_caret_changed: mouse selection detected (entering visual)");
            apply_mouse_selection(
                controller,
                &mut ed,
                &mut self.caret_reconciler,
                &mut self.ui,
            );
        } else if !has_selection && mode.is_visual_or_select() {
            log::debug!("on_caret_changed: click deselect, exiting visual mode");
            let editor_id = ed.instance_id();
            controller.exit_mode_via_pipeline(&mut ed);
            controller.cleanup_visual_artifacts(editor_id, &mut ed);

            let char_col = crate::bridge::codec::i32_to_usize(ed.get_caret_column());
            let line_text = ed.get_line(ed.get_caret_line()).to_string();
            let grapheme_col = crate::bridge::codec::char_col_to_grapheme_col(&line_text, char_col);
            controller.set_engine_sticky_column(grapheme_col);

            let snap = controller.ui_snapshot(editor_id);
            EngineOutcome::with_snapshot(snap, crate::controller::PipelineOutcome::Passthrough)
                .apply_ui_update(&mut self.ui, &mut ed, &mut self.caret_reconciler);
        } else if has_selection && mode.is_visual_or_select() {
            log::trace!("on_caret_changed: visual selection updated");
            apply_mouse_selection(
                controller,
                &mut ed,
                &mut self.caret_reconciler,
                &mut self.ui,
            );
        } else {
            let char_col = crate::bridge::codec::i32_to_usize(ed.get_caret_column());
            let line_text = ed.get_line(ed.get_caret_line()).to_string();
            let grapheme_col = crate::bridge::codec::char_col_to_grapheme_col(&line_text, char_col);
            controller.set_engine_sticky_column(grapheme_col);
        }
    }
}

/// Translate Godot's selection extents into Vim anchor/head and forward to the
/// controller. Determines drag direction from caret position. Shared by Cases 1
/// (enter Visual) and 4 (extend Visual) in `on_caret_changed_impl`.
fn apply_mouse_selection(
    controller: &mut VimController,
    ed: &mut Gd<CodeEdit>,
    reconciler: &mut super::caret_reconcile::CaretReconciler,
    ui: &mut UiCoordinator,
) {
    let shape = detect_selection_shape(ed);

    let from_line = ed.get_selection_from_line();
    let from_col = ed.get_selection_from_column();
    let to_line = ed.get_selection_to_line();
    let to_col = ed.get_selection_to_column();

    // Godot puts the caret at the drag endpoint -- if caret is at the start
    // of the selection, the user dragged backward.
    let caret_line = ed.get_caret_line();
    let caret_col = ed.get_caret_column();
    let caret_at_start = caret_line == from_line && caret_col == from_col;

    let (anchor_line, anchor_col, head_line, head_col) = if caret_at_start {
        (to_line, to_col, from_line, from_col)
    } else {
        (from_line, from_col, to_line, to_col)
    };

    log::debug!(
        "apply_mouse_selection: shape={:?} anchor=({},{}) head=({},{})",
        shape,
        anchor_line,
        anchor_col,
        head_line,
        head_col
    );

    let did_change =
        controller.process_mouse_selection(ed, anchor_line, anchor_col, head_line, head_col, shape);

    if did_change {
        let editor_id = ed.instance_id();
        let snap = controller.ui_snapshot(editor_id);
        // Use EngineConsumed so apply_ui_update sets a caret expectation
        // (mouse selection moves the cursor from the engine's perspective).
        EngineOutcome::with_snapshot(
            snap,
            crate::controller::PipelineOutcome::EngineConsumed(
                vim_core::execution::host_api::ProcessResult {
                    consumed: true,
                    host_requests: Vec::new(),
                    deferred_actions: Vec::new(),
                },
            ),
        )
        .apply_ui_update(ui, ed, reconciler);
    }
}

/// Heuristic: Godot's triple-click produces a selection from col 0 to col 0
/// of the next line. Detecting this pattern lets us enter Visual Line mode
/// instead of Visual Char mode for triple-click selections.
fn detect_selection_shape(editor: &Gd<CodeEdit>) -> vim_core::primitives::SelectionShape {
    use vim_core::primitives::SelectionShape;

    let from_col = editor.get_selection_from_column();
    let to_line = editor.get_selection_to_line();
    let to_col = editor.get_selection_to_column();
    let from_line = editor.get_selection_from_line();

    if from_col == 0 && to_col == 0 && to_line > from_line {
        SelectionShape::Line
    } else {
        SelectionShape::Char
    }
}
