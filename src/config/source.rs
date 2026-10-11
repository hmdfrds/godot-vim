//! Sourcing a vimrc on request (`:source`, a Mapping Dialog save): which
//! file a `:source` names and how far it is trusted, what sourcing did, and
//! what the status bar says about it.

use std::fmt::Write;

use compact_str::{format_compact, CompactString};

use super::path::ResolvedConfig;
use crate::types::StatusMessage;

/// What the plugin knows when a `:source` arrives.
pub(crate) struct SourceEnv<'a> {
    pub(crate) active: &'a ResolvedConfig,
    /// The attached editor's script, when it is the script editor's current one.
    pub(crate) current_script: Option<&'a str>,
    pub(crate) home: Option<&'a str>,
    /// The working directory, which a relative path is opened from.
    pub(crate) cwd: Option<&'a str>,
    /// `ProjectSettings::globalize_path`: `res://` and `user://` to OS paths.
    pub(crate) globalize: &'a dyn Fn(&str) -> String,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum SourceTarget {
    /// The file loaded at startup, re-read in full as a restart would.
    ActiveConfig,
    /// Any other file, sourced on top of the current state as in Vim.
    File {
        path: String,
        is_project_level: bool,
    },
}

/// `raw` is the file as typed, `None` for a bare `:source`.
pub(crate) fn resolve(raw: Option<&str>, env: &SourceEnv<'_>) -> Result<SourceTarget, String> {
    let raw = raw.map_or("", str::trim);
    if raw.is_empty() || raw == "$MYVIMRC" {
        return Ok(SourceTarget::ActiveConfig);
    }
    let path = expand(raw, env)?;
    if path == env.active.path || absolute(&path, env) == absolute(&env.active.path, env) {
        return Ok(SourceTarget::ActiveConfig);
    }
    Ok(SourceTarget::File {
        is_project_level: is_in_project(&path, env),
        path,
    })
}

/// `%` and `~`; `$MYVIMRC` is the active config itself.
fn expand(raw: &str, env: &SourceEnv<'_>) -> Result<String, String> {
    if raw == "%" {
        return env.current_script.map(str::to_owned).ok_or_else(|| {
            "E499: Empty file name for '%' or '#', only works with \":p:h\"".to_owned()
        });
    }
    if let Some(rest) = raw.strip_prefix('~') {
        if rest.is_empty() || rest.starts_with(['/', '\\']) {
            let home = env
                .home
                .ok_or_else(|| format!("E484: Can't open file {raw}"))?;
            return Ok(format!("{}{rest}", home.trim_end_matches(['/', '\\'])));
        }
    }
    Ok(raw.to_owned())
}

/// The file the project is responsible for: under `res://` (or a `uid://`
/// into it), or inside the project folder however the path spells it.
fn is_in_project(path: &str, env: &SourceEnv<'_>) -> bool {
    if path.starts_with("res://") || path.starts_with("uid://") {
        return true;
    }
    // Without case: Windows and macOS file systems ignore it, and the error
    // can only fall on the side of more sandboxing.
    let root = absolute("res://", env).to_ascii_lowercase();
    let root = root.trim_end_matches('/');
    let path = absolute(path, env).to_ascii_lowercase();
    !root.is_empty() && (path == root || path.starts_with(&format!("{root}/")))
}

/// `path` as an absolute OS path with `/` separators and no `.` or `..`
/// segments, for comparing.
fn absolute(path: &str, env: &SourceEnv<'_>) -> String {
    let path = (env.globalize)(path).replace('\\', "/");
    if is_absolute(&path) {
        return normalize(&path);
    }
    match env.cwd {
        Some(cwd) => normalize(&format!("{}/{path}", cwd.replace('\\', "/"))),
        None => normalize(&path),
    }
}

fn is_absolute(path: &str) -> bool {
    let bytes = path.as_bytes();
    path.starts_with('/')
        || (bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':')
}

fn normalize(path: &str) -> String {
    let root_len = if path.starts_with('/') {
        1
    } else if is_absolute(path) {
        // `C:` and, when present, its `/`.
        if path[2..].starts_with('/') {
            3
        } else {
            2
        }
    } else {
        0
    };
    let (root, rest) = path.split_at(root_len);
    let mut parts: Vec<&str> = Vec::new();
    for part in rest.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            _ => parts.push(part),
        }
    }
    format!("{root}{}", parts.join("/"))
}

/// What sourcing a vimrc did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Loaded {
    Missing,
    /// A project file under Project Vimrc `Disabled`: nothing was sourced.
    Disabled,
    Sourced {
        /// Lines disabled by the Sandbox policy.
        stripped: usize,
        /// `panelmap` lines in a file other than the active config, which
        /// alone builds the panel binding table.
        ignored_panel_lines: usize,
    },
}

/// The status line after loading `path`. `None` keeps the engine's own
/// "Config file sourced".
pub(crate) fn status(path: &str, loaded: Loaded) -> StatusMessage {
    let (stripped, ignored) = match loaded {
        Loaded::Missing => {
            return StatusMessage::Error(format_compact!("E484: Can't open file {path}"));
        }
        Loaded::Disabled => {
            return StatusMessage::Error(format_compact!(
                "{path} not sourced: Security > Project Vimrc is Disabled"
            ));
        }
        Loaded::Sourced {
            stripped,
            ignored_panel_lines,
        } => (stripped, ignored_panel_lines),
    };
    let mut text = CompactString::from("Config file sourced");
    if stripped > 0 {
        let _ = write!(text, ", {stripped} line(s) skipped by the Sandbox policy");
    }
    if ignored > 0 {
        let _ = write!(
            text,
            ", {ignored} panelmap line(s) ignored: panel keys come from the active config only"
        );
        StatusMessage::Error(text)
    } else if stripped > 0 {
        StatusMessage::Info(text)
    } else {
        StatusMessage::None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn globalize(p: &str) -> String {
        if let Some(rest) = p.strip_prefix("res://") {
            format!("/proj/{rest}")
        } else if let Some(rest) = p.strip_prefix("user://") {
            format!("/data/proj/{rest}")
        } else {
            p.to_owned()
        }
    }

    fn env<'a>(active: &'a ResolvedConfig, current: Option<&'a str>) -> SourceEnv<'a> {
        SourceEnv {
            active,
            current_script: current,
            home: Some("/home/me"),
            cwd: Some("/proj"),
            globalize: &globalize,
        }
    }

    fn user_config() -> ResolvedConfig {
        ResolvedConfig {
            path: "user://.godot-vimrc".into(),
            is_project_level: false,
        }
    }

    fn file(path: &str, is_project_level: bool) -> Result<SourceTarget, String> {
        Ok(SourceTarget::File {
            path: path.into(),
            is_project_level,
        })
    }

    #[test]
    fn bare_and_myvimrc_reload_the_active_config() {
        let active = user_config();
        for raw in [None, Some(""), Some("  "), Some("$MYVIMRC")] {
            assert_eq!(
                resolve(raw, &env(&active, None)),
                Ok(SourceTarget::ActiveConfig),
                "{raw:?}"
            );
        }
    }

    #[test]
    fn the_active_config_by_any_name_is_a_reload() {
        let active = user_config();
        for raw in [
            "user://.godot-vimrc",
            "/data/proj/.godot-vimrc",
            "/data/proj/./sub/../.godot-vimrc",
            "../data/proj/.godot-vimrc",
        ] {
            assert_eq!(
                resolve(Some(raw), &env(&active, None)),
                Ok(SourceTarget::ActiveConfig),
                "{raw}"
            );
        }
    }

    /// A project config stays the active config, with its own provenance,
    /// however `:source` names it.
    #[test]
    fn a_project_active_config_is_reloaded_not_added() {
        let active = ResolvedConfig {
            path: "res://.godot-vimrc".into(),
            is_project_level: true,
        };
        for raw in [
            "$MYVIMRC",
            "res://.godot-vimrc",
            "/proj/.godot-vimrc",
            ".godot-vimrc",
        ] {
            assert_eq!(
                resolve(Some(raw), &env(&active, None)),
                Ok(SourceTarget::ActiveConfig),
                "{raw}"
            );
        }
    }

    #[test]
    fn percent_is_the_current_script() {
        let active = user_config();
        assert_eq!(
            resolve(Some("%"), &env(&active, Some("res://tools/extra.gd"))),
            file("res://tools/extra.gd", true)
        );
        assert!(resolve(Some("%"), &env(&active, None))
            .unwrap_err()
            .starts_with("E499"));
    }

    #[test]
    fn tilde_is_home() {
        let active = user_config();
        assert_eq!(
            resolve(Some("~/.vim/godot.vim"), &env(&active, None)),
            file("/home/me/.vim/godot.vim", false)
        );
        assert_eq!(
            resolve(Some("~"), &env(&active, None)),
            file("/home/me", false)
        );
        // Not expanded, so a path relative to the working directory.
        assert_eq!(
            resolve(Some("~user/x"), &env(&active, None)),
            file("~user/x", true)
        );
        let no_home = SourceEnv {
            home: None,
            ..env(&active, None)
        };
        assert_eq!(
            resolve(Some("~/x.vim"), &no_home),
            Err("E484: Can't open file ~/x.vim".into())
        );
    }

    #[test]
    fn project_files_are_project_level_however_named() {
        let active = user_config();
        for raw in [
            "res://extra.vim",
            "uid://b2x4kd7",
            "/proj/sub/extra.vim",
            "/proj/../proj/extra.vim",
            "sub/extra.vim",
            "./extra.vim",
            "/PROJ/extra.vim",
        ] {
            assert_eq!(
                resolve(Some(raw), &env(&active, None)),
                file(raw, true),
                "{raw}"
            );
        }
    }

    #[test]
    fn files_outside_the_project_are_the_users() {
        let active = user_config();
        for raw in [
            "/project2/extra.vim",
            "/proj/../elsewhere/extra.vim",
            "../tools/extra.vim",
            "user://extra.vim",
        ] {
            assert_eq!(
                resolve(Some(raw), &env(&active, None)),
                file(raw, false),
                "{raw}"
            );
        }
    }

    #[test]
    fn windows_paths_compare_by_separator_and_case() {
        let globalize = |p: &str| match p.strip_prefix("res://") {
            Some(rest) => format!("C:/Games/Proj/{rest}"),
            None => p.to_owned(),
        };
        let active = user_config();
        let env = SourceEnv {
            active: &active,
            current_script: None,
            home: Some("C:\\Users\\me"),
            cwd: Some("C:\\Games\\Proj"),
            globalize: &globalize,
        };
        for (raw, is_project_level) in [
            ("c:\\games\\proj\\extra.vim", true),
            ("extra.vim", true),
            ("C:/Games/Proj2/extra.vim", false),
            ("~\\extra.vim", false),
        ] {
            let Ok(SourceTarget::File {
                is_project_level: got,
                ..
            }) = resolve(Some(raw), &env)
            else {
                panic!("{raw}");
            };
            assert_eq!(got, is_project_level, "{raw}");
        }
    }

    #[test]
    fn a_clean_load_leaves_the_engines_message() {
        let clean = Loaded::Sourced {
            stripped: 0,
            ignored_panel_lines: 0,
        };
        assert_eq!(status("user://.godot-vimrc", clean), StatusMessage::None);
    }

    #[test]
    fn every_other_outcome_is_named() {
        let path = "res://.godot-vimrc";
        let sourced = |stripped, ignored_panel_lines| Loaded::Sourced {
            stripped,
            ignored_panel_lines,
        };
        assert_eq!(
            status(path, Loaded::Missing),
            StatusMessage::Error("E484: Can't open file res://.godot-vimrc".into())
        );
        assert_eq!(
            status(path, Loaded::Disabled),
            StatusMessage::Error(
                "res://.godot-vimrc not sourced: Security > Project Vimrc is Disabled".into()
            )
        );
        assert_eq!(
            status(path, sourced(2, 0)),
            StatusMessage::Info(
                "Config file sourced, 2 line(s) skipped by the Sandbox policy".into()
            )
        );
        assert_eq!(
            status(path, sourced(0, 1)),
            StatusMessage::Error(
                "Config file sourced, 1 panelmap line(s) ignored: \
                 panel keys come from the active config only"
                    .into()
            )
        );
        assert_eq!(
            status(path, sourced(2, 1)),
            StatusMessage::Error(
                "Config file sourced, 2 line(s) skipped by the Sandbox policy, \
                 1 panelmap line(s) ignored: panel keys come from the active config only"
                    .into()
            )
        );
    }
}
