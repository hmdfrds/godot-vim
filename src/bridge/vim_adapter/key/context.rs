//! Key context - shared state passed through the consumer pipeline.

use std::sync::Arc;

use crate::bridge::vim_adapter::mapping::{MappingMode, MappingStore};
use smallvec::SmallVec;
use vim_core::inputs::VimKey;
use vim_core::state::mode::Mode;
use vim_core::state::VimState;

/// Context passed through the key consumer pipeline.
///
/// Contains all information needed for consumers to process a key,
/// plus mutable state they can modify (e.g., `should_handle_input`).
pub struct KeyContext<'a> {
    /// The key being processed
    pub key: VimKey,

    /// Reference to Vim state (read-only in most consumers)
    pub vim_state: &'a VimState,

    /// Current mode at start of processing
    pub initial_mode: Mode,

    /// Whether input has been handled (set by consumers)
    pub input_handled: bool,

    /// Whether to skip remaining consumers
    pub stop_processing: bool,

    /// Debug/trace message collector
    pub trace_messages: Vec<String>,

    /// Mapping store for mapping lookups (shared reference via Arc)
    pub mapping_store: Arc<MappingStore>,

    /// Pending keys for mapping resolution
    /// Uses SmallVec - rarely more than 4 pending keys.
    pub pending_keys: SmallVec<[VimKey; 4]>,

    /// Whether code completion popup is active
    pub completion_active: bool,
}

impl<'a> KeyContext<'a> {
    /// Creates a new key context.
    #[must_use]
    pub fn new(
        key: VimKey,
        vim_state: &'a VimState,
        mapping_store: Arc<MappingStore>,
        pending_keys: &[VimKey],
        completion_active: bool,
    ) -> Self {
        let initial_mode = vim_state.mode();
        Self {
            key,
            vim_state,
            initial_mode,
            input_handled: false,
            stop_processing: false,
            trace_messages: Vec::new(),
            mapping_store,
            // Copy is cheap now that VimKey is Copy
            pending_keys: SmallVec::from_slice(pending_keys),
            completion_active,
        }
    }

    /// Marks input as handled (Godot will not process it).
    pub fn mark_handled(&mut self) {
        self.input_handled = true;
    }

    /// Adds a trace message for debugging.
    pub fn trace(&mut self, msg: impl Into<String>) {
        self.trace_messages.push(msg.into());
    }

    /// Returns true if we're in a recording state.
    #[must_use]
    pub fn is_recording(&self) -> bool {
        self.vim_state.macros.recording.is_some()
    }

    /// Returns true if in Insert mode.
    #[must_use]
    pub fn is_insert_mode(&self) -> bool {
        matches!(self.initial_mode, Mode::Insert(..))
    }

    /// Returns true if in any Visual mode.
    #[must_use]
    pub fn is_visual_mode(&self) -> bool {
        matches!(self.initial_mode, Mode::Visual(_))
    }

    /// Returns true if in Normal mode.
    #[must_use]
    pub fn is_normal_mode(&self) -> bool {
        matches!(self.initial_mode, Mode::Normal)
    }

    /// Returns true if in `CmdLine` mode.
    #[must_use]
    pub fn is_cmdline_mode(&self) -> bool {
        matches!(self.initial_mode, Mode::CmdLine(_))
    }

    /// Returns the mapping mode corresponding to the current Vim mode.
    #[must_use]
    pub fn mapping_mode(&self) -> Option<MappingMode> {
        if self.is_insert_mode() {
            Some(MappingMode::Insert)
        } else if self.is_visual_mode() {
            Some(MappingMode::Visual)
        } else if self.is_cmdline_mode() {
            None
        } else {
            Some(MappingMode::Normal)
        }
    }

    /// Checks if the current key could start a mapping.
    #[must_use]
    pub fn could_start_mapping(&self) -> bool {
        if let Some(mode) = self.mapping_mode() {
            self.mapping_store.could_start_mapping(&self.key, mode)
        } else {
            false
        }
    }
}
