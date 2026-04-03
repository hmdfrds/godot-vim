//! Clipboard abstraction for testability.
//!
//! Production: [`GodotClipboard`] delegates to `DisplayServer::singleton()`.
//! Tests: [`MockClipboard`] uses an in-memory `String`.

use godot::classes::DisplayServer;
use godot::prelude::*;

/// System clipboard abstraction.
pub(crate) trait ClipboardPort {
    fn read(&self) -> String;
    fn write(&mut self, text: &str);
}

/// Production clipboard backed by Godot's [`DisplayServer`].
pub(crate) struct GodotClipboard;

impl ClipboardPort for GodotClipboard {
    fn read(&self) -> String {
        DisplayServer::singleton().clipboard_get().to_string()
    }

    fn write(&mut self, text: &str) {
        DisplayServer::singleton().clipboard_set(&GString::from(text));
    }
}

#[cfg(test)]
pub(crate) struct MockClipboard {
    pub content: String,
}

#[cfg(test)]
impl MockClipboard {
    pub fn new() -> Self {
        Self { content: String::new() }
    }
}

#[cfg(test)]
impl ClipboardPort for MockClipboard {
    fn read(&self) -> String {
        self.content.clone()
    }

    fn write(&mut self, text: &str) {
        self.content = text.to_string();
    }
}
