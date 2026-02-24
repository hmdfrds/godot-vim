//! Mapping State - Tracks pending keys for custom mappings.
//!
//! Accumulate keys, start timer on first key,
//! execute mapping or flush as literal on timeout.
//!
//! # Timeout Handling
//!
//! Timeout is managed by Godot's `Timer` node in `VimController`.
//! This avoids polling and integrates with Godot's event loop.
//!
//! In **tests**, `start_time` and `is_timed_out()` are available for verifying
//! timeout logic without Godot runtime.

use smallvec::SmallVec;
use vim_core::inputs::VimKey;

/// Tracks pending keys while waiting for a mapping to complete.
///
/// # Timer Logic (like Vim's `timeoutlen`)
///
/// When the first key of a potential mapping is pressed:
/// 1. Start timing
/// 2. If another key arrives before timeout and completes a mapping -> execute mapping
/// 3. If timeout expires -> flush pending keys as literal input
/// 4. If no mapping matches and no prefix exists -> flush immediately
#[derive(Debug)]
pub struct MappingState {
    /// Keys accumulated while waiting for mapping completion
    /// Uses SmallVec - rarely more than 4 pending keys.
    pending_keys: SmallVec<[VimKey; 4]>,

    /// Configured timeout in milliseconds (test-only)
    #[cfg(test)]
    timeoutlen: u64,

    /// When the first key was pressed (test-only for timeout verification)
    #[cfg(test)]
    start_time: Option<std::time::Instant>,
}

impl MappingState {
    /// Creates a new `MappingState`.
    ///
    /// In production, timeout is handled externally.
    /// In tests, a default timeout can be assumed or set.
    #[must_use]
    pub fn new() -> Self {
        Self {
            pending_keys: SmallVec::new(),
            #[cfg(test)]
            timeoutlen: 1000,
            #[cfg(test)]
            start_time: None,
        }
    }

    /// Updates the timeout length (Test only).
    #[cfg(test)]
    pub fn set_timeoutlen(&mut self, ms: u64) {
        self.timeoutlen = ms;
    }

    /// Adds a key to the pending sequence.
    ///
    /// In tests, this also starts the timeout timer.
    /// In production, `VimController` manages the Godot Timer.
    pub fn add_key(&mut self, key: VimKey) {
        #[cfg(test)]
        if self.pending_keys.is_empty() {
            self.start_time = Some(std::time::Instant::now());
        }
        self.pending_keys.push(key);
    }

    /// Returns the pending keys without modifying state.
    #[must_use]
    pub fn pending_keys(&self) -> &[VimKey] {
        &self.pending_keys
    }

    /// Returns true if there are pending keys.
    #[must_use]
    pub fn has_pending(&self) -> bool {
        !self.pending_keys.is_empty()
    }

    /// Returns true if the timeout has elapsed since the first key.
    ///
    /// **Test-only**: In production, Godot's Timer handles timeout events.
    #[cfg(test)]
    #[must_use]
    pub fn is_timed_out(&self) -> bool {
        use std::time::Duration;
        if let Some(start) = self.start_time {
            start.elapsed() > Duration::from_millis(self.timeoutlen)
        } else {
            false
        }
    }

    /// Flushes pending keys and resets state. Returns the keys.
    pub fn flush(&mut self) -> SmallVec<[VimKey; 4]> {
        #[cfg(test)]
        {
            self.start_time = None;
        }
        std::mem::take(&mut self.pending_keys)
    }

    /// Resets state without returning keys (use when mapping executed).
    pub fn reset(&mut self) {
        self.pending_keys.clear();
        #[cfg(test)]
        {
            self.start_time = None;
        }
    }
}

impl Default for MappingState {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vim_core::inputs::{KeyCode, VimModifiers};

    fn char_key(c: char) -> VimKey {
        VimKey::new(KeyCode::Char(c), VimModifiers::NONE)
    }

    #[test]
    fn test_new_state_is_empty() {
        let state = MappingState::new();
        assert!(!state.has_pending());
        assert!(!state.is_timed_out());
    }

    #[test]
    fn test_add_key_starts_timer() {
        let mut state = MappingState::new();
        state.add_key(char_key('j'));
        assert!(state.has_pending());
        assert!(!state.is_timed_out());
        assert_eq!(state.pending_keys().len(), 1);
    }

    #[test]
    fn test_flush_returns_keys_and_resets() {
        let mut state = MappingState::new();
        state.add_key(char_key('j'));
        state.add_key(char_key('j'));

        let keys = state.flush();
        assert_eq!(keys.len(), 2);
        assert!(!state.has_pending());
    }

    #[test]
    fn test_reset_clears_without_returning() {
        let mut state = MappingState::new();
        state.add_key(char_key('j'));
        state.reset();

        assert!(!state.has_pending());
    }

    #[test]
    fn test_timeout_with_zero_ms() {
        let mut state = MappingState::new();
        state.set_timeoutlen(0);
        state.add_key(char_key('j'));
        // With 0ms timeout, should be immediately timed out
        assert!(state.is_timed_out());
    }
}
