//! Utility functions for `VimController`.
//!
//! Cursor extraction logic lives in `vim_adapter::controller::cursor` to keep
//! all `vim_core` type usage inside the adapter boundary.

use crate::bridge::vim_adapter::controller::LifecycleTrait;
use crate::bridge::vim_wrapper::VimController;

use godot::classes::{CodeEdit, EditorInterface, EditorSettings};
use godot::prelude::*;
use vim_core::state::store::PersistError;

const INTERNAL_STATE_KEY: &str = "plugins/GodotVim/internal/state_json";

fn with_editor_settings<R>(f: impl FnOnce(Gd<EditorSettings>) -> R) -> Option<R> {
    if !godot::classes::Engine::singleton().is_editor_hint() {
        return None;
    }
    let settings = EditorInterface::singleton().get_editor_settings()?;
    Some(f(settings))
}

impl VimController {
    /// Attach to a CodeEdit editor instance.
    pub fn attach(&mut self, editor: Gd<CodeEdit>) {
        self.attach_to_editor(editor);
    }

    /// Fully disconnects and frees resources.
    pub fn detach(&mut self) {
        self.persist_runtime_state();
        self.detach_fully();
    }

    /// Restore persisted runtime state if available.
    pub(crate) fn restore_runtime_state(&mut self) {
        let Some(raw) = with_editor_settings(|settings| {
            let key: GString = INTERNAL_STATE_KEY.into();
            if !settings.has_setting(&key) {
                return None;
            }
            settings.get_setting(&key).try_to::<String>().ok()
        })
        .flatten() else {
            return;
        };

        if raw.trim().is_empty() {
            return;
        }

        match self.engine.import_persisted_state_json(&raw) {
            Ok(()) => {
                log::info!("Restored persisted Vim runtime state");
            }
            Err(PersistError::SchemaMismatch { expected, found }) => {
                log::warn!(
                    "Resetting persisted Vim state due to schema mismatch (expected {}, found {})",
                    expected,
                    found
                );
                self.engine.reset_runtime_state();
                self.persist_runtime_state();
            }
            Err(err) => {
                log::warn!("Resetting persisted Vim state due to decode failure: {}", err);
                self.engine.reset_runtime_state();
                self.persist_runtime_state();
            }
        }
    }

    /// Persist runtime state snapshot to editor settings.
    pub(crate) fn persist_runtime_state(&mut self) {
        let encoded = match self.engine.export_persisted_state_json() {
            Ok(value) => value,
            Err(err) => {
                log::warn!("Skipping Vim state persistence due to encode failure: {}", err);
                return;
            }
        };

        let _ = with_editor_settings(|mut settings| {
            let key: GString = INTERNAL_STATE_KEY.into();
            settings.set_setting(&key, &encoded.to_variant());
        });
    }

    #[must_use]
    pub(crate) fn is_attached_to_editor(&self, editor_id: godot::obj::InstanceId) -> bool {
        self.attach_session.is_attached_to(editor_id)
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
