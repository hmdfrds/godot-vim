//! Loading a vimrc: the active config from a clean slate, or another file
//! on top, for every path that loads one.

use godot::classes::{Os, ProjectSettings};
use godot::prelude::*;
use vim_core::execution::ConfigText;

use super::GodotVimCore;
use crate::config::path::ResolvedConfig;
use crate::config::policy::{LoadPolicy, SourceAccess, Tier};
use crate::config::source::{self, Loaded, SourceEnv, SourceTarget};
use crate::controller::LoadReport;
use crate::settings::FileAccessScope;
use crate::types::StatusMessage;

/// What [`GodotVimCore::load_vimrc`] loads.
pub(super) enum LoadTarget {
    /// The active config, with what a config defines reset first.
    Active,
    /// Another file, on top of the current state as in Vim.
    File {
        path: String,
        is_project_level: bool,
    },
}

/// What the plugin knows about the editor for resolving a file name.
struct EnvParts {
    current_script: Option<String>,
    home: Option<String>,
    cwd: Option<String>,
}

fn globalize(path: &str) -> String {
    ProjectSettings::singleton()
        .globalize_path(path)
        .to_string()
}

impl GodotVimCore {
    /// `raw` is the file as typed, `None` for a bare `:source`.
    pub(super) fn source_command(&mut self, raw: Option<&str>) {
        let active = self.resolve_config_path();
        let status = match self.source_target(raw, &active) {
            Err(error) => StatusMessage::Error(error.into()),
            Ok(SourceTarget::ActiveConfig) => {
                let loaded = self.load_vimrc(LoadTarget::Active, "source");
                source::status(&active.path, loaded)
            }
            Ok(SourceTarget::File {
                path,
                is_project_level,
            }) => match crate::host::file::validate_path_scope(&path, self.file_access_scope()) {
                Err(error) => StatusMessage::Error(error.to_string().into()),
                Ok(()) => {
                    let target = LoadTarget::File {
                        path: path.clone(),
                        is_project_level,
                    };
                    source::status(&path, self.load_vimrc(target, "source"))
                }
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
        let parts = self.env_parts();
        source::resolve(raw, &env(active, &parts))
    }

    fn env_parts(&self) -> EnvParts {
        EnvParts {
            current_script: self
                .attached_editor
                .as_ref()
                .and_then(crate::host::editor_host::attached_script_path),
            home: home_dir(),
            cwd: std::env::current_dir()
                .ok()
                .map(|dir| dir.to_string_lossy().into_owned()),
        }
    }

    pub(super) fn file_access_scope(&self) -> FileAccessScope {
        self.settings
            .as_ref()
            .map_or(FileAccessScope::ProjectOnly, |s| s.file_access_scope)
    }

    /// Load a vimrc: startup, the enable edge, panic recovery, `:source`
    /// and a Mapping Dialog save all come here. The file is read once and
    /// each command is judged by the [`LoadPolicy`] of its tier. Errors and
    /// refusals go to Output, where they are seen with Log Level Off.
    ///
    /// The active config is loaded from a clean slate, and the panel table
    /// is built from its panel lines; with no file, or under Project Vimrc
    /// `Disabled`, the reset and the seeded defaults still run. Another
    /// file is only sourced on top.
    pub(super) fn load_vimrc(&mut self, target: LoadTarget, caller: &str) -> Loaded {
        let active = self.resolve_config_path();
        let project_vimrc = self.project_vimrc();
        let (path, is_project_level, reload) = match target {
            LoadTarget::Active => (active.path.clone(), active.is_project_level, true),
            LoadTarget::File {
                path,
                is_project_level,
            } => (path, is_project_level, false),
        };
        let tier = Tier::of_file(is_project_level, project_vimrc);
        let text = tier.and_then(|_| crate::config::writer::read_file(&path));
        if tier.is_none() {
            log::info!("{caller}: '{path}' skipped: Project Vimrc is Disabled");
        }
        if !reload && text.is_none() {
            return if tier.is_none() {
                Loaded::Disabled
            } else {
                Loaded::Missing
            };
        }

        let parts = self.env_parts();
        let read = |path: &str| crate::config::writer::read_file(path);
        let files = SourceAccess {
            env: env(&active, &parts),
            scope: self.file_access_scope(),
            read: &read,
        };
        let mut policy = LoadPolicy::new(project_vimrc)
            .with_actions(&self.actions)
            .with_files(&files);
        if reload {
            policy = policy.collecting_panels();
        }
        let config = text
            .as_deref()
            .zip(tier)
            .map(|(text, tier)| ConfigText::new(&path, text).with_trust(tier.tag()));
        let report = match &mut self.controller {
            None => LoadReport::default(),
            Some(controller) if reload => controller.reload_config(config, &mut policy),
            Some(controller) => config.map_or_else(LoadReport::default, |config| {
                controller.load_config(config, &mut policy)
            }),
        };
        for line in &report.lines {
            godot_print!("{line}");
        }
        let panel_lines = policy.panel_lines().to_vec();
        let ignored_panel_lines = policy.ignored_panel_lines();
        drop(policy);
        log::info!("{caller}: loaded config from '{path}'");

        self.refresh_filetype_setup();
        self.rebuild_langmap();
        if reload {
            self.rebuild_bindings(&panel_lines, &active.path);
        }
        match (tier, text) {
            (None, _) => Loaded::Disabled,
            (Some(_), None) => Loaded::Missing,
            (Some(_), Some(_)) => Loaded::Sourced {
                errors: report.errors,
                refused: report.refused,
                ignored_panel_lines,
            },
        }
    }
}

fn env<'a>(active: &'a ResolvedConfig, parts: &'a EnvParts) -> SourceEnv<'a> {
    SourceEnv {
        active,
        current_script: parts.current_script.as_deref(),
        home: parts.home.as_deref(),
        cwd: parts.cwd.as_deref(),
        globalize: &globalize,
    }
}

fn home_dir() -> Option<String> {
    let os = Os::singleton();
    ["HOME", "USERPROFILE"]
        .into_iter()
        .map(|key| os.get_environment(key).to_string())
        .find(|value| !value.is_empty())
}
