//! Vim-like navigation across Godot's editor UI.
//!
//! Two navigation layers:
//! - **Cross-panel** (`Ctrl+hjkl`): directional movement between major editor
//!   regions (docks, code editors) using spatial cone scoring.
//! - **Intra-dock** (plain `hjkl`): Vim-style item navigation within Tree,
//!   ItemList, and RichTextLabel dock controls.
//!
//! Entirely shell-side — vim-core has no knowledge of Godot's dock layout.
//!
//! These are **executors**, not a dispatcher. Which key runs which one, on
//! which surface, and whether the event is consumed all live in
//! `crate::actions`: every function here is reached by name through an
//! `ActionSpec` resolved from a per-surface binding trie. `focus.rs` and its
//! `classify_focus` / `FocusContext` were deleted in the same commit that
//! removed their last caller; the surface forest in
//! `crate::actions::providers` answers "where is this keystroke" now, and
//! `DockKind` moved to [`dock`] because the Tree-versus-ItemList signal
//! arity is a real Godot API distinction rather than a dispatch convenience.

mod cycle;
pub(crate) mod dock;
pub(crate) mod dock_nav;
mod dock_search;
pub(crate) mod filesystem_explorer;
pub(crate) mod window;

pub(crate) use cycle::handle_window_nav_action;
pub(crate) use dock_search::{enter_submits, find_sibling_nav_control};
pub(crate) use filesystem_explorer::{is_in_filesystem_dock, FileSystemExplorer};
pub(crate) use window::handle_window_nav;
