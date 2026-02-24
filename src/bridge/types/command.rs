//! Editor command types for the godot-vim shell.
//!
//! # ADR: Unified Command Enum
//!
//! **Context**: vim-core splits commands across `ShellRequest` (30+ variants)
//! and `ExternalCommand` (40+ variants). The shell must dispatch both, leading to
//! duplicate match arms and scattered handling across 24 handler files.
//!
//! **Decision**: `EditorCommand` unifies all shell-visible commands into a single
//! enum organized by category. The adapter translates both `ShellRequest` and
//! `ExternalCommand` into this type.
//!
//! **Consequence**: Shell dispatch is a single `match` on `EditorCommand`.
//! Adding new commands requires updating the adapter converter + one match arm.

use super::cursor::CursorPos;
use super::mode::EditorMode;
use vim_core::inputs::commands::motions::Motion;

/// A command the shell must execute.
///
/// These are the result of vim-core processing a key — the adapter translates
/// `ShellRequest` and `ExternalCommand` into this unified type.
///
/// # Categories
///
/// Commands are grouped by the subsystem that handles them:
/// - **Mode**: Mode transitions
/// - **Cursor**: Cursor movement and jumps
/// - **File**: Save/quit/buffer operations
/// - **Edit**: Text manipulation
/// - **Search**: Find and replace
/// - **Fold**: Code folding
/// - **Completion**: Autocompletion
/// - **Debug**: Breakpoints, stepping
/// - **Navigation**: Dock/window navigation
/// - **UI**: Status bar, prompts
/// - **Clipboard**: System clipboard
/// - **Macro**: Recording and playback
#[derive(Debug, Clone, PartialEq)]
pub enum EditorCommand {
    // ─────────────────────────────────────────────────────────────────────────
    // Mode Transitions
    // ─────────────────────────────────────────────────────────────────────────
    /// Change the editor mode.
    ModeChange {
        new_mode: EditorMode,
        previous_mode: Option<EditorMode>,
    },

    // ─────────────────────────────────────────────────────────────────────────
    // Cursor & Navigation
    // ─────────────────────────────────────────────────────────────────────────
    /// Jump to an absolute position.
    JumpTo(CursorPos),
    /// Go to a specific line.
    GoToLine {
        line: u32,
    },
    /// Go to definition under cursor.
    GotoDefinition,
    /// Show documentation popup.
    ShowDocumentation,

    // ─────────────────────────────────────────────────────────────────────────
    // File Operations
    // ─────────────────────────────────────────────────────────────────────────
    /// Save current file.
    Save,
    /// Close current tab.
    Quit,
    /// Save and close.
    SaveQuit,
    /// Close without saving.
    QuitNoSave,
    /// Reopen buffer from disk.
    BufferReopen,

    // ─────────────────────────────────────────────────────────────────────────
    // Buffer Navigation
    // ─────────────────────────────────────────────────────────────────────────
    /// Next buffer tab.
    BufferNext,
    /// Previous buffer tab.
    BufferPrev,
    /// Jump to buffer by index (1-indexed).
    BufferGoto(u32),

    // ─────────────────────────────────────────────────────────────────────────
    // Undo / Redo
    // ─────────────────────────────────────────────────────────────────────────
    /// Undo N changes.
    Undo {
        count: usize,
    },
    /// Redo N changes.
    Redo {
        count: usize,
    },

    // ─────────────────────────────────────────────────────────────────────────
    // Text Editing (Shell-Side)
    // ─────────────────────────────────────────────────────────────────────────
    /// Type a character in insert mode.
    TypeChar(char),
    /// Insert text.
    InsertText(String),
    /// Backspace in insert mode.
    Backspace,
    /// Replace character under cursor.
    ReplaceChar(char),
    /// Open new line below.
    OpenLineBelow {
        count: u32,
    },
    /// Open new line above.
    OpenLineAbove {
        count: u32,
    },
    /// Append mode entry.
    Append {
        at_eol: bool,
    },
    /// Insert at first non-blank.
    InsertAtFirstNonBlank,
    /// Go to last insert position and enter insert mode.
    InsertAtLastPosition,
    /// Increment number under cursor.
    IncrementNumber {
        count: u32,
    },
    /// Decrement number under cursor.
    DecrementNumber {
        count: u32,
    },

    // ─────────────────────────────────────────────────────────────────────────
    // Paste / Registers
    // ─────────────────────────────────────────────────────────────────────────
    /// Paste from register.
    Paste {
        after: bool,
        register: Option<char>,
        count: usize,
        adjust_indent: bool,
        move_cursor_to_end: bool,
    },
    /// Ctrl-R register insertion in insert mode.
    InsertRegister {
        name: char,
        literally: bool,
    },
    /// List all registers.
    ListRegisters,

    // ─────────────────────────────────────────────────────────────────────────
    // Marks & Jumps
    // ─────────────────────────────────────────────────────────────────────────
    /// Set a mark at current cursor.
    SetMark(char),
    /// Jump to a mark.
    JumpToMark {
        name: char,
        exact: bool,
    },

    // ─────────────────────────────────────────────────────────────────────────
    // Search & Replace
    // ─────────────────────────────────────────────────────────────────────────
    /// Search for pattern.
    Search {
        pattern: String,
        forward: bool,
    },
    /// Search next occurrence.
    SearchNext,
    /// Search previous occurrence.
    SearchPrev,
    /// Search word under cursor.
    SearchWordForward,
    /// Search word backward.
    SearchWordBackward,
    /// Partial word search forward (g*).
    SearchWordPartialForward,
    /// Partial word search backward (g#).
    SearchWordPartialBackward,
    /// Find and replace.
    FindAndReplace {
        pattern: String,
        replacement: String,
        flags: String,
    },
    /// Repeat substitute on all lines (g&).
    RepeatSubstituteAllLines,

    // ─────────────────────────────────────────────────────────────────────────
    // Folding
    // ─────────────────────────────────────────────────────────────────────────
    FoldOpen,
    FoldClose,
    FoldToggle,
    FoldAll,
    UnfoldAll,

    // ─────────────────────────────────────────────────────────────────────────
    // Completion
    // ─────────────────────────────────────────────────────────────────────────
    CompletionNext,
    CompletionPrev,
    CompletionAccept,
    CompletionCancel,

    // ─────────────────────────────────────────────────────────────────────────
    // Macros
    // ─────────────────────────────────────────────────────────────────────────
    /// Macro recording started.
    MacroStarted(char),
    /// Macro recording stopped.
    MacroStopped,

    // ─────────────────────────────────────────────────────────────────────────
    // Motion Dispatch
    // ─────────────────────────────────────────────────────────────────────────
    /// Execute a Vim motion using adapter-side viewport/capability context.
    Motion {
        motion: Motion,
        count: usize,
    },

    // ─────────────────────────────────────────────────────────────────────────
    // Internal Operations
    // ─────────────────────────────────────────────────────────────────────────
    /// Mark undo checkpoint.
    UndoSync,
    /// Disable undo sync.
    UndoNoSync,
    /// Begin block insert (Ctrl-V I).
    BeginBlockInsert {
        lines: (usize, usize),
        col: usize,
        origin: CursorPos,
    },
    /// Begin block append (Ctrl-V A).
    BeginBlockAppend {
        lines: (usize, usize),
        end_col: usize,
        origin: CursorPos,
    },
    /// Finish block insert.
    FinishBlockInsert {
        lines: (usize, usize),
        col: usize,
        text: String,
        origin: CursorPos,
    },
    /// Finish block append.
    FinishBlockAppend {
        lines: (usize, usize),
        col: usize,
        text: String,
        origin: CursorPos,
    },
    /// Block insert preview (live update).
    BlockInsertPreview {
        lines: (usize, usize),
        col: usize,
        text: String,
    },
    /// Block insert backspace.
    BlockInsertBackspace {
        lines: (usize, usize),
        col: usize,
        text: String,
    },

    // ─────────────────────────────────────────────────────────────────────────
    // Viewport
    // ─────────────────────────────────────────────────────────────────────────
    /// Scroll the viewport.
    ScrollWindow {
        up: bool,
    },
    /// Update viewport top line.
    ViewportUpdate {
        top_line: usize,
    },

    // ─────────────────────────────────────────────────────────────────────────
    // Clipboard
    // ─────────────────────────────────────────────────────────────────────────
    /// Set system clipboard content.
    ClipboardSet(String),

    // ─────────────────────────────────────────────────────────────────────────
    // UI & Messages
    // ─────────────────────────────────────────────────────────────────────────
    /// Display a message in status/cmdline.
    Message(String),
    /// Show expression register prompt.
    ShowExpressionPrompt,

    // ─────────────────────────────────────────────────────────────────────────
    // Repeat
    // ─────────────────────────────────────────────────────────────────────────
    /// Repeat last change (.).
    Repeat {
        count: usize,
    },
    /// Exit insert mode with text to repeat.
    ExitInsertMode {
        text: String,
        count: usize,
    },

    // ─────────────────────────────────────────────────────────────────────────
    // Scripting & Advanced
    // ─────────────────────────────────────────────────────────────────────────
    /// Source a vim script file.
    Source {
        path: String,
    },
    /// Read file or command output.
    Read {
        command: bool,
        path: String,
    },
    /// Execute last Ex command.
    ExecuteLastEx,
    /// Execute a register as commands.
    ExecuteRegister {
        register: char,
    },
    /// Custom extension command.
    Custom {
        cmd: String,
        args: Vec<String>,
    },
    /// Sleep for N milliseconds.
    Sleep {
        milliseconds: u64,
    },
}
