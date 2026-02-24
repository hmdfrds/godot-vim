//! SearchManager - Encapsulates bridge-side search state synchronization.
//!
//! This module owns the `last_synced_search` state and provides methods
//! for syncing search highlights to Godot's CodeEdit.
//!
//! ## Separation of Concerns
//! - `VimState.last_search` (pure core): The actual search pattern
//! - `SearchManager.last_synced` (bridge): Tracks what we've synced to Godot

use godot::classes::CodeEdit;
use godot::prelude::*;

/// Manages search highlight synchronization between Vim state and Godot.
///
/// This struct encapsulates the bridge-side caching logic to avoid redundant
/// Godot API calls when the search pattern hasn't changed.
#[derive(Debug, Default)]
pub struct SearchManager {
    /// Last search pattern synced to Godot (to avoid redundant updates)
    last_synced: Option<String>,
}

impl SearchManager {
    /// Creates a new SearchManager with no synced state.
    #[must_use]
    pub fn new() -> Self {
        Self { last_synced: None }
    }

    /// Syncs search highlighting with Godot's CodeEdit.
    ///
    /// Only syncs "simple" patterns (literals) to avoid misleading highlights,
    /// as Godot does not support Regex search/highlighting natively.
    ///
    /// # Arguments
    /// * `current_pattern` - The current search pattern from `VimState.last_search`
    /// * `editor` - The CodeEdit to sync highlighting to
    pub fn sync_highlight(&mut self, current_pattern: Option<&str>, editor: &mut Gd<CodeEdit>) {
        let search_pattern = current_pattern.unwrap_or("");

        // Filter: Only sync simple (literal) patterns to Godot.
        // Regex-special characters would produce misleading highlights since
        // Godot's search is literal-only.
        #[allow(clippy::manual_pattern_char_comparison)]
        let is_simple = !search_pattern.contains(|c| {
            matches!(
                c,
                '\\' | '.' | '^' | '$' | '*' | '+' | '?' | '(' | ')' | '[' | ']' | '{' | '}' | '|'
            )
        });

        let text_to_sync = if is_simple && !search_pattern.is_empty() {
            search_pattern
        } else {
            ""
        };

        // Skip if this exact text was already synced to Godot.
        if let Some(ref last) = self.last_synced {
            if last == text_to_sync {
                return;
            }
        }

        editor.set_search_text(text_to_sync);

        // Cache the transformed text sent to the editor, not the original pattern.
        self.last_synced = Some(text_to_sync.to_string());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_manager_has_no_synced_state() {
        let manager = SearchManager::new();
        assert!(manager.last_synced.is_none());
    }
}
