//! Compound actions returned by the effect dispatch layer for multi-step
//! operations that require driving the engine in a loop (e.g., `:norm`)
//! or that require controller-level Godot access (e.g., window navigation).

use crate::types::RemapPolicy;

/// Zero-based line index for compound action ranges (Godot convention).
///
/// Vim's 1-based `:norm` range is converted to 0-based at `CompoundAction`
/// creation in `dispatch.rs`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct LineNumber(usize);

#[allow(dead_code)] // Used by dispatch.rs for compound action construction.
impl LineNumber {
    #[must_use]
    pub(crate) const fn new(val: usize) -> Self {
        Self(val)
    }
    #[must_use]
    pub(crate) const fn get(self) -> usize {
        self.0
    }
}

/// Window navigation action produced by `Ctrl-W` window commands.
///
/// The controller handles these with Godot scene tree access
/// (via `navigation::window`) after dispatch completes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WindowNavAction {
    MoveLeft,
    MoveRight,
    MoveUp,
    MoveDown,
    CycleNext,
    CyclePrev,
    CloseTab,
}

/// Actions that cannot be completed inline during dispatch. The dispatcher
/// collects these and the controller handles them after the dispatch loop:
/// `:norm` re-drives the engine per-line, window nav traverses the scene tree.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum CompoundAction {
    /// `:norm` command applied to a line range.
    NormCommand {
        start_line: LineNumber,
        end_line: LineNumber,
        keys: String,
        remap: RemapPolicy,
    },
    /// Window navigation requiring Godot scene tree access.
    WindowNav { action: WindowNavAction },
}
