//! What a config may do, judged on each command vim-core parsed from it.
//!
//! The tier comes from where the file is: the seeded defaults are
//! [`Tier::Builtin`], `user://` and the Config File Path are
//! [`Tier::Trusted`], and a file inside the project follows Project Vimrc.
//! The Sandbox judge is an allowlist: a command kind it does not name is
//! refused.

use vim_core::execution::{
    ConfigAction, ConfigCommand, ConfigHost, MapDefinition, Opaque, ReachedCommand, SourceReply,
    SourceRequest, TrustTag, Verdict,
};
use vim_core::grammar::types::{ExCommand, SetAssignment};
use vim_core::grammar::Reach;
use vim_core::keymap::MappingKind;

use super::cmdline_judge::SandboxLineJudge;
use super::source::{SourceEnv, SourceTarget};
use crate::actions::action::ActionRegistry;
use crate::ftplugin::FiletypeCommand;
use crate::settings::{FileAccessScope, ProjectVimrc};

/// How far a config is trusted. The numbers are its [`TrustTag`]s, the
/// most trusted lowest, as vim-core keeps the highest of several.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Tier {
    Builtin = 0,
    Trusted = 1,
    Sandbox = 2,
}

impl Tier {
    pub(crate) const fn tag(self) -> TrustTag {
        TrustTag::new(self as u32)
    }

    /// An unknown tag is judged as the Sandbox.
    pub(super) const fn of(tag: TrustTag) -> Self {
        match tag.get() {
            0 => Self::Builtin,
            1 => Self::Trusted,
            _ => Self::Sandbox,
        }
    }

    /// The tier of a file by where it is; `None` for a project file under
    /// Project Vimrc `Disabled`, which is not read.
    pub(crate) const fn of_file(
        is_project_level: bool,
        project_vimrc: ProjectVimrc,
    ) -> Option<Self> {
        if !is_project_level {
            return Some(Self::Trusted);
        }
        match project_vimrc {
            ProjectVimrc::Disabled => None,
            ProjectVimrc::Sandbox => Some(Self::Sandbox),
            ProjectVimrc::Trusted => Some(Self::Trusted),
        }
    }
}

/// The name the seeded defaults are loaded under.
pub(crate) const DEFAULTS_NAME: &str = "godot-vim defaults";

/// What a nested `:source` needs: where files are and how to read one.
pub(crate) struct SourceAccess<'a> {
    pub(crate) env: SourceEnv<'a>,
    pub(crate) scope: FileAccessScope,
    pub(crate) read: &'a dyn Fn(&str) -> Option<String>,
}

/// The [`ConfigHost`] godot-vim loads every config through.
///
/// It claims `:filetype` and the panel lines, refuses every other command
/// that would become a host request, and answers each nested `:source`
/// itself, so a load leaves nothing pending in the engine.
pub(crate) struct LoadPolicy<'a> {
    actions: Option<&'a ActionRegistry>,
    files: Option<&'a SourceAccess<'a>>,
    project_vimrc: ProjectVimrc,
    /// Panel lines of the top file build the panel table.
    collect_panels: bool,
    filetype: Vec<FiletypeCommand>,
    panel_lines: Vec<(u32, String)>,
    ignored_panel_lines: usize,
}

impl<'a> LoadPolicy<'a> {
    /// A policy that knows no action names and refuses every nested
    /// `:source`.
    pub(crate) fn new(project_vimrc: ProjectVimrc) -> Self {
        Self {
            actions: None,
            files: None,
            project_vimrc,
            collect_panels: false,
            filetype: Vec::new(),
            panel_lines: Vec::new(),
            ignored_panel_lines: 0,
        }
    }

    /// `<Action>` and `:action` names a Sandbox mapping may use.
    pub(crate) const fn with_actions(mut self, actions: &'a ActionRegistry) -> Self {
        self.actions = Some(actions);
        self
    }

    pub(crate) const fn with_files(mut self, files: &'a SourceAccess<'a>) -> Self {
        self.files = Some(files);
        self
    }

    /// Keep the panel lines of the top file for the panel table; other
    /// files' are counted as ignored.
    pub(crate) const fn collecting_panels(mut self) -> Self {
        self.collect_panels = true;
        self
    }

    /// The judge of the lines Sandbox mappings make when they run, with
    /// the same action names.
    pub(crate) fn command_line_judge(&self) -> SandboxLineJudge {
        SandboxLineJudge::new(self.actions)
    }

    pub(crate) fn take_filetype(&mut self) -> Vec<FiletypeCommand> {
        std::mem::take(&mut self.filetype)
    }

    /// The panel lines of the top file, with their line numbers.
    pub(crate) fn panel_lines(&self) -> &[(u32, String)] {
        &self.panel_lines
    }

    pub(crate) const fn ignored_panel_lines(&self) -> usize {
        self.ignored_panel_lines
    }

    fn decide(&self, command: &ConfigCommand<'_>, tier: Tier) -> Decision {
        if let ConfigAction::Custom { name, args, .. } = command.action {
            if let Some(decision) = host_line(name, args, tier) {
                return decision;
            }
        }
        match tier {
            Tier::Sandbox => self.sandbox(command),
            Tier::Builtin | Tier::Trusted => trusted(command),
        }
    }

    /// The Sandbox allowlist.
    fn sandbox(&self, command: &ConfigCommand<'_>) -> Decision {
        let refuse = |reason: &str| Decision::Refuse(sandbox_error(command.text, reason));
        match command.action {
            ConfigAction::SetOptions { options, .. } => options
                .iter()
                .find(|option| option.secure || is_blocked_option(&option.name))
                .map_or(Decision::Allow, |option| {
                    refuse(&format!("option '{}'", option.name))
                }),
            ConfigAction::Map(map) => self
                .mapping_refusal(map)
                .map_or(Decision::Allow, |reason| refuse(&reason)),
            ConfigAction::Unmap { .. }
            | ConfigAction::MapClear { .. }
            | ConfigAction::Leader { .. }
            | ConfigAction::Message => Decision::Allow,
            ConfigAction::Abbreviate { .. }
            | ConfigAction::Unabbreviate { .. }
            | ConfigAction::AbClear { .. } => refuse("abbreviation"),
            ConfigAction::Handler => refuse("handler"),
            ConfigAction::Source { .. } => refuse("sources a file"),
            ConfigAction::Host => refuse("editor command"),
            ConfigAction::Custom { .. } if runs_own_command(command) => refuse("godot-vim command"),
            ConfigAction::Custom { .. } => refuse("not a vim-core command"),
            _ => refuse("not on the Sandbox allowlist"),
        }
    }

    /// Why a Sandbox mapping is refused, if it is: anything its keys can
    /// run that the Sandbox would refuse in the file, or that cannot be
    /// followed.
    fn mapping_refusal(&self, map: &MapDefinition<'_>) -> Option<String> {
        if map.flags.expr {
            return Some("<expr>".into());
        }
        if map.kind == MappingKind::Recursive {
            return Some("recursive map".into());
        }
        let reach = map.reach;
        if let Some(opaque) = reach.opaque.first() {
            return Some(opaque_reason(*opaque).into());
        }
        if let Some(name) = reach.actions.iter().find(|name| !self.is_action(name)) {
            return Some(format!("<Action>({name})"));
        }
        if let Some(name) = reach.plugs.first() {
            return Some(format!("<Plug>({name})"));
        }
        reach
            .commands
            .iter()
            .find_map(|command| self.reached_refusal(command))
    }

    /// The rule for a command a Sandbox mapping's keys can run.
    fn reached_refusal(&self, reached: &ReachedCommand) -> Option<String> {
        // One that does not parse only gives its error when it runs.
        let Ok((_, command)) = &reached.parsed else {
            return None;
        };
        reached_refusal(command, reached.reach, &|name| self.is_action(name))
            .map(|what| format!("{what}: {}", reached.text))
    }

    fn is_action(&self, name: &str) -> bool {
        self.actions
            .is_some_and(|actions| actions.id_of(name).is_some())
    }

    fn resolve_source(&self, request: &SourceRequest<'_>) -> Result<SourceReply, String> {
        let Some(files) = self.files else {
            return Err(format!(
                "E484: Can't open file {}",
                request.path.unwrap_or_default()
            ));
        };
        let (path, is_project_level) = match super::source::resolve(request.path, &files.env)? {
            SourceTarget::ActiveConfig => (
                files.env.active.path.clone(),
                files.env.active.is_project_level,
            ),
            SourceTarget::File {
                path,
                is_project_level,
            } => {
                crate::host::file::validate_path_scope(&path, files.scope)
                    .map_err(|error| error.to_string())?;
                (path, is_project_level)
            }
        };
        let tier = Tier::of_file(is_project_level, self.project_vimrc)
            .ok_or_else(|| format!("{path} not sourced: Security > Project Vimrc is Disabled"))?;
        let text = (files.read)(&path).ok_or_else(|| format!("E484: Can't open file {path}"))?;
        Ok(SourceReply::Text {
            name: path,
            text,
            trust: tier.tag(),
        })
    }
}

impl ConfigHost for LoadPolicy<'_> {
    fn judge(&mut self, command: &ConfigCommand<'_>) -> Verdict {
        match self.decide(command, Tier::of(command.trust)) {
            Decision::Allow => Verdict::Allow,
            Decision::Refuse(error) => Verdict::Refuse(error.into()),
            Decision::Ignore => Verdict::Claim,
            Decision::Filetype(filetype) => {
                self.filetype.push(filetype);
                Verdict::Claim
            }
            Decision::Panel(line) => {
                if self.collect_panels && command.depth == 1 {
                    self.panel_lines.push((command.line, line));
                } else {
                    self.ignored_panel_lines += 1;
                }
                Verdict::Claim
            }
        }
    }

    fn has_feature(&self, feature: &str) -> bool {
        feature == "godot"
    }

    fn source(&mut self, request: &SourceRequest<'_>) -> SourceReply {
        self.resolve_source(request)
            .unwrap_or_else(SourceReply::Refuse)
    }
}

/// What the policy does with one command.
#[derive(Debug)]
enum Decision {
    Allow,
    Refuse(String),
    /// Claimed and dropped without a word.
    Ignore,
    Filetype(FiletypeCommand),
    /// A panel line, as the panel parser reads it.
    Panel(String),
}

/// The tiers above the Sandbox run what vim-core runs, but nothing that
/// would become a host request: those are editor actions, not settings.
fn trusted(command: &ConfigCommand<'_>) -> Decision {
    let text = command.text;
    match command.action {
        ConfigAction::Host => Decision::Refuse(format!("E523: Not allowed here: {text}")),
        ConfigAction::Custom { vim_command, .. } => {
            Decision::Refuse(if runs_own_command(command) {
                format!("E523: Not allowed here: {text}")
            } else if vim_command {
                format!("E319: Sorry, the command is not available in this version: {text}")
            } else {
                format!("E492: Not an editor command: {text}")
            })
        }
        _ => Decision::Allow,
    }
}

/// Whether the host would run the command as one of godot-vim's: it gets
/// the text, not vim-core's name for it (`save`, not `saveas`).
fn runs_own_command(command: &ConfigCommand<'_>) -> bool {
    matches!(command.command, ExCommand::Custom { command } if crate::host::runs_own_command(command))
}

/// `:filetype` and the panel lines, which godot-vim runs itself.
fn host_line(name: &str, args: &str, tier: Tier) -> Option<Decision> {
    if name == "filetype" {
        let args = args.trim();
        if args.is_empty() {
            return Some(Decision::Ignore);
        }
        return Some(
            crate::ftplugin::parse_filetype_command(&format!("filetype {args}")).map_or_else(
                || Decision::Refuse(format!("E474: Invalid argument: {args}")),
                Decision::Filetype,
            ),
        );
    }
    if name != super::panelmap::MAP && name != super::panelmap::UNMAP {
        return None;
    }
    // The opt-in line of later versions, so a vimrc shared with them loads
    // here without a word.
    let mut words = args.split_whitespace();
    if name == super::panelmap::MAP && words.next() == Some("preset") && words.nth(1).is_none() {
        return Some(Decision::Ignore);
    }
    let line = format!("{name} {args}");
    if tier == Tier::Sandbox && !panel_line_is_safe(&line) {
        return Some(Decision::Refuse(sandbox_error(&line, "panel target")));
    }
    Some(Decision::Panel(line))
}

/// Vim's sandbox error (ex_docmd.c `do_one_cmd()`, a command without
/// `EX_SBOXOK`), with the reason.
pub(super) fn sandbox_error(text: &str, reason: &str) -> String {
    format!("E48: Not allowed in sandbox: {text} ({reason})")
}

/// Why the Sandbox refuses a command a Sandbox mapping's keys run, that
/// `reach` describes, if it does. `is_action` says which `<Action>` and
/// `:action` names are registered.
pub(crate) fn reached_refusal(
    command: &ExCommand,
    reach: Reach,
    is_action: &dyn Fn(&str) -> bool,
) -> Option<&'static str> {
    let outside = reach.difference(REACHED_ALLOWED);
    if !outside.is_empty() {
        return Some(reach_reason(outside));
    }
    if reach.contains(Reach::FILE_WRITE | Reach::NAMED_PATH) {
        return Some("writes a named file");
    }
    command_refusal(command, is_action, 0)
}

/// What the flags do not show: the names a command and the commands it
/// runs on others use.
fn command_refusal(
    command: &ExCommand,
    is_action: &dyn Fn(&str) -> bool,
    depth: u8,
) -> Option<&'static str> {
    match command {
        ExCommand::Norm { .. } => Some("runs keys"),
        ExCommand::MkVimrc { .. } => Some("writes a vimrc"),
        ExCommand::Action { name } if !is_action(name) => Some("editor shortcut"),
        ExCommand::Custom { command } if crate::host::names_own_command(command) => {
            Some("godot-vim command")
        }
        ExCommand::Set { assignments }
        | ExCommand::SetLocal { assignments }
        | ExCommand::SetGlobal { assignments } => assignments
            .iter()
            .any(set_names_blocked)
            .then_some("blocked option"),
        ExCommand::WinDo { command, .. }
        | ExCommand::BufDo { command, .. }
        | ExCommand::TabDo { command, .. }
        | ExCommand::StructuralExtract { command, .. }
        | ExCommand::StructuralComplement { command, .. } => {
            // The parser bounds the nesting; this only fails closed.
            if depth >= MAX_REACHED_NESTING {
                return Some("nested too deeply");
            }
            command_refusal(command, is_action, depth + 1)
        }
        _ => None,
    }
}

/// The flags a command a Sandbox mapping reaches may have. A plain `:w`,
/// `:e f` and buffer commands stay, as in a mapping the user types; a
/// command vim-core lacks is an error unless it is one of godot-vim's.
const REACHED_ALLOWED: Reach = Reach::OPTIONS
    .union(Reach::MESSAGE)
    .union(Reach::BUFFER)
    .union(Reach::HOST)
    .union(Reach::FILE_READ)
    .union(Reach::NAMED_PATH)
    .union(Reach::FILE_WRITE)
    .union(Reach::QUIT)
    .union(Reach::WINDOW)
    .union(Reach::NESTED)
    .union(Reach::ACTION)
    .union(Reach::CUSTOM);

const MAX_REACHED_NESTING: u8 = 16;

fn reach_reason(reach: Reach) -> &'static str {
    [
        (Reach::SHELL, "runs the shell"),
        (Reach::SOURCE, "sources a file"),
        (Reach::REGISTER_EXECUTION, "runs a register"),
        (Reach::SECURE_OPTION, "sets a secure option"),
        (Reach::MAPPING, "changes mappings"),
        (Reach::ABBREVIATION, "changes abbreviations"),
        (Reach::LEADER, "sets the leader"),
        (Reach::HANDLER, "changes handlers"),
        (Reach::VARIABLE, "sets a variable"),
    ]
    .into_iter()
    .find_map(|(flag, reason)| reach.contains(flag).then_some(reason))
    .unwrap_or("not on the Sandbox allowlist")
}

fn opaque_reason(opaque: Opaque) -> &'static str {
    match opaque {
        Opaque::Expr => "<expr>",
        Opaque::Recursive => "recursive map",
        Opaque::RegisterExecution => "runs a register",
        Opaque::FilterOperator => "runs the shell",
        Opaque::CommandWindow => "command-line window",
        Opaque::CommandLineCompletion => "command-line completion",
        Opaque::CommandLineHistory => "command-line history",
        Opaque::RegisterInsert => "inserts a register",
        Opaque::NormalKeys => "runs keys",
        Opaque::UserText => "edits what the user typed",
        Opaque::TooComplex => "too complex to follow",
        _ => "keys it cannot follow",
    }
}

fn set_names_blocked(assignment: &SetAssignment) -> bool {
    match assignment {
        SetAssignment::SetBool(name)
        | SetAssignment::UnsetBool(name)
        | SetAssignment::ToggleBool(name)
        | SetAssignment::Query(name)
        | SetAssignment::Assign(name, _)
        | SetAssignment::Append(name, _)
        | SetAssignment::Remove(name, _)
        | SetAssignment::Prepend(name, _) => is_blocked_option(name),
        SetAssignment::InvertBool(name)
        | SetAssignment::Reset { name, .. }
        | SetAssignment::Invalid { name, .. } => is_blocked_option(name),
        SetAssignment::ShowAll => false,
        _ => true,
    }
}

/// Options that can run the shell or send data out, and the `langmap`
/// family, which turns a key a right-hand side types into `:`. Long and
/// short names, as `:set` takes either; a second list after vim-core's
/// `secure`.
const BLOCKED_SET_OPTIONS: &[&str] = &[
    "shell",
    "sh",
    "shellcmdflag",
    "shcf",
    "shellpipe",
    "sp",
    "shellredir",
    "srr",
    "shellquote",
    "shq",
    "shellxquote",
    "sxq",
    "shellxescape",
    "sxe",
    "makeprg",
    "mp",
    "grepprg",
    "gp",
    "equalprg",
    "ep",
    "formatprg",
    "fp",
    "keywordprg",
    "kp",
    "langmap",
    "lmap",
    "langremap",
    "lrm",
    "langnoremap",
    "lnr",
];

/// Extract the option name from a `set` token, stripping value delimiters
/// (`=`, `?`, `!`, etc.) and the `no` or `inv` prefix (`noshell` -> `shell`).
pub(crate) fn extract_option_name_from_token(token: &str) -> &str {
    let name = token
        .split(['=', '?', '!', '+', '-', '^', ':'])
        .next()
        .unwrap_or("");
    name.strip_prefix("no")
        .or_else(|| name.strip_prefix("inv"))
        .unwrap_or(name)
}

/// Whether a `set` token, or an option name, names a blocked option.
pub(crate) fn is_blocked_option(token: &str) -> bool {
    let name = extract_option_name_from_token(token);
    BLOCKED_SET_OPTIONS
        .iter()
        .any(|blocked| name.eq_ignore_ascii_case(blocked))
}

/// Whether a `panelmap` / `panelunmap` line is safe to honour from a
/// project-level vimrc, which is committed and therefore attacker-supplied.
///
/// The parser is the arbiter, not a regex: a line that does not parse is not
/// safe, because "unparseable" and "harmless" are different claims.
///
/// The rule is about the RIGHT-hand side, and it holds because that side is a
/// **closed vocabulary**:
///
/// - `Action(id)`: a registered action id plus integer-only params. It
///   cannot expand into `:!`, `:source`, or a recursive mapping chain. An
///   unregistered id is rejected at load, so the worst case is a dead key.
/// - `native`: the give-back. Legal at EVERY trust tier because it can only
///   *reduce* what the plugin consumes; a hostile project can at most hand
///   keys back to Godot.
/// - `<Shortcut>(path)`: DENIED here. It invokes an arbitrary Godot editor
///   command by name: an open vocabulary, and outside anything this plugin
///   can reason about.
///
/// `panelunmap` is always safe: it can only remove a binding.
pub(crate) fn panel_line_is_safe(trimmed: &str) -> bool {
    use crate::config::panelmap::{parse_panel_line, PanelLine, TargetSpec};
    match parse_panel_line(trimmed) {
        Ok(Some(PanelLine::Unmap { .. })) => true,
        Ok(Some(PanelLine::Map(ref m))) => {
            !matches!(m.target, TargetSpec::Shortcut(_))
                // An `editor.*` surface is live while the VIM ENGINE owns the
                // keyboard, and the engine is the only route out of Insert
                // mode. `editor.completion` is the first such surface to carry
                // rules, and a rule there that accepts consumes the key:
                // `panelmap editor.completion <Esc> godotvim.completion.trigger`
                // parses, is admitted by every other clause here, registers,
                // resolves on every insert-like keystroke because TRIGGER
                // requires nothing, returns `Handled`, and `Elastic` consumes.
                // Escape is then dead until the vimrc is edited, which breaks
                // the invariant that a hostile project can at worst produce a
                // DEAD KEY.
                //
                // Deliberately broader than `<void>`: consumption follows from
                // acceptance, not from the flag. Deliberately narrower than
                // "no panel lines": every non-`editor.` surface, and
                // `panelunmap` on any surface, are unaffected. The user's own
                // vimrc is not sandboxed and keeps the rope.
                && !m.surface.starts_with("editor.")
        }
        // Anything that fails to parse is stripped rather than trusted.
        Ok(None) | Err(_) => false,
    }
}

#[cfg(test)]
mod oracle_tests;
#[cfg(test)]
mod tests;
