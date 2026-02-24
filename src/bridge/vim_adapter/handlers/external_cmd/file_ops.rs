//! File operations: Save, Quit, WriteRange, BufferReopen.

use crate::bridge::godot::script::ScriptContext;
use crate::bridge::vim_wrapper::VimController;

impl VimController {
    pub(super) fn handle_save(&mut self) {
        match ScriptContext::current() {
            Some(ctx) if ctx.save() => {
                // save() syncs CodeEdit text → Script.source_code, then saves via ResourceSaver
                self.show_cmdline_message("Saved");
            }
            Some(_) => self.show_cmdline_message("Save failed"),
            None => self.show_cmdline_message("No script to save (or file not saved to disk yet)"),
        }
    }

    pub(super) fn handle_quit(&mut self) {
        // Closes the current tab by freeing the ScriptEditorBase node.
        // Shows confirmation dialog if file has unsaved changes.
        ScriptContext::close_current_tab();
    }

    pub(super) fn handle_save_quit(&mut self) {
        match ScriptContext::current() {
            Some(ctx) if ctx.save() => {
                // Save first, then close
                ScriptContext::close_current_tab();
            }
            Some(_) => self.show_cmdline_message("Save failed, not quitting"),
            None => self.show_cmdline_message("No script to save"),
        }
    }

    pub(super) fn handle_quit_no_save(&mut self) {
        // :q! closes without saving and without showing a confirmation dialog.
        // Tag the version as saved first so is_unsaved() returns false, then close the tab.
        if let Some(mut editor) = self.get_editor() {
            // tag_saved_version() makes CodeEdit think the current content is the saved version
            editor.tag_saved_version();
        }
        ScriptContext::close_current_tab();
    }

    pub(super) fn handle_save_all(&mut self) {
        let (saved, failed) = ScriptContext::save_all();
        if failed > 0 {
            self.show_cmdline_message(&format!("Saved {saved} files ({failed} failed)"));
        } else if saved > 0 {
            self.show_cmdline_message(&format!("Saved {saved} files"));
        } else {
            self.show_cmdline_message("Nothing to save");
        }
    }

    pub(super) fn handle_buffer_reopen(&mut self) {
        match ScriptContext::current() {
            Some(mut ctx) => {
                ctx.reload();
                self.show_cmdline_message("Reloaded");
            }
            None => self.show_cmdline_message("No script open"),
        }
    }
}

/// Handle source command - execute Vimscript file.
impl VimController {
    pub(super) fn handle_source(&mut self, path: String) {
        match std::fs::read_to_string(&path) {
            Ok(content) => self.handle_execute_script(content),
            Err(e) => log::error!("Failed to source file path={} error={}", path, e),
        }
    }

    pub(super) fn handle_execute_script(&mut self, content: String) {
        for line in content.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('"') {
                continue;
            }
            self.execute_ex_command_with_visuals(line, vim_core::state::mode::Mode::Normal);
        }
    }
}
