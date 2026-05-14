//! [`GodotHost`]: Godot-side host adapter implementing `Document + VimHost`.
//!
//! Owns the `Gd<CodeEdit>`, text cache, cursor state, shell state,
//! and providers. Delegates effect application to [`crate::effects::dispatch`]
//! and host request handling to [`crate::host::execute`].
//!
//! Created once per editor and stored inside `VimSession<GodotHost>`.

use godot::classes::CodeEdit;
use godot::prelude::*;
use vim_core::document::{Document, Providers};
use vim_core::effects::Effect;
use vim_core::execution::{
    HostCapability, HostCapabilitySet, HostRequest, HostResult, RequestDisposition, ViewportInfo,
    VimHost,
};
use vim_core::primitives::{Mode, Offset, Position, SelectionRange, VisualType};

use super::clipboard::GodotClipboard;
use super::code_edit_ext::CodeEditExt;
use super::codec::{self, LineIndex};
use super::context::{OwnedGodotFoldProvider, OwnedGodotIndentProvider};
use super::port_impl::{AutoBraceSnapshot, SyntaxRegion};
use crate::effects::dispatch::{AutoBraceMode, DispatchContext};
use crate::host::SecurityPolicy;
use crate::settings::{FileAccessScope, ProjectVimrc, ShellExecution};
use crate::state::ShellState;

// ═══════════════════════════════════════════════════════════════════════════════
// PendingUiAction — deferred controller commands
// ═══════════════════════════════════════════════════════════════════════════════

/// Actions deferred for the controller/plugin layer to execute after
/// `VimSession::process_key()` returns.
///
/// These arise from custom Ex commands that need scene-tree or plugin-level
/// access that GodotHost does not have.
#[derive(Debug, Clone)]
pub(crate) enum PendingUiAction {
    OpenMappingDialog,
    SourceConfigFile,
    ShowUndoTree,
    Vimdebug(compact_str::CompactString),
    PerfReport,
    PerfReset,
    ShowTooltip {
        symbol: String,
        line: i32,
        col: i32,
        warp_pos: Option<Vector2i>,
    },
}

// ═══════════════════════════════════════════════════════════════════════════════
// GodotHost
// ═══════════════════════════════════════════════════════════════════════════════

/// Godot-side host adapter implementing [`Document`] + [`VimHost`].
///
/// Wraps a `Gd<CodeEdit>` and provides the document cache, cursor state,
/// selection, viewport, shell state, and fold/indent providers
/// that `VimSession<GodotHost>` needs.
pub(crate) struct GodotHost {
    // ── Document backing ────────────────────────────────────────────────
    editor: Gd<CodeEdit>,
    text_cache: String,
    line_index: LineIndex,
    cache_editor_id: InstanceId,
    cached_generation: u64,

    // ── VimHost state ───────────────────────────────────────────────────
    cursor_offset: usize,
    viewport: ViewportInfo,

    // ── Effect dispatch state ───────────────────────────────────────────
    state: ShellState,

    // ── Providers (own Gd<CodeEdit> clones) ─────────────────────────────
    fold_provider: OwnedGodotFoldProvider,
    indent_provider: OwnedGodotIndentProvider,

    // ── Host request handling ───────────────────────────────────────────
    security_policy: SecurityPolicy,
    clipboard: GodotClipboard,
    host_request_depth: u32,
    current_mode: Mode,

    // ── Configuration ───────────────────────────────────────────────────
    scrolloff: i32,
    highlight_yank_duration_ms: u32,
    auto_brace_eligible: bool,
    engine_auto_pairs_active: bool,
    #[allow(clippy::type_complexity)]
    brace_pair_cache: Option<(InstanceId, std::rc::Rc<Vec<(String, String)>>)>,

    // ── Deferred actions for controller ─────────────────────────────────
    pending_ui_actions: Vec<PendingUiAction>,

    // ── Vimdebug support ───────────────────────────────────────────────
    /// Whether vimdebug needs the effects summary captured this cycle.
    vimdebug_enabled: bool,
}

// ═══════════════════════════════════════════════════════════════════════════════
// Document implementation
// ═══════════════════════════════════════════════════════════════════════════════

impl Document for GodotHost {
    fn text(&self) -> &str {
        &self.text_cache
    }

    fn line_count(&self) -> usize {
        self.line_index.line_count()
    }

    fn offset_to_pos(&self, offset: Offset) -> Option<Position> {
        let lc = self
            .line_index
            .offset_to_line_col(&self.text_cache, offset.get())?;
        Some(Position::from_raw(
            codec::i32_to_usize(lc.line),
            codec::i32_to_usize(lc.col),
        ))
    }

    fn pos_to_offset(&self, pos: Position) -> Option<Offset> {
        let offset = self.line_index.line_col_to_offset(
            &self.text_cache,
            pos.line().get(),
            pos.col().get(),
        )?;
        Some(Offset::new(offset))
    }

    fn text_generation(&self) -> Option<u64> {
        Some(self.editor.get_version() as u64)
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// VimHost implementation
// ═══════════════════════════════════════════════════════════════════════════════

impl VimHost for GodotHost {
    fn capabilities(&self) -> HostCapabilitySet {
        HostCapabilitySet::FULL.without(HostCapability::NativeInsert)
    }

    fn cursor_offset(&self) -> usize {
        self.cursor_offset
    }

    fn viewport(&self) -> ViewportInfo {
        self.viewport
    }

    fn selection(&self) -> Option<SelectionRange> {
        let editor_id = self.editor.instance_id();
        self.state
            .buffer_ref(editor_id)
            .and_then(|b| b.visual())
            .map(|vs| SelectionRange::new(vs.anchor, vs.head))
    }

    fn providers(&self) -> Providers<'_> {
        Providers::default()
            .with_fold(&self.fold_provider)
            .with_indent(&self.indent_provider)
    }

    fn apply_effects(&mut self, effects: &[Effect]) {
        if effects.is_empty() {
            return;
        }

        // Track the last mode change so handle_request sees the current mode.
        // VimSession delivers effects BEFORE calling handle_request, so updating
        // current_mode here ensures it reflects the post-effects state.
        for effect in effects {
            if let Effect::SetMode { mode, .. } = effect {
                self.current_mode = *mode;
            }
        }

        let effects_vec: Vec<Effect> = effects.to_vec();

        let auto_brace = if self.auto_brace_eligible {
            AutoBraceMode::Eligible
        } else {
            AutoBraceMode::Ineligible
        };

        let has_text_mutation = effects.iter().any(|e| e.is_text_mutation());
        let line_index_hint = if has_text_mutation {
            None
        } else {
            Some(self.line_index.clone())
        };

        let mut auto_brace_snapshot = if self.auto_brace_eligible {
            AutoBraceSnapshot::from_editor(&self.editor, &mut self.brace_pair_cache)
        } else {
            AutoBraceSnapshot::disabled()
        };
        if self.engine_auto_pairs_active {
            auto_brace_snapshot.filter_engine_owned_pairs();
        }

        let editor_id = self.editor.instance_id();
        let scrolloff = self.scrolloff;
        let highlight_yank_duration_ms = self.highlight_yank_duration_ms;

        // Clone editor for syntax closure (Gd::clone is a cheap refcount bump).
        let editor_for_syntax = self.editor.clone();
        let cursor_count = self.editor.get_caret_count().max(1) as usize;

        // Destructure to satisfy the borrow checker: we need &mut self.editor
        // for CodeEditPort AND &mut self.state for DispatchContext
        // simultaneously. This works because Rust allows borrowing disjoint
        // fields of a struct.
        let Self {
            editor,
            state,
            clipboard,
            text_cache,
            line_index,
            cached_generation,
            cursor_offset,
            pending_ui_actions,
            ..
        } = self;

        let mut port = crate::bridge::port_impl::CodeEditPort(editor, pending_ui_actions);

        let _compound_actions = crate::effects::dispatch::dispatch(
            effects_vec,
            &mut port,
            DispatchContext {
                state,
                editor_id,
                auto_brace,
                auto_brace_snapshot,
                line_index_hint,
                scrolloff,
                highlight_yank_duration_ms,
                syntax_query: Box::new(move |line, col| {
                    SyntaxRegion::from_editor(&editor_for_syntax, line, col)
                }),
                clipboard,
                cursor_count,
            },
            text_cache,
        );

        // Compound actions (NormCommand, WindowNav) are now handled by
        // VimSession (Phase 3.0) at the HostRequest level. Any remaining
        // compound actions are logged by dispatch() itself.

        // Refresh text cache and cursor if text was mutated.
        if has_text_mutation {
            *text_cache = editor.get_text().to_string();
            *line_index = LineIndex::new(text_cache);
            *cached_generation = editor.get_version() as u64;
        }

        // Update cursor offset from editor.
        *cursor_offset = line_index.line_col_to_byte(
            text_cache,
            editor.get_caret_line(),
            editor.get_caret_column(),
        );

    }

    fn record_internal_undo_node(
        &mut self,
        node_id: vim_core::primitives::NodeId,
        text_before: &str,
    ) {
        let editor_id = self.editor.instance_id();
        let store = self.state.buffer(editor_id).undo_store_mut();
        store.begin_group(text_before);
        store.end_group(node_id, &self.text_cache);
    }

    fn handle_request(&mut self, request: &HostRequest) -> RequestDisposition {
        self.host_request_depth += 1;
        let result = self.handle_request_inner(request);
        self.host_request_depth -= 1;
        RequestDisposition::Completed(result)
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// Private helpers
// ═══════════════════════════════════════════════════════════════════════════════

/// Host request recursion depth limit. Typical depth is 1-2;
/// `:source` chains can reach 3. Five allows headroom without risk.
const MAX_HOST_REQUEST_DEPTH: u32 = 5;

impl GodotHost {
    fn handle_request_inner(&mut self, request: &HostRequest) -> HostResult {
        if self.host_request_depth > MAX_HOST_REQUEST_DEPTH {
            return HostResult::Failure {
                id: request.id(),
                error: compact_str::CompactString::from("host request depth limit exceeded"),
            };
        }

        // Intercept controller-level commands that need scene-tree or
        // plugin-level access. These are deferred for the controller
        // to execute after VimSession::process_key() returns.
        if let HostRequest::CustomExCommand { meta: _, command } = request {
            let cmd = command.as_str().trim();
            match cmd {
                "mappings" => {
                    self.pending_ui_actions
                        .push(PendingUiAction::OpenMappingDialog);
                    return HostResult::Success {
                        id: request.id(),
                        message: None,
                    };
                }
                "source" => {
                    self.pending_ui_actions
                        .push(PendingUiAction::SourceConfigFile);
                    return HostResult::Success {
                        id: request.id(),
                        message: None,
                    };
                }
                "undotree" => {
                    self.pending_ui_actions.push(PendingUiAction::ShowUndoTree);
                    return HostResult::Success {
                        id: request.id(),
                        message: None,
                    };
                }
                "perf" => {
                    self.pending_ui_actions.push(PendingUiAction::PerfReport);
                    return HostResult::Success {
                        id: request.id(),
                        message: None,
                    };
                }
                "perf reset" => {
                    self.pending_ui_actions.push(PendingUiAction::PerfReset);
                    return HostResult::Success {
                        id: request.id(),
                        message: None,
                    };
                }
                cmd_str if cmd_str.starts_with("vimdebug") => {
                    self.pending_ui_actions.push(PendingUiAction::Vimdebug(
                        compact_str::CompactString::from(cmd_str),
                    ));
                    return HostResult::Success {
                        id: request.id(),
                        message: None,
                    };
                }
                _ => {} // Fall through to host dispatch
            }
        }

        let mode_str = mode_to_vim_string(self.current_mode);
        let result = crate::host::execute(
            request,
            &mut self.editor,
            &self.security_policy,
            mode_str,
            &self.clipboard,
            &mut self.pending_ui_actions,
        );

        // Sandbox ReadConfigFile results to filter dangerous commands.
        if self.security_policy.project_vimrc == ProjectVimrc::Sandbox {
            if let HostRequest::ReadConfigFile { .. } = request {
                if let HostResult::Data { id, data, offset } = result {
                    let sandboxed = crate::config::sandbox::sandbox_config_text(data.as_str());
                    return HostResult::Data {
                        id,
                        data: compact_str::CompactString::from(sandboxed),
                        offset,
                    };
                }
            }
        }

        result
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// Construction and public API
// ═══════════════════════════════════════════════════════════════════════════════

impl GodotHost {
    /// Create a new `GodotHost` wrapping the given `Gd<CodeEdit>`.
    #[must_use]
    pub(crate) fn new(editor: Gd<CodeEdit>) -> Self {
        let text = editor.get_text().to_string();
        let line_index = LineIndex::new(&text);
        let cursor_offset =
            line_index.line_col_to_byte(&text, editor.get_caret_line(), editor.get_caret_column());
        let editor_id = editor.instance_id();
        let generation = editor.get_version() as u64;
        Self {
            fold_provider: OwnedGodotFoldProvider::new(editor.clone()),
            indent_provider: OwnedGodotIndentProvider::new(editor.clone()),
            editor,
            text_cache: text,
            line_index,
            cache_editor_id: editor_id,
            cached_generation: generation,
            cursor_offset,
            viewport: ViewportInfo {
                first_line: 0,
                height: 0,
                width: 0,
            },
            state: ShellState::default(),
            security_policy: SecurityPolicy {
                shell_execution: ShellExecution::Disabled,
                file_access_scope: FileAccessScope::ProjectOnly,
                project_vimrc: ProjectVimrc::Sandbox,
            },
            clipboard: GodotClipboard,
            host_request_depth: 0,
            current_mode: Mode::Normal,
            scrolloff: 0,
            highlight_yank_duration_ms: 150,
            auto_brace_eligible: false,
            engine_auto_pairs_active: false,
            brace_pair_cache: None,
            pending_ui_actions: Vec::new(),
            vimdebug_enabled: false,
        }
    }

    // ── Sync from editor ────────────────────────────────────────────────

    /// Sync text cache, cursor, and viewport from the live CodeEdit.
    ///
    /// Called before every `VimSession::process_key()` to ensure the host's
    /// document snapshot matches the editor's authoritative state.
    pub(crate) fn refresh_from_editor(&mut self) {
        let editor_id = self.editor.instance_id();
        let live_generation = self.editor.get_version() as u64;
        if editor_id != self.cache_editor_id {
            // Buffer switch — full refresh.
            self.text_cache = self.editor.get_text().to_string();
            self.line_index = LineIndex::new(&self.text_cache);
            self.cache_editor_id = editor_id;
            self.cached_generation = live_generation;
        } else if live_generation != self.cached_generation {
            // Text mutated externally (completion, IME, formatter, etc.)
            self.text_cache = self.editor.get_text().to_string();
            self.line_index = LineIndex::new(&self.text_cache);
            self.cached_generation = live_generation;
        }
        self.cursor_offset = self.line_index.line_col_to_byte(
            &self.text_cache,
            self.editor.get_caret_line(),
            self.editor.get_caret_column(),
        );
        // Viewport.
        self.viewport = ViewportInfo {
            first_line: codec::i32_to_usize(self.editor.get_first_visible_line()),
            height: self.editor.safe_visible_line_count(),
            width: approximate_viewport_width(&self.editor),
        };
    }

    /// Force a full text cache rebuild from the editor.
    pub(crate) fn invalidate_cache(&mut self) {
        self.text_cache = self.editor.get_text().to_string();
        self.line_index = LineIndex::new(&self.text_cache);
        self.cached_generation = self.editor.get_version() as u64;
    }

    // ── Configuration setters ───────────────────────────────────────────

    pub(crate) fn set_auto_brace_eligible(&mut self, eligible: bool) {
        self.auto_brace_eligible = eligible;
    }

    pub(crate) fn set_engine_auto_pairs_active(&mut self, active: bool) {
        self.engine_auto_pairs_active = active;
    }

    pub(crate) fn set_scrolloff(&mut self, scrolloff: i32) {
        self.scrolloff = scrolloff;
    }

    pub(crate) fn set_highlight_yank_duration_ms(&mut self, ms: u32) {
        self.highlight_yank_duration_ms = ms;
    }

    pub(crate) fn set_security_policy(&mut self, policy: SecurityPolicy) {
        self.security_policy = policy;
    }

    /// Update the cached mode so `handle_request` can pass it to host dispatch.
    pub(crate) fn set_current_mode(&mut self, mode: Mode) {
        self.current_mode = mode;
    }

    // ── Deferred UI actions ─────────────────────────────────────────────

    /// Drain all pending UI actions accumulated during the last process cycle.
    pub(crate) fn take_pending_ui_actions(&mut self) -> Vec<PendingUiAction> {
        std::mem::take(&mut self.pending_ui_actions)
    }

    // ── Undo safety ─────────────────────────────────────────────────────

    /// Check for orphaned `begin_group` calls left open by a bug or panic.
    /// Insert/Replace legitimately hold a pending group across keystrokes
    /// (opened on mode entry, closed on Esc); in other modes a pending
    /// group indicates a bug.
    pub(crate) fn ensure_undo_balanced(&mut self, mode: Mode) {
        if mode.is_insert() || mode.is_replace() {
            // Pending group is expected during insert/replace — the engine
            // opens a group on mode entry and closes it on Esc.
            return;
        }

        let editor_id = self.editor.instance_id();
        let has_pending = self.state.buffer(editor_id).undo_store().has_pending();
        if has_pending {
            log::warn!(
                "ensure_undo_balanced: orphaned undo group in {} mode, editor=#{} — discarding pending text",
                mode,
                editor_id.to_i64(),
            );
            self.state
                .buffer(editor_id)
                .undo_store_mut()
                .take_pending_text();
            self.state.globals_mut().set_error(
                "Internal: orphaned undo group recovered -- undo may be inconsistent",
            );
        }
    }

    // ── Vimdebug support ───────────────────────────────────────────────

    pub(crate) fn set_vimdebug_enabled(&mut self, enabled: bool) {
        self.vimdebug_enabled = enabled;
    }

    // ── Field accessors ─────────────────────────────────────────────────

    pub(crate) fn state(&self) -> &ShellState {
        &self.state
    }

    pub(crate) fn state_mut(&mut self) -> &mut ShellState {
        &mut self.state
    }

    /// Split borrow: simultaneous mutable access to state and pending UI
    /// actions, needed when a `CodeEditPort` is constructed from a separate
    /// `&mut Gd<CodeEdit>` (e.g. in step-mode effect application).
    pub(crate) fn state_and_pending_ui_actions_mut(
        &mut self,
    ) -> (&mut ShellState, &mut Vec<PendingUiAction>) {
        (&mut self.state, &mut self.pending_ui_actions)
    }

    pub(crate) fn take_state(&mut self) -> ShellState {
        std::mem::take(&mut self.state)
    }

    pub(crate) fn set_state(&mut self, state: ShellState) {
        self.state = state;
    }

    pub(crate) fn highlight_yank_duration_ms(&self) -> u32 {
        self.highlight_yank_duration_ms
    }

    // ── Multi-cursor sync accessors ────────────────────────────────────

    /// Access the cached text for multi-cursor position computation.
    pub(crate) fn text_cache(&self) -> &str {
        &self.text_cache
    }

    /// Access the line index for byte→line/col conversion.
    pub(crate) fn line_index(&self) -> &LineIndex {
        &self.line_index
    }

    /// Mutable access to the editor for multi-cursor sync operations.
    #[allow(dead_code)]
    pub(crate) fn editor_mut(&mut self) -> &mut Gd<CodeEdit> {
        &mut self.editor
    }

    /// Immutable access to the editor (for multi-cursor import).
    pub(crate) fn editor(&self) -> &Gd<CodeEdit> {
        &self.editor
    }

    /// Split borrow: simultaneous mutable access to editor, state, and
    /// pending UI actions.
    ///
    /// Required by multi-cursor sync which needs `CodeEditPort` (from editor)
    /// and `BufferState` (from shell state) simultaneously, and by any path
    /// that constructs a `CodeEditPort` (which carries `&mut Vec<PendingUiAction>`).
    pub(crate) fn editor_and_state_mut(
        &mut self,
    ) -> (&mut Gd<CodeEdit>, &mut ShellState, &mut Vec<PendingUiAction>) {
        (&mut self.editor, &mut self.state, &mut self.pending_ui_actions)
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// Free functions
// ═══════════════════════════════════════════════════════════════════════════════

/// Estimate viewport width in columns from pixel dimensions and font metrics.
///
/// CodeEdit exposes no column-count API, so we divide pixel width by the
/// space-character advance. Exact for monospace fonts (the code editor case);
/// only approximate if a proportional font is somehow configured. Falls back
/// to 80 columns when font metrics are unavailable or degenerate.
fn approximate_viewport_width(editor: &Gd<CodeEdit>) -> usize {
    const DEFAULT_VIEWPORT_WIDTH: usize = 80;

    let pixel_width = editor.get_size().x;
    if pixel_width <= 0.0 {
        return DEFAULT_VIEWPORT_WIDTH;
    }

    let Some(font) = editor.get_theme_font("font") else {
        return DEFAULT_VIEWPORT_WIDTH;
    };
    let font_size = editor.get_theme_font_size("font_size");
    let char_width = font.get_char_size(' ' as u32, font_size).x;
    if char_width <= 0.0 {
        return DEFAULT_VIEWPORT_WIDTH;
    }

    let ratio = pixel_width / char_width;
    if !ratio.is_finite() {
        return DEFAULT_VIEWPORT_WIDTH;
    }
    let columns = (ratio as usize).min(10000);
    if columns == 0 {
        DEFAULT_VIEWPORT_WIDTH
    } else {
        columns
    }
}

/// Map engine `Mode` to the single-char string that Vim's `mode()` function
/// returns. Used by `handle_request_inner` when delegating to host dispatch.
fn mode_to_vim_string(mode: Mode) -> &'static str {
    match mode {
        Mode::Normal => "n",
        Mode::Insert => "i",
        Mode::Visual(VisualType::Char) => "v",
        Mode::Visual(VisualType::Line) => "V",
        Mode::Visual(VisualType::Block) => "\x16",
        Mode::Select(VisualType::Char) => "s",
        Mode::Select(VisualType::Line) => "S",
        Mode::Select(VisualType::Block) => "\x13",
        Mode::Replace => "R",
        Mode::VirtualReplace => "Rv",
        Mode::CommandLine => "c",
        Mode::OperatorPending(_) => "no",
        _ => {
            log::warn!(
                "mode_to_vim_string: unknown Mode variant {} — defaulting to Normal",
                mode
            );
            "n"
        }
    }
}

#[cfg(test)]
const HANDLED_MODES: &[vim_core::primitives::ModeKind] = &[
    vim_core::primitives::ModeKind::Normal,
    vim_core::primitives::ModeKind::Insert,
    vim_core::primitives::ModeKind::Visual,
    vim_core::primitives::ModeKind::Select,
    vim_core::primitives::ModeKind::Replace,
    vim_core::primitives::ModeKind::VirtualReplace,
    vim_core::primitives::ModeKind::CommandLine,
    vim_core::primitives::ModeKind::OperatorPending,
];

#[cfg(test)]
mod mode_coverage_tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn mode_dispatch_covers_all_variants() {
        let handled: HashSet<_> = HANDLED_MODES.iter().copied().collect();
        let all: HashSet<_> = vim_core::primitives::ModeKind::ALL.iter().copied().collect();
        let missing: Vec<_> = all.difference(&handled).collect();
        assert!(
            missing.is_empty(),
            "Unhandled ModeKind variants: {:?}",
            missing
        );
    }

    #[test]
    fn handled_modes_has_no_duplicates() {
        let mut seen = HashSet::new();
        for kind in HANDLED_MODES {
            assert!(seen.insert(kind), "Duplicate in HANDLED_MODES: {:?}", kind);
        }
    }
}
