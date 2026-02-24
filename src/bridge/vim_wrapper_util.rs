//! Utility functions for `VimController`.
//!
//! Cursor extraction logic lives in `vim_adapter::controller::cursor` to keep
//! all `vim_core` type usage inside the adapter boundary.

use crate::bridge::vim_adapter::controller::LifecycleTrait;
use crate::bridge::vim_wrapper::VimController;

use godot::classes::CodeEdit;
use godot::prelude::*;

impl VimController {
    /// Attach to a CodeEdit editor instance.
    pub fn attach(&mut self, editor: Gd<CodeEdit>) {
        self.attach_to_editor(editor);
    }

    /// Fully disconnects and frees resources.
    pub fn detach(&mut self) {
        self.detach_fully();
    }
}

/// Extracts the word at the given column position.
pub(crate) fn extract_word_at_col(line: &str, col: usize) -> Option<String> {
    if col >= line.len() {
        return None;
    }

    let chars: Vec<char> = line.chars().collect();
    if !chars
        .get(col)
        .is_some_and(|c| c.is_alphanumeric() || *c == '_')
    {
        return None;
    }

    let mut start = col;
    while start > 0
        && chars
            .get(start - 1)
            .is_some_and(|c| c.is_alphanumeric() || *c == '_')
    {
        start -= 1;
    }

    let mut end = col;
    while end < chars.len()
        && chars
            .get(end)
            .is_some_and(|c| c.is_alphanumeric() || *c == '_')
    {
        end += 1;
    }

    Some(chars[start..end].iter().collect())
}
