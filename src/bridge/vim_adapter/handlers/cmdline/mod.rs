//! Command line handler trait for `VimController`.
//!
//! Handles command line submission (`:`, `/`, `?`).

mod incsearch;
mod search_target;
mod submit;
mod substitute_preview;

/// Trait for handling command line submission.
pub trait CmdLineHandler {
    /// Handle command line submission (Enter pressed in `:`, `/`, or `?` mode).
    fn handle_cmd_submitted(&mut self, text: &str);
}

/// Trait for incremental search highlighting.
///
/// Provides real-time highlighting during `/`, `?`, and `:s/` commands.
pub trait IncsearchHandler {
    /// Update incremental search highlights based on command line text.
    ///
    /// Parses the pattern from search commands (`/pattern`, `?pattern`)
    /// and substitute commands (`:s/pattern/...`) then applies highlighting.
    /// For substitute commands, also updates the status bar with match count.
    fn update_incsearch_highlights(&mut self, text: &str);

    /// Clear incremental search highlights (called on Escape).
    fn clear_incsearch_highlights(&mut self);
}
