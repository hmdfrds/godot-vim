//! Leaf-level types shared across modules.
//!
//! These types live at the bottom of the dependency graph so that `bridge`,
//! `state`, `ui`, `effects`, and `controller` can all import them without
//! creating circular dependencies. Each type exists to replace a primitive
//! (bool, tuple, Option pair) with a named, mis-use-resistant alternative.

use compact_str::CompactString;
use vim_core::primitives::{CommandLinePrompt, Direction, Mode};

// ─────────────────────────────────────────────────────────────────────────────
// ForceOverride
// ─────────────────────────────────────────────────────────────────────────────

/// Boolean-replacement for the Vim `!` modifier.
///
/// `handle_quit(id, editor, ForceOverride::Force)` is self-documenting
/// where `handle_quit(id, editor, true)` leaves the reader guessing
/// which of several booleans "true" refers to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ForceOverride {
    Normal,
    Force,
}

impl ForceOverride {
    #[must_use]
    pub(crate) const fn is_force(self) -> bool {
        matches!(self, Self::Force)
    }
}

impl From<bool> for ForceOverride {
    fn from(force: bool) -> Self {
        if force {
            Self::Force
        } else {
            Self::Normal
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// CharLineCol / GraphemeLineCol -- the two column-counting conventions
// ─────────────────────────────────────────────────────────────────────────────

/// Godot-facing `(line, col)` where columns count Unicode codepoints.
///
/// This is the unit CodeEdit's API speaks. Distinct from [`GraphemeLineCol`]
/// to prevent silently mixing the two column conventions -- a `CharLineCol`
/// cannot be passed where a `GraphemeLineCol` is expected without an explicit
/// conversion through the codec layer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct CharLineCol {
    pub(crate) line: i32,
    pub(crate) col: i32,
}

impl CharLineCol {
    #[must_use]
    pub(crate) const fn new(line: i32, col: i32) -> Self {
        Self { line, col }
    }
}

/// Engine-facing `(line, col)` where columns count grapheme clusters.
///
/// This is the unit vim-core's `Document` trait speaks. See [`CharLineCol`]
/// for the Godot-facing counterpart. The two types are intentionally
/// incompatible to force explicit conversion at the bridge boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct GraphemeLineCol {
    pub(crate) line: i32,
    pub(crate) col: i32,
}

impl GraphemeLineCol {
    #[inline]
    #[must_use]
    pub(crate) const fn new(line: i32, col: i32) -> Self {
        Self { line, col }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// PixelPos
// ─────────────────────────────────────────────────────────────────────────────

/// Screen-pixel coordinate for cursor overlay positioning.
///
/// Named fields prevent the x/y swap bugs that anonymous `(i32, i32)` tuples
/// invited in the `corrected_col_x` -> overlay rendering pipeline.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PixelPos {
    pub(crate) x: i32,
    pub(crate) y: i32,
}

impl PixelPos {
    #[must_use]
    pub(crate) const fn new(x: i32, y: i32) -> Self {
        Self { x, y }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// MatchRange
// ─────────────────────────────────────────────────────────────────────────────

/// Start/end range in [`CharLineCol`] coordinates for substitute preview
/// (`:s` inccommand) and vimdebug range highlighting.
///
/// The optional `replacement` field carries the post-substitution text for
/// live `:s` preview (inccommand). Vimdebug ranges leave it `None`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MatchRange {
    pub(crate) start: CharLineCol,
    pub(crate) end: CharLineCol,
    /// The replacement text that would be produced by `:s`. `None` for
    /// vimdebug range highlights and other non-substitute uses.
    pub(crate) replacement: Option<CompactString>,
}

impl MatchRange {
    #[must_use]
    #[allow(dead_code)] // Used by vimdebug range annotation; currently disabled in new pipeline.
    pub(crate) const fn new(start: CharLineCol, end: CharLineCol) -> Self {
        Self {
            start,
            end,
            replacement: None,
        }
    }

    #[must_use]
    pub(crate) fn with_replacement(
        start: CharLineCol,
        end: CharLineCol,
        replacement: CompactString,
    ) -> Self {
        Self {
            start,
            end,
            replacement: Some(replacement),
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// HighlightYank
// ─────────────────────────────────────────────────────────────────────────────

/// Payload for the brief flash animation on yanked text.
/// Bundles the range with its duration so the UI layer can start the
/// animation without querying any additional state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct HighlightYank {
    pub(crate) start: CharLineCol,
    pub(crate) end: CharLineCol,
    pub(crate) duration_ms: u32,
}

impl HighlightYank {
    #[must_use]
    #[allow(dead_code)] // Constructor for yank highlight — currently unused after vim-core removed HighlightYank effect
    pub(crate) const fn new(start: CharLineCol, end: CharLineCol, duration_ms: u32) -> Self {
        Self {
            start,
            end,
            duration_ms,
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// RemapPolicy
// ─────────────────────────────────────────────────────────────────────────────

/// Controls whether `:norm` recursively expands mappings (`:norm`) or
/// executes keys literally (`:norm!`).
///
/// A bare `bool remap` parameter is ambiguous at call sites -- "remap: true"
/// could mean "should remap" or "was already remapped."
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RemapPolicy {
    /// `:norm` — recursively expand mappings during execution.
    Remap,
    /// `:norm!` — execute keys literally without mapping expansion.
    NoRemap,
}

impl From<bool> for RemapPolicy {
    fn from(remap: bool) -> Self {
        if remap {
            Self::Remap
        } else {
            Self::NoRemap
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// StatusMessage
// ─────────────────────────────────────────────────────────────────────────────

/// Tri-state status bar message that makes the illegal state unrepresentable.
///
/// Previously `(Option<String>, bool)` where `(None, true)` was expressible
/// but meaningless. This enum collapses the flag into the variant, and drives
/// display color (red for `Error`, default for `Info`) from the type itself.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) enum StatusMessage {
    #[default]
    None,
    /// e.g. "3 lines yanked" -- rendered in default color.
    Info(CompactString),
    /// e.g. "E486: Pattern not found" -- rendered in red.
    Error(CompactString),
}

impl StatusMessage {
    #[must_use]
    pub(crate) fn text(&self) -> Option<&str> {
        match self {
            Self::None => Option::None,
            Self::Info(msg) | Self::Error(msg) => Some(msg.as_str()),
        }
    }

    #[must_use]
    pub(crate) const fn is_error(&self) -> bool {
        matches!(self, Self::Error(_))
    }

    #[must_use]
    #[allow(dead_code)] // Reserved for status bar rendering
    pub(crate) const fn is_none(&self) -> bool {
        matches!(self, Self::None)
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// UiSnapshot and sub-structs -- data contract between controller and UI
// ─────────────────────────────────────────────────────────────────────────────

/// Command-line prompt, input buffer, and cursor position for the status bar.
#[derive(Debug, Clone)]
pub(crate) struct CommandLineState {
    /// `None` when not in command-line mode; otherwise Ex / SearchForward / SearchBackward.
    pub(crate) prompt: Option<CommandLinePrompt>,
    pub(crate) input: CompactString,
    /// Char offset within `input` (not byte offset).
    pub(crate) cursor: usize,
}

/// Diagnostic overlay state for `:vimdebug`.
///
/// Sum type replacing four `Option` fields. The previous struct had 16
/// representable combinations (`2^4`) but only three were legal: all `None`
/// (inactive), provenance+effects+range without step (watch mode), or all
/// four populated (step mode). This enum collapses to exactly those three.
#[derive(Debug, Clone, Default)]
pub(crate) enum VimdebugSnapshot {
    /// No vimdebug session active. All display annotations suppressed.
    #[default]
    Inactive,
    /// `:vimdebug watch` — status bar shows provenance and effect summary.
    Watch {
        provenance: CompactString,
        effects: CompactString,
        range: Option<MatchRange>,
    },
    /// `:vimdebug step` — additionally shows step navigation status line.
    Step {
        provenance: CompactString,
        effects: CompactString,
        range: Option<MatchRange>,
        step_status: CompactString,
    },
}

impl VimdebugSnapshot {
    /// Whether any vimdebug annotations should be displayed.
    #[must_use]
    pub(crate) const fn is_active(&self) -> bool {
        !matches!(self, Self::Inactive)
    }

    /// Provenance string (e.g. "dd", "ciw") if vimdebug is active.
    #[must_use]
    pub(crate) fn provenance(&self) -> Option<&str> {
        match self {
            Self::Inactive => None,
            Self::Watch { provenance, .. } | Self::Step { provenance, .. } => {
                Some(provenance.as_str())
            }
        }
    }

    /// Effect summary string if vimdebug is active.
    #[must_use]
    pub(crate) fn effects(&self) -> Option<&str> {
        match self {
            Self::Inactive => None,
            Self::Watch { effects, .. } | Self::Step { effects, .. } => Some(effects.as_str()),
        }
    }

    /// Highlighted range (substitute preview or vimdebug range) if present.
    #[must_use]
    pub(crate) fn range(&self) -> Option<&MatchRange> {
        match self {
            Self::Inactive => None,
            Self::Watch { range, .. } | Self::Step { range, .. } => range.as_ref(),
        }
    }

    /// Step navigation status line, only present in step mode.
    #[must_use]
    pub(crate) fn step_status(&self) -> Option<&str> {
        match self {
            Self::Step { step_status, .. } => Some(step_status.as_str()),
            _ => None,
        }
    }
}

/// Immutable snapshot of engine state, built once per keystroke by the
/// controller and consumed by the UI coordinator.
///
/// This is the sole data contract between controller and UI -- the UI layer
/// never touches engine internals directly. All fields are pre-converted to
/// display-ready types (CharLineCol, CompactString, etc.).
#[derive(Debug)]
pub(crate) struct UiSnapshot {
    pub(crate) mode: Mode,
    pub(crate) message: StatusMessage,
    pub(crate) cmdline: CommandLineState,
    /// Register character being recorded (e.g. 'q'), or `None`.
    pub(crate) recording_register: Option<char>,
    pub(crate) search_pattern: Option<(CompactString, Direction)>,
    /// `false` after `:noh` (highlights suppressed but pattern retained);
    /// `true` after any new search.
    pub(crate) hlsearch_enabled: bool,
    /// Engine head in visual mode. Godot's caret sits at the exclusive
    /// selection end, so the cursor overlay needs the engine's actual head
    /// position to render correctly.
    pub(crate) visual_head: Option<CharLineCol>,
    pub(crate) pending_keys: CompactString,
    /// Showcmd string (e.g. "3d", "ci") combining parser and mapping state.
    pub(crate) pending_command: CompactString,
    /// `Some(vec)` = update/clear the inccommand overlay; `None` = no change.
    pub(crate) substitute_preview: Option<Vec<MatchRange>>,
    pub(crate) vimdebug: VimdebugSnapshot,
    pub(crate) highlight_yank: Option<HighlightYank>,
}
