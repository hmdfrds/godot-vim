//! Filetype layer: Vim's filetype detection and ftplugins, for Godot.
//!
//! Vim gives each language its own formatting through a filetype plugin
//! that runs `:setlocal` lines when a buffer gets its filetype. godot-vim
//! does the same on attach:
//!
//! - [`detect`] finds the filetype from what Godot exposes about the
//!   CodeEdit (script class, syntax highlighter, comment delimiters);
//! - [`runtime`] holds the transcribed `:setlocal` lines per filetype and
//!   applies them to the buffer's own options once, undoing them if the
//!   buffer's filetype changes.
//!
//! The switches mirror Vim's `:filetype` command. `filetype plugin off` in
//! the vimrc and the `filetype_plugin` Editor Setting both turn the plugins
//! off; whichever changed last wins, like every other pushed setting.

pub(crate) mod detect;
pub(crate) mod runtime;

/// Vim's `:filetype` state: whether filetypes are detected, and whether
/// their plugins run. Plugins run only when both are on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Switches {
    pub(crate) detection: bool,
    pub(crate) plugin: bool,
}

impl Default for Switches {
    fn default() -> Self {
        Self {
            detection: true,
            plugin: crate::settings::defaults::FILETYPE_PLUGIN,
        }
    }
}

impl Switches {
    /// Whether filetype plugins run.
    pub(crate) const fn plugins_run(self) -> bool {
        self.detection && self.plugin
    }

    pub(crate) fn apply(&mut self, cmd: FiletypeCommand) {
        if let Some(d) = cmd.detection {
            self.detection = d;
        }
        if let Some(p) = cmd.plugin {
            self.plugin = p;
        }
    }
}

/// The effect of one `:filetype` line on the [`Switches`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct FiletypeCommand {
    pub(crate) detection: Option<bool>,
    pub(crate) plugin: Option<bool>,
}

/// Parse a `:filet[ype] [plugin] [indent] on|off|detect` line, as Vim's
/// `:help :filetype-overview` describes it:
///
/// - `filetype on` turns detection on; `filetype off` turns it off.
/// - `filetype plugin on` turns detection and plugins on, and so does
///   `filetype plugin indent on`. `filetype plugin off` turns plugins off
///   and leaves detection alone.
/// - `indent` is accepted and has no effect of its own: Godot owns indent.
///   `filetype indent on` still turns detection on, as in Vim.
/// - `filetype detect` changes nothing.
///
/// Leading colons, a trailing `"` comment and a `|` with a command after
/// it are accepted, as Vim's `:filetype` takes them (see
/// [`split_filetype_line`]). Returns `None` for anything else, including a
/// bare `:filetype`, which only prints the state.
pub(crate) fn parse_filetype_command(line: &str) -> Option<FiletypeCommand> {
    split_filetype_line(line).map(|(cmd, _)| cmd)
}

/// [`parse_filetype_command`], plus the text after a `|` that ends the
/// command, if any. The host runs only the `:filetype` part; the sandbox
/// uses the rest to judge the whole line.
///
/// `:filetype` is `EX_TRLBAR` in Vim: its arguments end at the first `"`,
/// which starts a comment that runs to the end of the line, or at the first
/// `|`, which starts the next command. Its arguments are plain words, so no
/// escape is needed before either.
pub(crate) fn split_filetype_line(line: &str) -> Option<(FiletypeCommand, Option<&str>)> {
    let line = line.trim_start().trim_start_matches(':');
    let (args, rest) = match line.find(['"', '|']) {
        Some(at) if line[at..].starts_with('|') => (&line[..at], Some(&line[at + 1..])),
        Some(at) => (&line[..at], None),
        None => (line, None),
    };
    let mut words = args.split_whitespace();
    let cmd = words.next()?;
    if !is_filetype_abbrev(cmd) {
        return None;
    }
    let mut plugin = false;
    let mut indent = false;
    let mut state = None;
    for word in words {
        match word {
            "plugin" if state.is_none() => plugin = true,
            "indent" if state.is_none() => indent = true,
            "on" | "off" | "detect" if state.is_none() => state = Some(word),
            _ => return None,
        }
    }
    let out = match state? {
        "on" => FiletypeCommand {
            detection: Some(true),
            plugin: plugin.then_some(true),
        },
        "off" if plugin => FiletypeCommand {
            detection: None,
            plugin: Some(false),
        },
        "off" if indent => FiletypeCommand::default(),
        "off" => FiletypeCommand {
            detection: Some(false),
            plugin: None,
        },
        _ => FiletypeCommand::default(),
    };
    Some((out, rest))
}

/// `filet[ype]`, case-sensitive like Vim's Ex command names.
pub(crate) fn is_filetype_abbrev(word: &str) -> bool {
    word.len() >= "filet".len() && "filetype".starts_with(word)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(lines: &[&str]) -> Switches {
        let mut s = Switches {
            detection: true,
            plugin: true,
        };
        for line in lines {
            if let Some(cmd) = parse_filetype_command(line) {
                s.apply(cmd);
            }
        }
        s
    }

    #[test]
    fn plugin_off_and_on() {
        assert!(!run(&["filetype plugin off"]).plugins_run());
        assert!(run(&["filetype plugin off", "filetype plugin on"]).plugins_run());
        assert!(run(&["filetype plugin off", "filetype plugin indent on"]).plugins_run());
    }

    #[test]
    fn detection_off_stops_plugins_and_on_restores_them() {
        let off = run(&["filetype off"]);
        assert!(!off.plugins_run());
        assert!(off.plugin, "filetype off leaves the plugin switch alone");
        assert!(run(&["filetype off", "filetype on"]).plugins_run());
        assert!(run(&["filetype off", "filetype plugin on"]).plugins_run());
    }

    #[test]
    fn indent_and_detect_do_not_touch_plugins() {
        assert!(run(&["filetype indent off"]).plugins_run());
        assert!(run(&["filetype detect"]).plugins_run());
        assert!(!run(&["filetype plugin off", "filetype indent on"]).plugins_run());
    }

    #[test]
    fn abbreviations_and_rejects() {
        assert!(parse_filetype_command("filet plugin off").is_some());
        assert!(parse_filetype_command("filetyp on").is_some());
        assert!(parse_filetype_command("file on").is_none());
        assert!(parse_filetype_command("filetype").is_none());
        assert!(parse_filetype_command("filetype plugin").is_none());
        assert!(parse_filetype_command("filetype on off").is_none());
        assert!(parse_filetype_command("filetype bogus on").is_none());
        assert!(parse_filetype_command("filetypes on").is_none());
    }

    /// `:filetype` is EX_TRLBAR in Vim: a `"` starts a comment and a `|`
    /// ends the command. A leading colon is accepted.
    #[test]
    fn trailing_comment_bar_and_colon() {
        let off = Some(FiletypeCommand {
            detection: None,
            plugin: Some(false),
        });
        assert_eq!(
            parse_filetype_command("filetype plugin off \" no plugins"),
            off
        );
        assert_eq!(parse_filetype_command("filetype plugin off\"x"), off);
        assert_eq!(
            parse_filetype_command("filetype plugin off | set tw=10"),
            off
        );
        assert_eq!(parse_filetype_command("filetype plugin off|syntax on"), off);
        assert_eq!(parse_filetype_command(":filetype plugin off"), off);
        assert_eq!(parse_filetype_command("::filet plugin off"), off);
        assert!(run(&[
            "filetype plugin off",
            "filetype plugin indent on | syntax on"
        ])
        .plugins_run());
        assert!(parse_filetype_command("filetype \" plugin off").is_none());
        assert!(parse_filetype_command("filetype | plugin off").is_none());
        assert_eq!(
            split_filetype_line("filetype plugin off | set tw=10").map(|(_, rest)| rest),
            Some(Some(" set tw=10"))
        );
        assert_eq!(
            split_filetype_line("filetype plugin off \" x | !rm").map(|(_, rest)| rest),
            Some(None),
            "a bar inside the comment is part of the comment"
        );
    }
}
