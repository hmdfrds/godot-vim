//! vim-core's engine on `MockTextEdit` through the production dispatch and
//! `UndoStore`. `MockHost` and `Harness` copy only the CodeEdit glue of
//! `bridge::godot_host` and `controller::process`; keep them in step.
//!
//! Not modelled: the completion popup, syntax queries (all code), and the fold
//! and indent providers (new lines get vim-core's autoindent). The
//! `get_version()` check in `GodotHost::refresh_from_editor` is not exercised:
//! `MockTextEdit` has no version counter.

use godot::prelude::InstanceId;
use vim_core::document::{Document, Providers};
use vim_core::effects::Effect;
use vim_core::execution::{
    parse_keys_from_string, HostCapability, HostCapabilitySet, HostRequest, HostResult,
    RequestDisposition, ViewportInfo, VimEngine, VimHost, VimSession,
};
use vim_core::primitives::{Mode, NodeId, Offset, Position, SelectionRange};

use crate::bridge::clipboard::MockClipboard;
use crate::bridge::codec::{self, LineIndex};
use crate::bridge::port::TextEditorPort;
use crate::bridge::{AutoBraceSnapshot, SyntaxRegion};
use crate::effects::dispatch::AutoBraceMode;
use crate::state::ShellState;
use crate::testing::MockTextEdit;

const EDITOR_ID: i64 = 1;

/// `GodotHost` with a `MockTextEdit` where the `Gd<CodeEdit>` was.
struct MockHost {
    editor: MockTextEdit,
    text_cache: String,
    line_index: LineIndex,
    generation: u64,
    cursor_offset: usize,
    state: ShellState,
    clipboard: MockClipboard,
    /// Godot's auto-brace pairs, or `None` when Auto Brace Complete is off.
    brace_pairs: Option<Vec<(String, String)>>,
    auto_brace_eligible: bool,
    engine_auto_pairs_active: bool,
}

impl MockHost {
    fn new(text: &str) -> Self {
        let editor = MockTextEdit::new(text);
        let text_cache = editor.get_text();
        let line_index = LineIndex::new(&text_cache);
        Self {
            editor,
            text_cache,
            line_index,
            generation: 0,
            cursor_offset: 0,
            state: ShellState::default(),
            clipboard: MockClipboard::new(),
            brace_pairs: None,
            auto_brace_eligible: false,
            engine_auto_pairs_active: false,
        }
    }

    fn editor_id() -> InstanceId {
        InstanceId::from_i64(EDITOR_ID)
    }

    /// `GodotHost::refresh_from_editor`, comparing text in place of `get_version()`.
    fn refresh_from_editor(&mut self) {
        let live = self.editor.get_text();
        if live != self.text_cache {
            self.text_cache = live;
            self.line_index = LineIndex::new(&self.text_cache);
            self.generation += 1;
        }
        self.cursor_offset = self.line_index.line_col_to_byte(
            &self.text_cache,
            self.editor.get_caret_line(),
            self.editor.get_caret_column(),
        );
    }

    fn auto_brace_snapshot(&self) -> AutoBraceSnapshot {
        let mut pairs = match &self.brace_pairs {
            Some(pairs) if self.auto_brace_eligible => pairs.clone(),
            _ => return AutoBraceSnapshot::disabled(),
        };
        // Longest open key first, as `AutoBraceSnapshot::from_editor` sorts.
        pairs.sort_by_key(|p| std::cmp::Reverse(p.0.len()));
        let mut snapshot = AutoBraceSnapshot {
            enabled: true,
            pairs: std::rc::Rc::new(pairs),
            string_delimiters: vec!["\"".into(), "'".into()],
        };
        if self.engine_auto_pairs_active {
            snapshot.filter_engine_owned_pairs();
        }
        snapshot
    }

    fn has_pending_undo(&self) -> bool {
        self.state
            .buffer_ref(Self::editor_id())
            .is_some_and(|b| b.undo_store().has_pending())
    }
}

impl Document for MockHost {
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
        Some(self.generation)
    }
}

impl VimHost for MockHost {
    fn capabilities(&self) -> HostCapabilitySet {
        HostCapabilitySet::FULL.without(HostCapability::NativeInsert)
    }

    fn cursor_offset(&self) -> usize {
        self.cursor_offset
    }

    fn viewport(&self) -> ViewportInfo {
        ViewportInfo {
            first_line: codec::i32_to_usize(self.editor.get_first_visible_line()),
            height: codec::i32_to_usize(self.editor.get_visible_line_count()),
            width: 80,
        }
    }

    fn selection(&self) -> Option<SelectionRange> {
        self.state
            .buffer_ref(Self::editor_id())
            .and_then(|b| b.visual())
            .map(|vs| SelectionRange::new(vs.anchor, vs.head))
    }

    fn providers(&self) -> Providers<'_> {
        Providers::default()
    }

    fn apply_effects(&mut self, effects: &[Effect]) {
        if effects.is_empty() {
            return;
        }
        let has_text_mutation = effects.iter().any(Effect::is_text_mutation);
        let line_index_hint = if has_text_mutation {
            None
        } else {
            Some(self.line_index.clone())
        };
        let auto_brace = if self.auto_brace_eligible {
            AutoBraceMode::Eligible
        } else {
            AutoBraceMode::Ineligible
        };
        let auto_brace_snapshot = self.auto_brace_snapshot();
        let cursor_count = codec::i32_to_usize(self.editor.get_caret_count().max(1));
        crate::effects::dispatch(
            effects.to_vec(),
            &mut self.editor,
            crate::effects::DispatchContext {
                state: &mut self.state,
                editor_id: Self::editor_id(),
                auto_brace,
                auto_brace_snapshot,
                line_index_hint,
                scrolloff: 0,
                highlight_yank_duration_ms: 0,
                syntax_query: Box::new(|_, _| SyntaxRegion::code()),
                clipboard: &mut self.clipboard,
                cursor_count,
            },
            &self.text_cache,
        );
        if has_text_mutation {
            self.text_cache = self.editor.get_text();
            self.line_index = LineIndex::new(&self.text_cache);
            self.generation += 1;
        }
        self.cursor_offset = self.line_index.line_col_to_byte(
            &self.text_cache,
            self.editor.get_caret_line(),
            self.editor.get_caret_column(),
        );
    }

    fn record_internal_undo_node(&mut self, node_id: NodeId, text_before: &str) {
        let store = self.state.buffer(Self::editor_id()).undo_store_mut();
        store.begin_group(text_before);
        store.end_group(node_id, &self.text_cache);
    }

    fn handle_request(&mut self, request: &HostRequest) -> RequestDisposition {
        RequestDisposition::Completed(HostResult::Failure {
            id: request.id(),
            error: "not modelled by MockHost".into(),
        })
    }
}

/// One attached editor: keys go through the same steps as a key that reaches
/// the engine in `controller::process::process_cycle_impl`.
pub(crate) struct Harness {
    session: VimSession<MockHost>,
    /// Pending undo groups `ensure_undo_balanced` had to throw away.
    orphaned_undo_groups: usize,
}

impl Harness {
    /// A tab-indented buffer in Normal mode with the caret at (0, 0), as
    /// `VimController::new` and `attach_session` set the engine up.
    pub(crate) fn new(text: &str) -> Self {
        let mut engine = VimEngine::new();
        engine.set_shadow_execution(true);
        // `sync_indent_from_editor` with Godot's defaults.
        let opts = engine.options_mut();
        opts.set_expandtab(false);
        opts.set_shiftwidth(4);
        opts.set_tabstop(4);
        let mut session = VimSession::from_parts(engine, MockHost::new(text));
        let initial = session.host().text().to_owned();
        session.engine_mut().set_shadow_text(initial);
        Self {
            session,
            orphaned_undo_groups: 0,
        }
    }

    /// Feed keys in Vim notation, one engine pass per key.
    pub(crate) fn feed(&mut self, keys: &str) {
        for key in parse_keys_from_string(keys) {
            let session = &mut self.session;
            session.host_mut().state.globals_mut().clear_message();
            let mode = session.engine().mode();
            let auto_pairs_active = session.engine().options().auto_pairs().is_some();
            let host = session.host_mut();
            host.refresh_from_editor();
            host.auto_brace_eligible = mode.is_insert();
            host.engine_auto_pairs_active = auto_pairs_active;
            let _ = session.process_key(key);
            self.ensure_undo_balanced();
        }
    }

    /// `GodotHost::ensure_undo_balanced`, counted instead of logged.
    fn ensure_undo_balanced(&mut self) {
        let mode = self.session.engine().mode();
        if mode.is_insert() || mode.is_replace() {
            return;
        }
        let host = self.session.host_mut();
        if host.has_pending_undo() {
            host.state
                .buffer(MockHost::editor_id())
                .undo_store_mut()
                .take_pending_text();
            self.orphaned_undo_groups += 1;
        }
    }

    /// Move Godot's caret, as a click does, without telling the engine.
    pub(crate) fn place_caret(&mut self, line: i32, col: i32) {
        let editor = &mut self.session.host_mut().editor;
        editor.set_caret_line(line);
        editor.set_caret_column(col);
    }

    pub(crate) fn text(&self) -> String {
        self.session.host().editor.get_text()
    }

    pub(crate) fn line(&self, line: i32) -> String {
        self.session.host().editor.get_line(line)
    }

    /// Godot's caret (line, column).
    pub(crate) fn caret(&self) -> (i32, i32) {
        let editor = &self.session.host().editor;
        (editor.get_caret_line(), editor.get_caret_column())
    }

    pub(crate) fn mode(&self) -> Mode {
        self.session.engine().mode()
    }

    pub(crate) fn orphaned_undo_groups(&self) -> usize {
        self.orphaned_undo_groups
    }
}
