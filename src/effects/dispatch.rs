//! Two-pass effect dispatcher: applies vim-core [`Effect`] values to CodeEdit.
//!
//! Pass 1 processes text mutations (insert, delete, replace, undo/redo).
//! Pass 2 processes everything else (cursor, selection, mode, scroll, messages)
//! against the final document text.

use std::borrow::Cow;

use godot::prelude::*;
use vim_core::effects::Effect;
#[cfg(test)]
use vim_core::effects::EffectKind;

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
    /// Number of active cursors in the engine (1 = single cursor).
    /// Used to skip `remove_secondary_carets()` when multi-cursor is active.
    pub(crate) cursor_count: usize,
}

/// Read-only environment for pass-2 effect dispatch. Separates immutable
/// context from mutable targets to align with Rust's borrow model.
#[allow(dead_code)]
pub(crate) struct DispatchEnv<'a> {
    pub(crate) doc: &'a DocumentView<'a>,
    pub(crate) scrolloff: i32,
    pub(crate) highlight_yank_duration_ms: u32,
    pub(crate) editor_id: InstanceId,
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

/// Every [`EffectKind`] variant that has an explicit match arm in the dispatch
/// system. The coverage test in `coverage_tests` verifies this matches
/// [`EffectKind::ALL`] exactly — when vim-core adds a new variant, `cargo test`
/// fails immediately naming the missing variant.
#[cfg(test)]
const HANDLED_EFFECTS: &[EffectKind] = &[
    // Pass 1: text mutations + undo
    EffectKind::Insert,
    EffectKind::Delete,
    EffectKind::Replace,
    EffectKind::BeginUndoGroup,
    EffectKind::EndUndoGroup,
    EffectKind::Undo,
    EffectKind::UndoLine,
    EffectKind::Redo,
    // Pass 2 main loop: cursor/selection lifecycle
    EffectKind::SetCursor,
    EffectKind::SetSelection,
    EffectKind::ClearSelection,
    EffectKind::SaveSelections,
    EffectKind::RestoreSelections,
    // Pass 2: mode
    EffectKind::SetMode,
    EffectKind::CommandLineEdit,
    EffectKind::BeginInsert,
    EffectKind::SetBlockInsert,
    // Pass 2: cursor style
    EffectKind::SetCursorStyle,
    // Pass 2: search
    EffectKind::SetSearchPattern,
    EffectKind::ClearHighlights,
    EffectKind::HighlightMatches,
    EffectKind::SubstitutePreview,
    EffectKind::ClearSubstitutePreview,
    EffectKind::SearchMatchInfo,
    // Pass 2: scroll
    EffectKind::ScrollTo,
    EffectKind::CenterCursor,
    EffectKind::CursorToTop,
    EffectKind::CursorToBottom,
    EffectKind::ScrollLeft,
    EffectKind::ScrollRight,
    // Pass 2: fold
    EffectKind::FoldLine,
    EffectKind::UnfoldLine,
    EffectKind::ToggleFold,
    EffectKind::ToggleFoldRecursive,
    EffectKind::FoldAll,
    EffectKind::UnfoldAll,
    EffectKind::FoldLineRecursive,
    EffectKind::UnfoldLineRecursive,
    EffectKind::DeleteFold,
    EffectKind::DeleteFoldRecursive,
    EffectKind::SetFoldEnable,
    EffectKind::EliminateAllFolds,
    EffectKind::ToggleFoldEnable,
    // Pass 2: window (actionable)
    EffectKind::WindowMoveLeft,
    EffectKind::WindowMoveRight,
    EffectKind::WindowMoveUp,
    EffectKind::WindowMoveDown,
    EffectKind::WindowNext,
    EffectKind::WindowPrev,
    EffectKind::WindowClose,
    // Pass 2: window (no-op in Godot)
    EffectKind::WindowSplit,
    EffectKind::WindowVSplit,
    EffectKind::WindowOnly,
    EffectKind::WindowEqualSize,
    EffectKind::WindowNew,
    EffectKind::WindowRotateDown,
    EffectKind::WindowRotateUp,
    EffectKind::WindowIncreaseHeight,
    EffectKind::WindowDecreaseHeight,
    EffectKind::WindowIncreaseWidth,
    EffectKind::WindowDecreaseWidth,
    // Pass 2: messages
    EffectKind::ShowInfo,
    EffectKind::ShowError,
    EffectKind::ShowWarning,
    EffectKind::ClearMessage,
    // Pass 2: registers
    EffectKind::SetRegister,
    EffectKind::CopyToClipboard,
    // Pass 2: compound actions
    EffectKind::NormCommand,
    EffectKind::OperatorFilter,
    // Pass 2: engine-internal (enriched logging)
    EffectKind::SetMark,
    EffectKind::Event,
    // Pass 2: engine-internal (state updated by effect_processor)
    EffectKind::PushJumpList,
    EffectKind::JumpOlder,
    EffectKind::JumpNewer,
    EffectKind::StartRecording,
    EffectKind::StopRecording,
    EffectKind::OperatorToMark,
    EffectKind::OperatorReindent,
    EffectKind::SaveLastVisual,
    EffectKind::SetLastFind,
    EffectKind::ChangelistOlder,
    EffectKind::ChangelistNewer,
    EffectKind::SetStickyColumn,
    EffectKind::SetSubstitutePattern,
    EffectKind::SetHighlightRange,
    EffectKind::ClearHighlightRange,
    EffectKind::ClearNamedRegister,
    EffectKind::ClearMark,
    EffectKind::JumpToBuffer,
    EffectKind::SetDiagnostics,
    EffectKind::SyncFoldRanges,
    EffectKind::SetLastSubstitute,
    EffectKind::SetLastSubstituteFlags,
    // Pass 2: macro replay
    EffectKind::PlayMacro,
    // Pass 2: LSP navigation
    EffectKind::GotoDefinition,
    EffectKind::ShowDocumentation,
    // Pass 2: host action
    EffectKind::HostAction,
    // Pass 2: virtual text
    EffectKind::SetVirtualText,
    EffectKind::ClearVirtualText,
    // Pass 2: undo tree visualization
    EffectKind::UndoTreeSnapshot,
    // Pass 2: unsupported commands
    EffectKind::OpenCommandWindow,
    EffectKind::CallOperatorFunc,
    // Pass 2: no-op
    EffectKind::Noop,
    // Pass 2: mode transition
    EffectKind::ModeTransition,
    // Pass 2: substitute confirm
    EffectKind::SubstituteConfirmShow,
    EffectKind::SubstituteConfirmEnd,
    EffectKind::SetSubstituteConfirmState,
    EffectKind::ClearSubstituteConfirmState,
    // Pass 2: syntax selection
    EffectKind::SyntaxSelectionPush,
    EffectKind::SyntaxSelectionPop,
    // Pass 2: multi-selection / block selection
    EffectKind::SelectNextMatch,
    EffectKind::SelectPreviousMatch,
    EffectKind::SetBlockSelections,
    EffectKind::HighlightRows,
    // Pass 2: scroll half-count
    EffectKind::SetScrollHalfCount,
    // Pass 2: explicit no-ops (user-facing, intentionally unhandled)
    EffectKind::Bell,
    EffectKind::ShowMatch,
    EffectKind::CursorShapeHint,
    EffectKind::RequestTimer,
    EffectKind::CrossBufferEdit,
    // Pass 2: explicit no-ops (engine-internal, consumed by effect processor)
    EffectKind::SetExtState,
    EffectKind::ClearExtState,
    EffectKind::SetVariable,
    EffectKind::DeleteVariable,
    EffectKind::SyntaxHistoryClear,
    EffectKind::SetSyntaxSelections,
];

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
        auto_brace,
        auto_brace_snapshot,
        line_index_hint,
        scrolloff,
        highlight_yank_duration_ms,
        syntax_query,
        clipboard,
        cursor_count,
    } = ctx;
    let auto_brace_eligible = matches!(auto_brace, AutoBraceMode::Eligible);
    log::trace!("dispatch: {} effects", effects.len());
    let mut pass2 = Vec::with_capacity(effects.len());

    // Clear stale carets and selections. When multi-cursor is active,
    // preserve secondary carets — they're managed by the indexed SetCursor
    // routing in pass 2 and post-dispatch sync.
    if cursor_count <= 1 {
        editor.remove_secondary_carets();
    }
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
                let text_str: &str = &text;
                state
                    .buffer(editor_id)
                    .undo_store_mut()
                    .begin_group(text_str);
            }
            Effect::EndUndoGroup { node_id } => {
                if let Some(node_id) = node_id {
                    let text_str: &str = &text;
                    state
                        .buffer(editor_id)
                        .undo_store_mut()
                        .end_group(node_id, text_str);
                } else {
                    // Empty group (no edits) — discard pending text.
                    state.buffer(editor_id).undo_store_mut().take_pending_text();
                }
            }
            Effect::Undo { steps, .. } => {
                let mut any_applied = false;
                for step in &steps {
                    let current_text: &str = &text;
                    let result = state
                        .buffer(editor_id)
                        .undo_store_mut()
                        .undo_step(step.node_id, current_text);
                    if let Some(result) = result {
                        let doc = DocumentView::new(&text, &line_index);
                        undo::apply_changes_to_editor(editor, &doc, &result);
                        undo::restore_cursors(editor, &result.text, &step.cursors);
                        text = Cow::Owned(result.text);
                        line_index = LineIndex::new(&text);
                        any_applied = true;
                    } else {
                        log::warn!(
                            "undo: no snapshot for node {} — skipping step",
                            step.node_id
                        );
                    }
                }
                if any_applied {
                    text_mutated = true;
                }
            }
            Effect::UndoLine { count } => {
                undo::handle_undo_line(count);
            }
            Effect::Redo { steps, .. } => {
                let mut any_applied = false;
                for step in &steps {
                    let current_text: &str = &text;
                    let result = state
                        .buffer(editor_id)
                        .undo_store_mut()
                        .redo_step(step.node_id, current_text);
                    if let Some(result) = result {
                        let doc = DocumentView::new(&text, &line_index);
                        undo::apply_changes_to_editor(editor, &doc, &result);
                        undo::restore_cursors(editor, &result.text, &step.cursors);
                        text = Cow::Owned(result.text);
                        line_index = LineIndex::new(&text);
                        any_applied = true;
                    } else {
                        log::warn!(
                            "redo: no snapshot for node {} — skipping step",
                            step.node_id
                        );
                    }
                }
                if any_applied {
                    text_mutated = true;
                }
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
    let env = DispatchEnv {
        doc: &doc,
        scrolloff,
        highlight_yank_duration_ms,
        editor_id,
    };

    let mut pairing = SelectionPairing::Idle;
    let mut cursor_effect_index: usize = 0;

    for effect in pass2 {
        match effect {
            Effect::SetSelection {
                anchor,
                head,
                shape,
            } => {
                if cursor_count > 1 {
                    log::trace!("pass2: SetSelection skipped (multi-cursor, sync-only)");
                } else {
                    log::trace!(
                        "pass2: SetSelection anchor={} head={} shape={:?}",
                        anchor.get(),
                        head.get(),
                        shape
                    );
                    cursor::handle_set_selection(editor, &doc, anchor.get(), head.get(), shape);
                    let head_pos = doc.line_index.byte_to_line_col(doc.text, head.get());
                    let anchor_pos = doc.line_index.byte_to_line_col(doc.text, anchor.get());
                    state
                        .buffer(editor_id)
                        .update_visual_selection(anchor, head, head_pos, anchor_pos);
                }
                pairing = pairing.on_set_selection();
            }
            Effect::ClearSelection => {
                if cursor_count > 1 {
                    editor.deselect();
                    state.buffer(editor_id).clear_visual_selection();
                } else {
                    // Capture canonical head before clearing — Godot's caret is at
                    // head_col+1 from inclusive→exclusive rendering in SetSelection.
                    let restore_pos = state.buffer(editor_id).visual().map(|vs| vs.head_pos);
                    cursor::handle_clear_selection(editor);
                    state.buffer(editor_id).clear_visual_selection();
                    if let Some(pos) = restore_pos {
                        editor.set_caret_line(pos.line);
                        editor.set_caret_column(pos.col);
                    }
                }
                pairing = pairing.on_consume_cursor();
            }
            Effect::SetCursor { offset: _ } if pairing.should_suppress_cursor() => {
                log::trace!("pass2: SetCursor skipped (awaiting cursor for selection)");
                pairing = pairing.on_consume_cursor();
            }
            Effect::SetCursor { offset } => {
                if cursor_count > 1 {
                    log::trace!("pass2: SetCursor skipped (multi-cursor, sync-only)");
                } else {
                    let pos = doc.line_index.byte_to_line_col(doc.text, offset.get());
                    if cursor_effect_index == 0 {
                        cursor::handle_set_cursor(editor, &doc, offset.get(), scrolloff);
                    } else {
                        let idx = cursor_effect_index as i32;
                        let caret_count = editor.get_caret_count();
                        if idx >= caret_count {
                            let added = editor.add_caret(pos.line, pos.col);
                            if added < 0 {
                                log::error!(
                                    "multi-cursor: add_caret({}, {}) failed for index {}",
                                    pos.line,
                                    pos.col,
                                    cursor_effect_index
                                );
                                cursor_effect_index += 1;
                                continue;
                            }
                        } else {
                            editor.set_caret_line_for(pos.line, idx);
                            editor.set_caret_column_for(pos.col, idx);
                        }
                    }
                }
                cursor_effect_index += 1;
            }
            Effect::SaveSelections { tag } => {
                // Snapshot current cursor positions from the editor into buffer state.
                let caret_count = editor.get_caret_count();
                let mut positions = Vec::with_capacity(caret_count as usize);
                for idx in 0..caret_count {
                    let line = editor.get_caret_line_for(idx) as usize;
                    let col = editor.get_caret_column_for(idx) as usize;
                    positions.push((line, col, 0));
                }
                log::trace!(
                    "SaveSelections({:?}): saved {} caret positions",
                    tag,
                    positions.len()
                );
                state.buffer(editor_id).save_selections(positions);
            }
            Effect::RestoreSelections { tag } => {
                if let Some(positions) = state.buffer(editor_id).saved_selections() {
                    let positions = positions.to_vec();
                    log::trace!(
                        "RestoreSelections({:?}): restoring {} caret positions",
                        tag,
                        positions.len()
                    );
                    // Restore carets: primary first, then secondaries.
                    for (idx, &(line, col, _)) in positions.iter().enumerate() {
                        let line_i32 = line as i32;
                        let col_i32 = col as i32;
                        if idx == 0 {
                            editor.set_caret_line(line_i32);
                            editor.set_caret_column(col_i32);
                        } else {
                            let caret_idx = idx as i32;
                            let caret_count = editor.get_caret_count();
                            if caret_idx >= caret_count {
                                editor.add_caret(line_i32, col_i32);
                            } else {
                                editor.set_caret_line_for(line_i32, caret_idx);
                                editor.set_caret_column_for(col_i32, caret_idx);
                            }
                        }
                    }
                    // Remove excess carets beyond the restored count.
                    let target = positions.len() as i32;
                    let current = editor.get_caret_count();
                    for idx in (target..current).rev() {
                        editor.remove_caret(idx);
                    }
                    // Update cursor_effect_index so the tail cleanup preserves
                    // the restored carets instead of clearing them.
                    cursor_effect_index = positions.len();
                    // Update last_caret_count so the import logic sees the
                    // correct count after RestoreSelections.
                    state
                        .buffer(editor_id)
                        .set_last_caret_count(positions.len());
                    // One-shot restore: clear saved data so stale positions are
                    // never accidentally re-applied.
                    state.buffer(editor_id).clear_saved_selections();
                } else {
                    log::trace!("RestoreSelections({:?}): no saved state — skipped", tag);
                }
            }
            other => {
                dispatch_pass2_effect(
                    other,
                    editor,
                    state,
                    &env,
                    &mut compound_actions,
                    clipboard,
                );
            }
        }
    }

    // Note: pairing may end in AwaitingCursor when vim-core emits multiple
    // SetSelection effects without matching SetCursor (e.g., visual block
    // extension can emit 2 SetSelections + 1 SetCursor). This is not a bug —
    // the last SetSelection wins and its cursor is correctly positioned.
    if matches!(pairing, SelectionPairing::AwaitingCursor { .. }) {
        log::trace!(
            "selection pairing ended in {:?} (extra SetSelections without cursor — normal for visual block)",
            pairing
        );
    }

    if cursor_count > 1 {
        // Multi-cursor: post-sync owns all caret lifecycle. No cleanup here.
    } else if cursor_effect_index >= 2 {
        let target_count = cursor_effect_index as i32;
        let current_count = editor.get_caret_count();
        for idx in (target_count..current_count).rev() {
            editor.remove_caret(idx);
        }
    } else {
        editor.remove_secondary_carets();
    }

    compound_actions
}

/// Route a single pass-2 effect to its domain handler. Compound actions
/// (`:norm`, window nav) are collected for the controller to handle after
/// dispatch completes.
pub(crate) fn dispatch_pass2_effect(
    effect: Effect,
    editor: &mut (impl FoldCapable + IdeCapable + NavigationCapable),
    state: &mut ShellState,
    env: &DispatchEnv<'_>,
    compound_actions: &mut Vec<CompoundAction>,
    clipboard: &mut dyn crate::bridge::clipboard::ClipboardPort,
) {
    match effect {
        // ── Cursor ──────────────────────────────────────────────────────
        Effect::SetCursor { .. } => {
            dispatch_cursor_effect(effect, editor, env.doc, env.scrolloff);
        }
        // ── Mode ────────────────────────────────────────────────────────
        Effect::SetMode { .. }
        | Effect::CommandLineEdit(_)
        | Effect::BeginInsert { .. }
        | Effect::SetBlockInsert { .. } => {
            dispatch_mode_effect(effect, editor);
        }

        // ── Cursor style ─────────────────────────────────────────────────
        Effect::SetCursorStyle { style } => {
            state.set_cursor_style(style);
        }

        // ── Search ──────────────────────────────────────────────────────
        Effect::SetSearchPattern { .. }
        | Effect::ClearHighlights
        | Effect::HighlightMatches { .. }
        | Effect::SubstitutePreview { .. }
        | Effect::ClearSubstitutePreview
        | Effect::SearchMatchInfo { .. } => {
            dispatch_search_effect(effect, state, env.doc);
        }

        // ── Scroll ──────────────────────────────────────────────────────
        Effect::ScrollTo { .. }
        | Effect::CenterCursor
        | Effect::CursorToTop
        | Effect::CursorToBottom
        | Effect::ScrollLeft { .. }
        | Effect::ScrollRight { .. } => {
            dispatch_scroll_effect(effect, editor, env.doc);
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
        Effect::ShowInfo { .. } | Effect::ShowError { .. } | Effect::ShowWarning { .. } | Effect::ClearMessage => {
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
        // ── Highlight ranges (yank flash) ──────────────────────────────────
        Effect::SetHighlightRange { ref owner, ref range, shape, .. } => {
            if owner.as_str() == vim_core::effects::HIGHLIGHT_OWNER_YANK
                && env.highlight_yank_duration_ms > 0
            {
                let start = env.doc.line_index.byte_to_line_col(env.doc.text, range.start().get());
                let end = env.doc.line_index.byte_to_line_col(env.doc.text, range.end().get());
                state.set_highlight_yank(crate::types::HighlightYank::new(
                    start,
                    end,
                    env.highlight_yank_duration_ms,
                    shape,
                ));
            } else {
                log::trace!("[highlight] SetHighlightRange owner={} (no-op)", owner);
            }
        }
        Effect::ClearHighlightRange { ref owner, .. } => {
            log::trace!("[highlight] ClearHighlightRange owner={}", owner);
        }

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
        | Effect::SetSubstitutePattern { .. }
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
            let report = crate::state::undo_format::format_undo_tree_snapshot(&snapshot);
            messages::handle_show_message(state.globals_mut(), &report);
        }
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

        // ── Engine-internal: mode transition (processed by effect_processor) ──
        Effect::ModeTransition { .. } => {
            log::trace!("[internal] mode transition");
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

        // ── Multi-selection / block selection state ───────────────────────
        // SaveSelections and RestoreSelections are handled in the main pass 2
        // loop (alongside SetCursor) so they can update cursor_effect_index.
        // They should never reach here.
        Effect::SelectNextMatch {
            ref pattern,
            skip_current,
        } => {
            // Engine-internal state mutation already applied before effects reach
            // the host. The post-dispatch sync renders the new cursor positions.
            log::trace!(
                "SelectNextMatch: pattern={:?} skip_current={}",
                pattern,
                skip_current
            );
        }
        Effect::SelectPreviousMatch {
            ref pattern,
            skip_current,
        } => {
            log::trace!(
                "SelectPreviousMatch: pattern={:?} skip_current={}",
                pattern,
                skip_current
            );
        }
        Effect::SetBlockSelections { .. } => {
            // Rendered by BlockVisualOverlay via UiSnapshot::block_visual.
            // This effect carries the logical selection data for engine state;
            // no additional host work needed.
            log::trace!("[internal] SetBlockSelections (rendered via BlockVisualOverlay)");
        }
        Effect::HighlightRows { .. } => {
            log::trace!("[internal] HighlightRows");
        }

        // ── Engine-internal: scroll half-count (state-only) ─────────────
        Effect::SetScrollHalfCount { .. } => {
            log::trace!("[internal] SetScrollHalfCount");
        }

        // ── Explicit no-ops: user-facing effects intentionally unhandled ──
        Effect::Bell => {
            // No audible/visual bell in Godot editor
        }
        Effect::ShowMatch { .. } => {
            // CodeEdit has native bracket matching; showmatch not needed
        }
        Effect::CursorShapeHint { .. } => {
            // Mode-based pull model handles cursor shape
        }
        Effect::RequestTimer { .. } => {
            // Timer infrastructure not yet needed
        }
        Effect::CrossBufferEdit { .. } => {
            // Single-buffer editor, no cross-buffer support
        }

        // ── Explicit no-ops: engine-internal, consumed by effect processor ──
        Effect::SetExtState { .. } => {}
        Effect::ClearExtState { .. } => {}
        Effect::SetVariable { .. } => {}
        Effect::DeleteVariable { .. } => {}
        Effect::SyntaxHistoryClear => {}
        Effect::SetSyntaxSelections { .. } => {}

        // ── Forward compatibility for #[non_exhaustive] ─────────────────
        // This arm only fires for Effect variants from a newer vim-core
        // that godot-vim was not compiled against.
        effect => {
            log::warn!(
                "dispatch: unrecognized effect from newer vim-core: {:?}",
                effect.kind()
            );
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
        Effect::SearchMatchInfo { current, total, .. } => {
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
        Effect::ShowInfo { info } => {
            messages::handle_show_message(state.globals_mut(), &format!("{}", info));
        }
        Effect::ShowWarning { text } => {
            messages::handle_show_message(state.globals_mut(), &text);
        }
        Effect::ShowError { error, .. } => {
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

#[cfg(test)]
mod coverage_tests {
    use super::HANDLED_EFFECTS;
    use vim_core::effects::EffectKind;

    #[test]
    fn effect_dispatch_covers_all_known_variants() {
        use std::collections::HashSet;

        let handled: HashSet<EffectKind> = HANDLED_EFFECTS.iter().copied().collect();
        let all: HashSet<EffectKind> = EffectKind::ALL.iter().copied().collect();

        let missing: Vec<_> = all.difference(&handled).collect();
        let stale: Vec<_> = handled.difference(&all).collect();

        assert!(
            missing.is_empty(),
            "Effect variants not in HANDLED_EFFECTS (add explicit match arm + registry entry): {:?}",
            missing
        );
        assert!(
            stale.is_empty(),
            "Stale entries in HANDLED_EFFECTS (variant removed from vim-core): {:?}",
            stale
        );
    }

    #[test]
    fn handled_effects_has_no_duplicates() {
        use std::collections::HashSet;

        let mut seen = HashSet::new();
        for kind in HANDLED_EFFECTS {
            assert!(
                seen.insert(kind),
                "Duplicate entry in HANDLED_EFFECTS: {:?}",
                kind
            );
        }
    }
}
