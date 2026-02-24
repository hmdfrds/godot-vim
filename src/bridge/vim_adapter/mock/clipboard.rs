//! Mock clipboard for testing without Godot runtime.
//!
//! Simulates Godot's DisplayServer clipboard operations.

/// In-memory clipboard simulation.
#[derive(Debug, Clone, Default)]
pub struct MockClipboard {
    content: String,
}

impl MockClipboard {
    /// Creates a new empty clipboard.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets the clipboard content.
    pub fn set(&mut self, content: &str) {
        self.content = content.to_string();
    }

    /// Gets the clipboard content.
    #[must_use]
    pub fn get(&self) -> &str {
        &self.content
    }

    /// Clears the clipboard.
    pub fn clear(&mut self) {
        self.content.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_clipboard_set_get() {
        let mut clipboard = MockClipboard::new();
        clipboard.set("hello");
        assert_eq!(clipboard.get(), "hello");
    }

    #[test]
    fn test_clipboard_clear() {
        let mut clipboard = MockClipboard::new();
        clipboard.set("hello");
        clipboard.clear();
        assert_eq!(clipboard.get(), "");
    }
}
