//! Custom keymapping system.
//!
//! Implements Vim-style key mappings with timeout support (like `timeoutlen`).
//! Allows mappings like `jj` -> `<Esc>` for quick insert mode exit.
//!
//! ```text
//! ┌─────────────────────────────────────────────────────┐
//! │         PURE CORE (vim::core::mapping)              │
//! │  KeyTrie, MappingStore, KeyMapping, MappedAction    │
//! └─────────────────────────────────────────────────────┘
//!                        ▲
//!                        │ Re-export
//! ┌──────────────────────┴──────────────────────────────┐
//! │         SHELL (this module)                         │
//! │  GodotMappingLoader: Loads from ProjectSettings     │
//! │  MappingState: Tracks pending keys & timeout        │
//! │  MappingPanel: Dock panel UI for managing mappings  │
//! └─────────────────────────────────────────────────────┘
//! ```

mod loader;
mod panel;
mod state;

// Re-export the vim-core mapping types consumed by this module.
pub use vim_core::inputs::mapping::{MappedAction, MappingLookup, MappingMode, MappingStore};

// Shell-layer exports
pub use loader::GodotMappingLoader;
pub use panel::MappingPanel;
pub use state::MappingState;
