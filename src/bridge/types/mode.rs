//! Editor mode types for the godot-vim shell.
//!
//! # ADR: Simplified Mode Enum
//!
//! **Context**: vim-core's `Mode` has 40+ variants including all pending states
//! (OperatorPending, FindCharPending, etc.). The shell only needs to know the
//! *user-visible* mode for UI rendering (cursor color, status bar, input routing).
//!
//! **Decision**: `EditorMode` exposes only the 8 user-visible modes.
//! Pending sub-states remain internal to `vim_adapter/`.
//!
//! **Consequence**: Shell components are decoupled from vim-core's state machine.
//! Mode changes are communicated via `EditorMode`, not raw `Mode`.

/// User-visible editor mode.
///
/// This is the shell's view of the mode — it collapses vim-core's 40+ internal
/// states into the modes exposed to the user.
///
/// # Invariant
///
/// Every `EditorMode` variant maps to exactly one cursor color and status bar text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EditorMode {
    /// Default navigation mode.
    Normal,
    /// Text input mode.
    Insert,
    /// One-shot normal command from insert mode (Ctrl-O).
    InsertNormal,
    /// Character-wise visual selection.
    Visual,
    /// Line-wise visual selection.
    VisualLine,
    /// Block (rectangular) visual selection.
    VisualBlock,
    /// Overwrite mode (R).
    Replace,
    /// Command-line mode (`:`, `/`, `?`).
    CmdLine(CmdLineKind),
    /// Macro recording is active.
    Recording { register: char },
    /// An operator is pending (d, c, y, etc.) — shown in status bar.
    OperatorPending,
}

/// Kind of command-line input.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CmdLineKind {
    /// Ex command (`:`)
    Ex,
    /// Forward search (`/`)
    SearchForward,
    /// Backward search (`?`)
    SearchBackward,
}

impl EditorMode {
    /// Display name for the status bar.
    #[must_use]
    pub const fn display_name(self) -> &'static str {
        match self {
            Self::Normal => "NORMAL",
            Self::Insert => "INSERT",
            Self::InsertNormal => "INSERT(norm)",
            Self::Visual => "VISUAL",
            Self::VisualLine => "V-LINE",
            Self::VisualBlock => "V-BLOCK",
            Self::Replace => "REPLACE",
            Self::CmdLine(CmdLineKind::Ex) => "COMMAND",
            Self::CmdLine(CmdLineKind::SearchForward) => "SEARCH /",
            Self::CmdLine(CmdLineKind::SearchBackward) => "SEARCH ?",
            Self::Recording { .. } => "RECORDING",
            Self::OperatorPending => "OP-PENDING",
        }
    }
}

impl std::fmt::Display for EditorMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.display_name())
    }
}
