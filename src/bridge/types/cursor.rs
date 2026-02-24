//! Cursor position types for the godot-vim shell.
//!
//! # ADR: Own Position Type
//!
//! **Context**: vim-core's `Position` has rkyv serialization, `derive_more::Display`,
//! and many internal methods. The shell only needs (line, col) for caret placement.
//!
//! **Decision**: `CursorPos` is a minimal (line, col) newtype. The adapter converts.
//!
//! **Consequence**: Shell components never import vim-core's `Position`. If vim-core
//! changes `Position`, only `vim_adapter/convert.rs` updates.

use std::cmp::Ordering;

/// A cursor position in a document (0-indexed line and column).
///
/// # Invariant
///
/// `line` and `col` are 0-indexed and refer to logical (unwrapped) positions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Hash)]
pub struct CursorPos {
    /// Line number (0-indexed).
    pub line: usize,
    /// Column number (0-indexed).
    pub col: usize,
}

impl CursorPos {
    /// Creates a new cursor position.
    #[inline]
    #[must_use]
    pub const fn new(line: usize, col: usize) -> Self {
        Self { line, col }
    }

    /// Origin position (0, 0).
    #[inline]
    #[must_use]
    pub const fn origin() -> Self {
        Self { line: 0, col: 0 }
    }

    /// Start of a given line.
    #[inline]
    #[must_use]
    pub const fn line_start(line: usize) -> Self {
        Self { line, col: 0 }
    }
}

impl Ord for CursorPos {
    #[inline]
    fn cmp(&self, other: &Self) -> Ordering {
        self.line.cmp(&other.line).then(self.col.cmp(&other.col))
    }
}

impl PartialOrd for CursorPos {
    #[inline]
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl From<(usize, usize)> for CursorPos {
    #[inline]
    fn from((line, col): (usize, usize)) -> Self {
        Self::new(line, col)
    }
}

impl From<CursorPos> for (usize, usize) {
    #[inline]
    fn from(pos: CursorPos) -> Self {
        (pos.line, pos.col)
    }
}

impl std::fmt::Display for CursorPos {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:{}", self.line, self.col)
    }
}
