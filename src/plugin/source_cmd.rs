//! `:source`: re-read the active config, or source another file on top.

use godot::classes::{Os, ProjectSettings};
use godot::prelude::*;

use super::GodotVimCore;
use crate::config::path::ResolvedConfig;
use crate::config::source::{self, Loaded, SourceEnv, SourceTarget};
use crate::settings::FileAccessScope;
use crate::types::StatusMessage;

impl GodotVimCore {
    /// `raw` is the file as typed, `None` for a bare `:source`.
    pub(super) fn source_command(&mut self, raw: Option<&str>) {
        let active = self.resolve_config_path();
        let status = match self.source_target(raw, &active) {
            Err(error) => StatusMessage::Error(error.into()),
            Ok(SourceTarget::ActiveConfig) => {
                let loaded = self.source_config_from_disk("source");
                source::status(&active.path, loaded)
            }
            Ok(SourceTarget::File {
                path,
                is_project_level,
            }) => match self.source_other_file(&path, is_project_level) {
                Ok(loaded) => source::status(&path, loaded),
                Err(error) => StatusMessage::Error(error.into()),
            },
        };
        if let Some(controller) = &mut self.controller {
            controller.show_status(status);
        }
    }

    fn source_target(
        &self,
        raw: Option<&str>,
        active: &ResolvedConfig,
    ) -> Result<SourceTarget, String> {
        let current_script = self
            .attached_editor
            .as_ref()
            .and_then(crate::host::editor_host::attached_script_path);
        let home = home_dir();
        let cwd = std::env::current_dir()
            .ok()
            .map(|dir| dir.to_string_lossy().into_owned());
        let globalize = |path: &str| {
            ProjectSettings::singleton()
                .globalize_path(path)
                .to_string()
        };
        source::resolve(
            raw,
            &SourceEnv {
                active,
                current_script: current_script.as_deref(),
                home: home.as_deref(),
                cwd: cwd.as_deref(),
                globalize: &globalize,
            },
        )
    }

    /// Source a file other than the active config on top of the current
    /// state, under the File Access Scope of `:e` and the Project Vimrc
    /// policy of startup.
    fn source_other_file(&mut self, path: &str, is_project_level: bool) -> Result<Loaded, String> {
        let scope = self
            .settings
            .as_ref()
            .map_or(FileAccessScope::ProjectOnly, |s| s.file_access_scope);
        crate::host::file::validate_path_scope(path, scope).map_err(|e| e.to_string())?;
        let Some(text) = crate::config::writer::read_file(path) else {
            return Ok(Loaded::Missing);
        };
        let Some(policed) = crate::config::sandbox::apply_vimrc_policy(
            &text,
            is_project_level,
            self.project_vimrc(),
        ) else {
            return Ok(Loaded::Disabled);
        };
        let ignored_panel_lines = policed
            .text
            .lines()
            .filter(|line| crate::config::panelmap::is_panel_line(line.trim()))
            .count();
        if let Some(controller) = &mut self.controller {
            controller.source_config(&policed.text);
        }
        self.refresh_filetype_setup();
        self.rebuild_langmap();
        Ok(Loaded::Sourced {
            stripped: policed.stripped,
            ignored_panel_lines,
        })
    }
}

fn home_dir() -> Option<String> {
    let os = Os::singleton();
    ["HOME", "USERPROFILE"]
        .into_iter()
        .map(|key| os.get_environment(key).to_string())
        .find(|value| !value.is_empty())
}
