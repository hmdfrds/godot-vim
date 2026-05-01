//! Vim-like navigation across Godot's editor UI.
//!
//! Two navigation layers:
//! - **Cross-panel** (`Ctrl+hjkl`): directional movement between major editor
//!   regions (docks, code editors) using spatial cone scoring.
//! - **Intra-dock** (plain `hjkl`): Vim-style item navigation within Tree,
//!   ItemList, and RichTextLabel dock controls.
//!
//! Entirely shell-side — vim-core has no knowledge of Godot's dock layout.
//! Uses a fixed keyset (not user-customizable via `:map`) to keep the
//! focus-management boundary simple and predictable.

mod cycle;
mod dock;
mod dock_nav;
mod dock_search;
pub(crate) mod filesystem_explorer;
mod focus;
pub(crate) mod window;

pub(crate) use cycle::handle_window_nav_action;
pub(crate) use dock::{handle_dock_input, handle_search_input};
pub(crate) use filesystem_explorer::{is_in_filesystem_dock, FileSystemExplorer};
pub(crate) use focus::{classify_focus, FocusContext};
pub(crate) use window::handle_window_nav;
