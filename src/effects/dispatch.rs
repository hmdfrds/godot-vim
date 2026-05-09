//! Two-pass effect dispatcher: applies vim-core [`Effect`] values to CodeEdit.
//!
//! Pass 1 processes text mutations (insert, delete, replace, undo/redo).
//! Pass 2 processes everything else (cursor, selection, mode, scroll, messages)
//! against the final document text.

use std::borrow::Cow;

use godot::prelude::*;
use vim_core::effects::Effect;

use super::{
    auto_brace,
    compound::{CompoundAction, LineNumber, WindowNavAction},
    cursor, messages, mode, navigation, registers, scroll, search, text, undo,
};
use crate::bridge::codec::{usize_to_i32, DocumentView, LineIndex};
use crate::bridge::port::{FoldCapable, IdeCapable, NavigationCapable, TextEditorPort};
use crate::bridge::{AutoBraceSnapshot, SyntaxRegion};
use crate::state::ShellState;
use crate::types::{MatchRange, RemapPolicy};

/// Cap for substitute preview matches to avoid UI lag on large files.
const MAX_SUBSTITUTE_PREVIEW_MATCHES: usize = 100;

/// Whether auto-brace completion should be applied for insert effects.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AutoBraceMode {
    /// Auto-brace completion pairs should be checked and applied.
    Eligible,
    /// Auto-brace completion is not applicable (e.g., during `:norm` execution).
    Ineligible,
}

/// Non-editor context for [`dispatch`], bundled to keep the signature narrow.
pub(crate) struct DispatchContext<'a> {
    pub(crate) state: &'a mut ShellState,
    pub(crate) editor_id: InstanceId,
    pub(crate) undo_depth: &'a mut undo::UndoDepth,
    pub(crate) auto_brace: AutoBraceMode,
    /// Brace pairs, string delimiters, and enabled flag — captured once before
    /// dispatch to avoid repeated FFI calls per effect.
    pub(crate) auto_brace_snapshot: AutoBraceSnapshot,
    /// Reusable `LineIndex` from context-build. Still valid when pass 1 has no
    /// mutations, avoiding an O(n) rebuild for pass 2.
    pub(crate) line_index_hint: Option<LineIndex>,
    pub(crate) scrolloff: i32,
    /// Yank highlight duration in ms (0 = disabled). Overrides the engine's value.
    pub(crate) highlight_yank_duration_ms: u32,
    /// Position-dependent syntax query (string/comment context). Production
    /// captures `Gd<CodeEdit>` for FFI; tests return `SyntaxRegion::code()`.
    pub(crate) syntax_query: Box<dyn Fn(i32, i32) -> SyntaxRegion + 'a>,
    /// Clipboard abstraction for register sync and copy-to-clipboard effects.
    pub(crate) clipboard: &'a mut dyn crate::bridge::clipboard::ClipboardPort,
}

/// State machine tracking SetSelection → SetCursor effect pairing.
///
/// The engine guarantees each `SetSelection` is followed by a `SetCursor`
/// at the selection head. The dispatch layer must suppress that `SetCursor`
/// because `select()` already positioned the caret. This enum replaces the
/// previous `selection_set_this_batch: bool` + `unpaired_selections: u32`
/// with a self-documenting state machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SelectionPairing {
    /// No pending SetSelection → SetCursor pair.
    Idle,
    /// One or more SetSelections received, awaiting their paired SetCursor(s).
    AwaitingCursor { count: u32 },
}

impl SelectionPairing {
    fn on_set_selection(self) -> Self {
        match self {
            Self::Idle => Self::AwaitingCursor { count: 1 },
            Self::AwaitingCursor { count } => Self::AwaitingCursor { count: count + 1 },
        }
    }

    fn on_consume_cursor(self) -> Self {
        match self {
            Self::AwaitingCursor { count: 1 } => Self::Idle,
            Self::AwaitingCursor { count } => Self::AwaitingCursor { count: count - 1 },
            Self::Idle => Self::Idle,
        }
    }

    fn should_suppress_cursor(&self) -> bool {
        matches!(self, Self::AwaitingCursor { .. })
    }
}

/// Translate a list of vim-core `Effect`s into Godot CodeEdit API calls.
///
/// # Effect Ordering Contract
///
/// **Pass 1** (text mutations): `Insert`, `Delete`, `Replace`, `Undo`, `Redo`,
/// `BeginUndoGroup`, `EndUndoGroup`. Applied in order. Text cache invalidated
/// after each mutation.
///
/// **Pass 2** (everything else): Applied against the final document text from
/// pass 1. `SetSelection` MUST appear before its matching `SetCursor` in the
/// effect list — the engine guarantees this. If `SetSelection` is present,
/// `SetCursor` is suppressed (Godot's `select()` already positions the caret).
/// Validated at runtime by `SelectionPairing` state machine + `debug_assert!`.
///
/// Returns any [`CompoundAction`]s (e.g., `:norm`) that require the caller
/// to re-drive the engine. The controller processes these in
/// `apply_effects` → `process_compound_action`.
pub(crate) fn dispatch(
    effects: Vec<Effect>,
    editor: &mut (impl FoldCapable + IdeCapable + NavigationCapable),
    ctx: DispatchContext<'_>,
    text_ref: &str,
) -> Vec<CompoundAction> {
    let DispatchContext {
        state,
        editor_id,
        undo_depth,
        auto_brace,
        auto_brace_snapshot,
        line_index_hint,
        scrolloff,
        highlight_yank_duration_ms,
        syntax_query,
        clipboard,
    } = ctx;
    let auto_brace_eligible = matches!(auto_brace, AutoBraceMode::Eligible);
    log::trace!("dispatch: {} effects", effects.len());
    let mut pass2 = Vec::with_capacity(effects.len());

    // Block visual selection creates secondary carets that persist across dispatches.
    // Godot's caret-relative APIs operate on ALL carets, so clear before pass 1.
    editor.remove_secondary_carets();
    editor.deselect();

    // Pass 1: text mutations and undo. The Cow starts as a zero-copy borrow;
    // any mutation transitions it to Owned via editor.get_text() or in-place splice.
    let mut text: Cow<str> = Cow::Borrowed(text_ref);
    let mut text_mutated = false;
    let mut line_index = match line_index_hint {
        Some(hint) => hint,
        None => LineIndex::new(&text),
    };

    for effect in effects {
        match effect {
            Effect::Insert {
                offset,
                text: content,
            } => {
                let doc = DocumentView::new(&text, &line_index);
                // Auto-brace only fires for single printable characters (typing,
                // not paste). Control chars are excluded because Godot's
                // _handle_unicode_input_internal never receives them.
                let is_single_char = content.chars().count() == 1;
                let is_control_char = content.starts_with(|c: char| c.is_control());
                if auto_brace_eligible
                    && is_single_char
                    && !is_control_char
                    && auto_brace_snapshot.enabled
                {
                    let Some(ch) = content.chars().next() else {
                        // Defensive: is_single_char guarantees at least one char.
                        text::handle_insert(editor, &doc, offset.get(), &content);
                        text = Cow::Owned(editor.get_text());
                        line_index = LineIndex::new(&text);
                        text_mutated = true;
                        continue;
                    };
                    let lc = doc.line_index.byte_to_line_col(doc.text, offset.get());
                    let syntax = syntax_query(lc.line, lc.col);
                    match auto_brace::handle_insert_with_auto_brace(
                        editor,
                        &doc,
                        offset.get(),
                        ch,
                        &auto_brace_snapshot,
                        &syntax,
                    ) {
                        auto_brace::AutoBraceResult::Inserted => {
                            // Auto-brace may have inserted a closing brace — we can't
                            // predict the change, so re-fetch authoritatively.
                            text = Cow::Owned(editor.get_text());
                            line_index = LineIndex::new(&text);
                            text_mutated = true;
                        }
                        auto_brace::AutoBraceResult::SkippedOver => {
                            // Skip-over only moves the caret; text unchanged.
                        }
                    }
                } else {
                    text::handle_insert(editor, &doc, offset.get(), &content);
                    let byte_offset = offset.get();
                    text.to_mut().insert_str(byte_offset, &content);
                    line_index.apply_insert(byte_offset, &content);
                    text_mutated = true;
                }
            }
            Effect::Delete { range } => {
                let doc = DocumentView::new(&text, &line_index);
                let start = range.start().get();
                let end = range.end().get();
                text::handle_delete(editor, &doc, start, end);
                // Auto-brace backspace: if the deleted range was an opening brace
                // with an adjacent close brace, delete the close brace too. The
                // Cow still holds pre-delete text (handle_delete only read it).
                let has_auto_brace = auto_brace_eligible && auto_brace_snapshot.enabled;
                if has_auto_brace {
                    auto_brace::handle_delete_with_auto_brace(
                        editor,
                        &doc,
                        start,
                        end,
                        &auto_brace_snapshot,
                    );
                    text = Cow::Owned(editor.get_text());
                    line_index = LineIndex::new(&text);
                } else {
                    text.to_mut().drain(start..end);
                    line_index.apply_delete(start, end);
                }
                text_mutated = true;
            }
            Effect::Replace {
                range,
                text: content,
            } => {
                let doc = DocumentView::new(&text, &line_index);
                let start = range.start().get();
                let end = range.end().get();
                text::handle_replace(editor, &doc, start, end, &content);
                text.to_mut().replace_range(start..end, &content);
                line_index.apply_delete(start, end);
                line_index.apply_insert(start, &content);
                text_mutated = true;
            }
            Effect::BeginUndoGroup { .. } => {
                undo::handle_begin_undo_group(editor, undo_depth);
            }
            Effect::EndUndoGroup { .. } => {
                undo::handle_end_undo_group(editor, undo_depth);
                // Record an undo tree snapshot at the outermost group boundary.
                if undo_depth.is_zero() {
                    let text_str: &str = &text;
                    state.buffer(editor_id).record_undo_edit(text_str);
                }
            }
            Effect::Undo { count, .. } => {
                undo::handle_undo(editor, count);
                text = Cow::Owned(editor.get_text());
                line_index = LineIndex::new(&text);
                text_mutated = true;
            }
            Effect::UndoLine { count } => {
                undo::handle_undo_line(count);
            }
            Effect::Redo { count, .. } => {
                undo::handle_redo(editor, count);
                text = Cow::Owned(editor.get_text());
                line_index = LineIndex::new(&text);
                text_mutated = true;
            }
            other => pass2.push(other),
        }
    }

    // Debug-only: verify our incremental text mirror matches the editor's
    // authoritative state. Catches splice divergence bugs early.
    #[cfg(debug_assertions)]
    if text_mutated {
        let editor_text = editor.get_text();
        debug_assert_eq!(
            text.as_ref(),
            editor_text.as_str(),
            "text mirror out of sync with editor after pass 1"
        );
    }

    // Pass 2: cursor, selection, mode, scroll, messages, etc. against
    // the final document text. The line_index is either the reused hint
    // (no pass-1 mutations) or incrementally updated through splices.
    let mut compound_actions = Vec::new();
    let doc = DocumentView::new(&text, &line_index);

    let mut pairing = SelectionPairing::Idle;

    for effect in pass2 {
        match effect {
            Effect::SetSelection {
                anchor,
                head,
                shape,
            } => {
                log::trace!(
                    "pass2: SetSelection anchor={} head={} shape={:?}",
                    anchor.get(),
                    head.get(),
                    shape
                );
                cursor::handle_set_selection(editor, &doc, anchor.get(), head.get(), shape);
                let head_pos = doc.line_index.byte_to_line_col(doc.text, head.get());
                state
                    .buffer(editor_id)
                    .update_visual_selection(anchor, head, head_pos);
                pairing = pairing.on_set_selection();
            }
            Effect::ClearSelection => {
                // Capture canonical head before clearing — Godot's caret is at
                // head_col+1 from inclusive→exclusive rendering in SetSelection.
                let restore_pos = state.buffer(editor_id).visual().map(|vs| vs.head_pos);
                cursor::handle_clear_selection(editor);
                state.buffer(editor_id).clear_visual_selection();
                if let Some(pos) = restore_pos {
                    editor.set_caret_line(pos.line);
                    editor.set_caret_column(pos.col);
                }
                pairing = pairing.on_consume_cursor();
            }
            Effect::SetCursor { offset: _ } if pairing.should_suppress_cursor() => {
                log::trace!("pass2: SetCursor skipped (awaiting cursor for selection)");
                pairing = pairing.on_consume_cursor();
            }
            other => {
                dispatch_pass2_effect(
                    other,
                    editor,
                    state,
                    &doc,
                    &mut compound_actions,
                    scrolloff,
                    highlight_yank_duration_ms,
                    clipboard,
                );
            }
        }
    }

    debug_assert!(
        matches!(pairing, SelectionPairing::Idle),
        "Engine invariant: selection pairing ended in {:?}, expected Idle",
        pairing
    );

    if matches!(pairing, SelectionPairing::AwaitingCursor { .. }) {
        log::warn!(
            "Engine invariant: selection pairing ended in {:?}, expected Idle",
            pairing
        );
    }

    compound_actions
}

/// Route a single pass-2 effect to its domain handler. Compound actions
/// (`:norm`, window nav) are collected for the controller to handle after
/// dispatch completes.
#[allow(clippy::too_many_arguments)]
pub(crate) fn dispatch_pass2_effect(
    effect: Effect,
    editor: &mut (impl FoldCapable + IdeCapable + NavigationCapable),
    state: &mut ShellState,
    doc: &DocumentView,
    compound_actions: &mut Vec<CompoundAction>,
    scrolloff: i32,
    _highlight_yank_duration_ms: u32,
    clipboard: &mut dyn crate::bridge::clipboard::ClipboardPort,
) {
    match effect {
        // ── Cursor ──────────────────────────────────────────────────────
        Effect::SetCursor { .. } => {
            dispatch_cursor_effect(effect, editor, doc, scrolloff);
        }
        // ── Mode ────────────────────────────────────────────────────────
        Effect::SetMode { .. }
        | Effect::CommandLineEdit(_)
        | Effect::BeginInsert { .. }
        | Effect::SetBlockInsert { .. } => {
            dispatch_mode_effect(effect, editor);
        }

        // ── Search ──────────────────────────────────────────────────────
        Effect::SetSearchPattern { .. }
        | Effect::ClearHighlights
        | Effect::HighlightMatches { .. }
        | Effect::SubstitutePreview { .. }
        | Effect::ClearSubstitutePreview
        | Effect::SearchMatchInfo { .. } => {
            dispatch_search_effect(effect, state, doc);
        }

        // ── Scroll ──────────────────────────────────────────────────────
        Effect::ScrollTo { .. }
        | Effect::CenterCursor
        | Effect::CursorToTop
        | Effect::CursorToBottom
        | Effect::ScrollLeft { .. }
        | Effect::ScrollRight { .. } => {
            dispatch_scroll_effect(effect, editor, doc);
        }
        // ── Fold ────────────────────────────────────────────────────────
        Effect::FoldLine { .. }
        | Effect::UnfoldLine { .. }
        | Effect::ToggleFold { .. }
        | Effect::ToggleFoldRecursive { .. }
        | Effect::FoldAll
        | Effect::UnfoldAll
        | Effect::FoldLineRecursive { .. }
        | Effect::UnfoldLineRecursive { .. }
        | Effect::DeleteFold { .. }
        | Effect::DeleteFoldRecursive { .. }
        | Effect::SetFoldEnable { .. }
        | Effect::EliminateAllFolds
        | Effect::ToggleFoldEnable => {
            dispatch_fold_effect(effect, editor);
        }

        // ── Window ──────────────────────────────────────────────────────
        // Actionable window effects are promoted to CompoundAction::WindowNav
        // so the controller can handle them with Godot scene tree access.
        Effect::WindowMoveLeft => {
            compound_actions.push(CompoundAction::WindowNav {
                action: WindowNavAction::MoveLeft,
            });
        }
        Effect::WindowMoveRight => {
            compound_actions.push(CompoundAction::WindowNav {
                action: WindowNavAction::MoveRight,
            });
        }
        Effect::WindowMoveUp => {
            compound_actions.push(CompoundAction::WindowNav {
                action: WindowNavAction::MoveUp,
            });
        }
        Effect::WindowMoveDown => {
            compound_actions.push(CompoundAction::WindowNav {
                action: WindowNavAction::MoveDown,
            });
        }
        Effect::WindowNext => {
            compound_actions.push(CompoundAction::WindowNav {
                action: WindowNavAction::CycleNext,
            });
        }
        Effect::WindowPrev => {
            compound_actions.push(CompoundAction::WindowNav {
                action: WindowNavAction::CyclePrev,
            });
        }
        Effect::WindowClose => {
            compound_actions.push(CompoundAction::WindowNav {
                action: WindowNavAction::CloseTab,
            });
        }
        // No meaningful mapping in Godot's single-editor-per-tab model.
        Effect::WindowSplit
        | Effect::WindowVSplit
        | Effect::WindowOnly
        | Effect::WindowEqualSize
        | Effect::WindowNew
        | Effect::WindowRotateDown
        | Effect::WindowRotateUp
        | Effect::WindowIncreaseHeight { .. }
        | Effect::WindowDecreaseHeight { .. }
        | Effect::WindowIncreaseWidth { .. }
        | Effect::WindowDecreaseWidth { .. } => {
            log::trace!("Window effect (no-op in Godot single-editor)");
        }

        // ── Message ─────────────────────────────────────────────────────
        Effect::ShowMessage { .. } | Effect::ShowError { .. } | Effect::ClearMessage => {
            dispatch_message_effect(effect, state);
        }

        // ── Register ────────────────────────────────────────────────────
        Effect::SetRegister { .. } | Effect::CopyToClipboard { .. } => {
            dispatch_register_effect(effect, clipboard);
        }

        // ── Compound actions (require re-driving the engine) ────────────
        Effect::NormCommand {
            start_line,
            end_line,
            keys,
            remap,
        } => {
            compound_actions.push(CompoundAction::NormCommand {
                start_line: LineNumber::new(start_line.get()),
                end_line: LineNumber::new(end_line.get()),
                keys: keys.to_string(),
                remap: RemapPolicy::from(remap),
            });
        }
        Effect::OperatorFilter { .. } => {
            log::warn!(
                "OperatorFilter reached effect dispatch — the engine should promote \
                 filters to HostRequest::FilterDocumentRange before they reach here"
            );
            state
                .globals_mut()
                .set_error("Internal error: filter command not processed — please report this bug");
        }

        // ── Engine-internal: enriched logging for diagnostically useful variants ──
        Effect::SetMark { name, .. } => {
            log::trace!("[internal] SetMark('{name}')");
        }
        Effect::Event { kind } => {
            log::trace!("[internal] Event({kind:?})");
        }

        // ── Engine-internal: state updated by effect_processor, no shell work ──
        e @ (Effect::PushJumpList { .. }
        | Effect::JumpOlder { .. }
        | Effect::JumpNewer { .. }
        | Effect::StartRecording { .. }
        | Effect::StopRecording
        | Effect::OperatorToMark { .. }
        | Effect::OperatorReindent { .. }
        | Effect::SaveLastVisual { .. }
        | Effect::SetLastFind { .. }
        | Effect::ChangelistOlder { .. }
        | Effect::ChangelistNewer { .. }
        | Effect::SetStickyColumn { .. }
        | Effect::SetCursorStyle { .. }
        | Effect::SetSubstitutePattern { .. }
        | Effect::SetPluginHighlight { .. }
        | Effect::ClearPluginHighlight { .. }
        | Effect::ClearNamedRegister { .. }
        | Effect::ClearMark { .. }
        | Effect::JumpToBuffer { .. }
        | Effect::SetDiagnostics { .. }
        | Effect::SyncFoldRanges { .. }
        | Effect::SetLastSubstitute { .. }
        | Effect::SetLastSubstituteFlags { .. }) => {
            log::trace!("[internal] {:?}", e.kind());
        }
        Effect::PlayMacro { register, count } => {
            log::debug!(
                "PlayMacro: register='{}' count={} (keys fed via drain_pending)",
                register.char(),
                count
            );
        }

        // ── Other: LSP, host action, virtual text, undo tree, etc. ──────
        Effect::GotoDefinition => {
            navigation::handle_goto_definition(editor);
        }
        Effect::ShowDocumentation => {
            navigation::handle_show_documentation(editor);
        }
        Effect::HostAction { name } => {
            log::debug!("HostAction: {}", name);
            messages::handle_show_message(state.globals_mut(), &format!("HostAction: {}", name));
        }
        Effect::SetVirtualText {
            namespace,
            line,
            col,
            ref text,
            position,
        } => {
            log::trace!(
                "SetVirtualText: ns={} line={} col={} text={} pos={:?}",
                namespace,
                line.get(),
                col.get(),
                text,
                position,
            );
        }
        Effect::ClearVirtualText { namespace } => {
            log::trace!("ClearVirtualText: ns={}", namespace);
        }
        Effect::UndoTreeSnapshot { snapshot } => {
            let report = crate::state::undo_tree::format_undo_tree_snapshot(&snapshot);
            log::info!("UndoTreeSnapshot:\n{}", report);
        }
        // Note: HighlightYank was removed from vim-core (replaced by
        // HighlightRows for a different purpose). Yank highlighting is now
        // handled via the host event pipeline. HighlightRows is matched
        // in the multi-selection block below.
        Effect::OpenCommandWindow { .. } => {
            log::warn!("q: / q/ command window not supported in CodeEdit");
            state
                .globals_mut()
                .set_error("E11: Command window not supported in CodeEdit");
        }
        Effect::CallOperatorFunc { range, motion_type } => {
            log::warn!("operatorfunc (g@) not yet supported");
            state
                .globals_mut()
                .set_error("E774: operatorfunc (g@) not yet supported");
            log::debug!(
                "CallOperatorFunc: range={}..{} motion={:?}",
                range.start().get(),
                range.end().get(),
                motion_type
            );
        }
        // Produced by the compose middleware when Insert+Delete annihilate.
        Effect::Noop => {}

        // ── Engine-internal: plugin state (processed by effect_processor) ──
        Effect::SetPluginState { .. } | Effect::ClearPluginState { .. } => {
            log::trace!("[internal] plugin state update");
        }

        // ── Engine-internal: substitute confirm (processed by effect_processor) ──
        Effect::SubstituteConfirmShow { .. }
        | Effect::SubstituteConfirmEnd
        | Effect::SetSubstituteConfirmState { .. }
        | Effect::ClearSubstituteConfirmState => {
            log::trace!("[internal] substitute confirm state update");
        }

        // ── Engine-internal: syntax selection (VS Code / multi-cursor) ──
        Effect::SyntaxSelectionPush { .. } | Effect::SyntaxSelectionPop => {
            log::trace!("[internal] syntax selection (no-op in CodeEdit)");
        }

        // ── Engine-internal: multi-selection / block selection state ─────
        Effect::HighlightRows { .. }
        | Effect::SetBlockSelections { .. }
        | Effect::SaveSelections { .. }
        | Effect::RestoreSelections { .. }
        | Effect::SelectNextMatch { .. }
        | Effect::SelectPreviousMatch { .. } => {
            log::trace!("[internal] multi-selection effect (no-op in CodeEdit)");
        }

        // ── Engine-internal: scroll half-count (state-only) ─────────────
        Effect::SetScrollHalfCount { .. } => {
            log::trace!("[internal] SetScrollHalfCount");
        }

        // ── Forward compatibility for #[non_exhaustive] ─────────────────
        // New effects from future vim-core versions are expected and benign.
        effect => {
            log::debug!("dispatch: unknown effect from newer vim-core: {:?}", effect);
        }
    }
}

// ── Domain sub-dispatchers ──────────────────────────────────────────────────

/// SetCursor only. SetSelection/ClearSelection are handled in the main
/// dispatch loop (with `SelectionPairing` tracking) and never reach here.
fn dispatch_cursor_effect(
    effect: Effect,
    editor: &mut impl TextEditorPort,
    doc: &DocumentView,
    scrolloff: i32,
) {
    match effect {
        Effect::SetCursor { offset } => {
            cursor::handle_set_cursor(editor, doc, offset.get(), scrolloff);
        }
        other => log::error!("dispatch_cursor_effect: unexpected effect {:?}", other),
    }
}

fn dispatch_mode_effect(effect: Effect, editor: &mut impl IdeCapable) {
    match effect {
        Effect::SetMode { mode, .. } => {
            mode::handle_set_mode(editor, mode);
        }
        Effect::CommandLineEdit(edit) => {
            mode::handle_command_line_edit(edit);
        }
        Effect::BeginInsert {
            entry_type,
            count,
            auto_indent_len,
            entry_offset,
        } => {
            mode::handle_begin_insert(entry_type, count, auto_indent_len, entry_offset);
        }
        Effect::SetBlockInsert {
            lines_below,
            grapheme_col,
            cursor_return_offset,
        } => {
            mode::handle_set_block_insert(lines_below, grapheme_col, cursor_return_offset);
        }
        other => log::error!("dispatch_mode_effect: unexpected effect {:?}", other),
    }
}

fn dispatch_search_effect(effect: Effect, state: &mut ShellState, doc: &DocumentView) {
    match effect {
        Effect::SetSearchPattern { .. } => {
            // Pattern stored engine-side (pulled via ui_snapshot). Shell only
            // needs to re-enable hlsearch (which :noh may have suppressed).
            log::trace!("SetSearchPattern: hlsearch re-enabled");
            state.globals_mut().set_hlsearch_enabled(true);
        }
        Effect::ClearHighlights => {
            search::handle_clear_highlights(state.globals_mut());
        }
        Effect::HighlightMatches { ranges } => {
            search::handle_highlight_matches(&ranges);
        }
        Effect::SubstitutePreview { ref matches } => {
            log::trace!("SubstitutePreview: {} match(es)", matches.len());
            let positions: Vec<MatchRange> = matches
                .iter()
                .take(MAX_SUBSTITUTE_PREVIEW_MATCHES)
                .map(|substitute_match| {
                    let start_pos = doc
                        .line_index
                        .byte_to_line_col(doc.text, substitute_match.match_start().get());
                    let end_pos = doc
                        .line_index
                        .byte_to_line_col(doc.text, substitute_match.match_end().get());
                    MatchRange::with_replacement(
                        start_pos,
                        end_pos,
                        compact_str::CompactString::from(substitute_match.replacement()),
                    )
                })
                .collect();
            state.set_substitute_preview(positions);
        }
        Effect::ClearSubstitutePreview => {
            log::trace!("ClearSubstitutePreview");
            state.clear_substitute_preview();
        }
        Effect::SearchMatchInfo { current, total } => {
            let msg = compact_str::format_compact!("[{}/{}]", current, total);
            messages::handle_show_message(state.globals_mut(), msg.as_str());
        }
        other => log::error!("dispatch_search_effect: unexpected effect {:?}", other),
    }
}

fn dispatch_scroll_effect(effect: Effect, editor: &mut impl TextEditorPort, doc: &DocumentView) {
    match effect {
        Effect::ScrollTo { offset } => {
            scroll::handle_scroll_to(editor, doc, offset.get());
        }
        Effect::CenterCursor => scroll::handle_center_cursor(editor),
        Effect::CursorToTop => scroll::handle_cursor_to_top(editor),
        Effect::CursorToBottom => scroll::handle_cursor_to_bottom(editor),
        Effect::ScrollLeft { count } => scroll::handle_scroll_left(editor, count),
        Effect::ScrollRight { count } => scroll::handle_scroll_right(editor, count),
        other => log::error!("dispatch_scroll_effect: unexpected effect {:?}", other),
    }
}

fn dispatch_fold_effect(effect: Effect, editor: &mut impl FoldCapable) {
    match effect {
        Effect::FoldLine { line } => {
            editor.fold_line(usize_to_i32(line.get()));
        }
        Effect::UnfoldLine { line } => {
            editor.unfold_line(usize_to_i32(line.get()));
        }
        Effect::ToggleFold { line } | Effect::ToggleFoldRecursive { line } => {
            // CodeEdit has no recursive fold toggle — best-effort non-recursive.
            editor.toggle_foldable_line(usize_to_i32(line.get()));
        }
        Effect::FoldAll => {
            editor.fold_all_lines();
        }
        Effect::UnfoldAll => {
            editor.unfold_all_lines();
        }
        // No recursive fold API in CodeEdit — best-effort non-recursive.
        Effect::FoldLineRecursive { line } => {
            editor.fold_line(usize_to_i32(line.get()));
        }
        Effect::UnfoldLineRecursive { line } => {
            editor.unfold_line(usize_to_i32(line.get()));
        }
        // Vim's zd/zD delete fold markers; CodeEdit manages folds
        // automatically, so unfold is the closest equivalent.
        Effect::DeleteFold { line } => {
            editor.unfold_line(usize_to_i32(line.get()));
        }
        Effect::DeleteFoldRecursive { line } => {
            editor.unfold_line(usize_to_i32(line.get()));
        }
        Effect::SetFoldEnable { enabled } => {
            if !enabled {
                editor.unfold_all_lines();
            }
        }
        Effect::EliminateAllFolds => {
            editor.unfold_all_lines();
        }
        Effect::ToggleFoldEnable => {
            // No fold-enable toggle in CodeEdit — unfold all as fallback.
            log::trace!("ToggleFoldEnable: no native toggle, unfold_all as fallback");
            editor.unfold_all_lines();
        }
        other => log::error!("dispatch_fold_effect: unexpected effect {:?}", other),
    }
}

fn dispatch_message_effect(effect: Effect, state: &mut ShellState) {
    match effect {
        Effect::ShowMessage { text: msg } => {
            messages::handle_show_message(state.globals_mut(), &msg);
        }
        Effect::ShowError { error } => {
            messages::handle_show_error(state.globals_mut(), &error);
        }
        Effect::ClearMessage => {
            messages::handle_clear_message(state.globals_mut());
        }
        other => log::error!("dispatch_message_effect: unexpected effect {:?}", other),
    }
}

fn dispatch_register_effect(
    effect: Effect,
    clipboard: &mut dyn crate::bridge::clipboard::ClipboardPort,
) {
    match effect {
        Effect::SetRegister {
            name,
            text: content,
            ..
        } => {
            registers::sync_register_to_clipboard(name, &content, clipboard);
        }
        Effect::CopyToClipboard { text: content, .. } => {
            registers::handle_copy_to_clipboard(&content, clipboard);
        }
        other => log::error!("dispatch_register_effect: unexpected effect {:?}", other),
    }
}

#[cfg(test)]
mod selection_pairing_tests {
    use super::SelectionPairing;

    #[test]
    fn idle_does_not_suppress_cursor() {
        assert!(!SelectionPairing::Idle.should_suppress_cursor());
    }

    #[test]
    fn set_selection_transitions_to_awaiting() {
        let state = SelectionPairing::Idle.on_set_selection();
        assert_eq!(state, SelectionPairing::AwaitingCursor { count: 1 });
        assert!(state.should_suppress_cursor());
    }

    #[test]
    fn consume_cursor_returns_to_idle() {
        let state = SelectionPairing::Idle
            .on_set_selection()
            .on_consume_cursor();
        assert_eq!(state, SelectionPairing::Idle);
    }

    #[test]
    fn multiple_selections_tracked() {
        let state = SelectionPairing::Idle.on_set_selection().on_set_selection();
        assert_eq!(state, SelectionPairing::AwaitingCursor { count: 2 });

        let state = state.on_consume_cursor();
        assert_eq!(state, SelectionPairing::AwaitingCursor { count: 1 });

        let state = state.on_consume_cursor();
        assert_eq!(state, SelectionPairing::Idle);
    }

    #[test]
    fn clear_selection_consumes_like_cursor() {
        let state = SelectionPairing::Idle
            .on_set_selection()
            .on_consume_cursor();
        assert_eq!(state, SelectionPairing::Idle);
    }

    #[test]
    fn consume_cursor_from_idle_stays_idle() {
        let state = SelectionPairing::Idle.on_consume_cursor();
        assert_eq!(state, SelectionPairing::Idle);
    }

    #[test]
    fn set_selection_cursor_clear_sequence() {
        // [SetSelection, SetCursor, ClearSelection]
        let state = SelectionPairing::Idle
            .on_set_selection() // -> AwaitingCursor { 1 }
            .on_consume_cursor() // SetCursor: -> Idle
            .on_consume_cursor(); // ClearSelection: -> Idle (no-op)
        assert_eq!(state, SelectionPairing::Idle);
    }

    #[test]
    fn set_selection_clear_cursor_sequence() {
        // [SetSelection, ClearSelection, SetCursor]
        let state = SelectionPairing::Idle
            .on_set_selection() // -> AwaitingCursor { 1 }
            .on_consume_cursor() // ClearSelection: -> Idle
            .on_consume_cursor(); // SetCursor: -> Idle (should NOT suppress)
        assert_eq!(state, SelectionPairing::Idle);
    }
}
