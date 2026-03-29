//! Applies search-related effects: set search pattern for CodeEdit's built-in
//! highlight, and manage highlight match ranges.

use vim_core::primitives::Range;

use crate::state::GlobalState;

/// `:noh` — suppress search highlighting without clearing the pattern.
/// `n`/`N` still use the preserved pattern; the next `/` or `?` re-enables
/// highlighting via `SetSearchPattern`.
pub(crate) fn handle_clear_highlights(globals: &mut GlobalState) {
    log::trace!("clear_highlights: hlsearch disabled");
    globals.set_hlsearch_enabled(false);
}

/// No-op: match highlighting is handled via pull-model in the rendering layer.
pub(super) fn handle_highlight_matches(ranges: &[Range]) {
    log::trace!(
        "HighlightMatches received with {} range(s) — ignored (pull-model sufficient)",
        ranges.len()
    );
}
