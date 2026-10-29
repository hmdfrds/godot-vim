//! Config sandboxing — strips dangerous constructs from untrusted vimrc files.
//!
//! When `project_vimrc` is set to `Sandbox`, mappings whose RHS could invoke
//! shell commands are silently removed. This prevents a malicious
//! `res://.godot-vimrc` from executing arbitrary commands on keypress.

/// Vim built-in functions that enable shell execution, file system access,
/// or indirect code evaluation. Case-sensitive because Vim functions are
/// case-sensitive (unlike ex-commands).
///
/// For `noremap` (non-recursive), single-pass RHS scanning catches these.
/// For `map` (recursive), chained expansions could compose safe fragments
/// into dangerous calls -- so recursive maps are stripped unconditionally
/// by `is_recursive_map_line`, independent of this list.
const FUNCTION_PATTERNS: &[&str] = &[
    // Shell execution
    "system(",
    "systemlist(",
    // Dynamic command/code execution
    "execute(",
    "feedkeys(",
    "timer_start(",
    "call(",
    // File system access
    "readfile(",
    "writefile(",
    "glob(",
    "delete(",
    "rename(",
    "mkdir(",
    // File system probing
    "shellescape(",
    "getfperm(",
    "getftype(",
    "getfsize(",
    "exepath(",
    // Indirect evaluation
    "eval(",
];

/// Ex-command patterns checked via abbreviation + case-insensitive matching,
/// mirroring the engine's `matches_abbrev` logic.
/// Format: `(min_prefix, full_command, what_follows)`.
const EX_COMMAND_PATTERNS: &[(&str, &str, Tail)] = &[
    // `:!` — shell execution (no abbreviation, just literal)
    ("!", "!", Tail::Any),
    // `:read !` — shell via read (abbreviation: r[ead])
    ("r", "read", Tail::Bang),
    // `:source` in every form: config chain-loading (abbreviation: so[urce])
    ("so", "source", Tail::Any),
];

/// What must follow a command name for [`EX_COMMAND_PATTERNS`] to match.
#[derive(Clone, Copy)]
enum Tail {
    Any,
    /// A `!` after optional blanks: `:r !cmd`, not `:r file`.
    Bang,
}

/// Command modifiers and the commands that run another command, as
/// `(min_prefix, full_command)`: `:silent so x` and `:windo so x` run
/// `:so x`. Vim's list, a superset of the engine's.
const COMMAND_PREFIXES: &[(&str, &str)] = &[
    ("abo", "aboveleft"),
    ("bel", "belowright"),
    ("bo", "botright"),
    ("bro", "browse"),
    ("conf", "confirm"),
    ("hid", "hide"),
    ("hor", "horizontal"),
    ("keepa", "keepalt"),
    ("keepj", "keepjumps"),
    ("kee", "keepmarks"),
    ("keepp", "keeppatterns"),
    ("lefta", "leftabove"),
    ("leg", "legacy"),
    ("loc", "lockmarks"),
    ("noa", "noautocmd"),
    ("nos", "noswapfile"),
    ("rightb", "rightbelow"),
    ("san", "sandbox"),
    ("sil", "silent"),
    ("tab", "tab"),
    ("to", "topleft"),
    ("uns", "unsilent"),
    ("verb", "verbose"),
    ("vert", "vertical"),
    ("vim9", "vim9cmd"),
    ("argdo", "argdo"),
    ("bufdo", "bufdo"),
    ("cdo", "cdo"),
    ("cfdo", "cfdo"),
    ("foldd", "folddoopen"),
    ("folddoc", "folddoclosed"),
    ("ldo", "ldo"),
    ("lfdo", "lfdo"),
    ("tabdo", "tabdo"),
    ("windo", "windo"),
];

/// `:global` (`:g!` too), `:vglobal` and vim-core's structural `:sx` and
/// `:sy`, as `(min_prefix, full_command)`. They run the command after their
/// pattern.
const GLOBAL_COMMANDS: &[(&str, &str)] = &[
    ("g", "global"),
    ("v", "vglobal"),
    ("sx", "sx"),
    ("sy", "sy"),
];

/// A vimrc after the Project Vimrc policy.
#[derive(Debug, Default)]
pub(crate) struct PolicyText {
    pub(crate) text: String,
    /// Lines the Sandbox policy disabled.
    pub(crate) stripped: usize,
}

/// Filter config text for sandbox mode using a **whitelist** approach.
///
/// Only known-safe constructs pass through (comments, blanks, safe `:set`
/// and `:setlocal`, `:filetype`, `:let mapleader`, non-recursive mappings
/// with clean RHS), and a line of several `|` commands only when each one
/// is. Everything else
/// -- including raw ex-commands like `:source` or `:!` -- is replaced with
/// a diagnostic comment. This prevents a malicious `res://.godot-vimrc`
/// from executing arbitrary commands.
fn sandbox(text: &str) -> PolicyText {
    let mut output = String::with_capacity(text.len());
    let mut stripped = 0;
    for line in text.lines() {
        let trimmed = line.trim();
        if let Some(command) = unsafe_bar_command(trimmed) {
            log::warn!(
                "sandbox: stripped line with an unsafe | command ({}) from project vimrc: {}",
                command,
                trimmed
            );
            output.push_str("\" [sandbox] stripped: ");
            stripped += 1;
            output.push_str(trimmed);
            output.push('\n');
        } else if is_safe_non_mapping_line(trimmed) {
            output.push_str(line);
            output.push('\n');
        } else if is_mapping_line(trimmed) {
            if is_recursive_map_line(trimmed) {
                // Recursive mappings are unconditionally stripped regardless of
                // RHS content. Their expansion chain can compose innocuous-looking
                // fragments into dangerous sequences at runtime, bypassing the
                // single-line shell pattern checks.
                log::warn!(
                    "sandbox: stripped recursive map (use noremap in project vimrc): {}",
                    trimmed
                );
                output.push_str(
                    "\" [sandbox] stripped recursive map (use noremap in project vimrc): ",
                );
                stripped += 1;
                output.push_str(trimmed);
                output.push('\n');
            } else if contains_shell_pattern(trimmed) {
                log::warn!(
                    "sandbox: stripped dangerous mapping from project vimrc: {}",
                    trimmed
                );
                output.push_str("\" [sandbox] stripped: ");
                stripped += 1;
                output.push_str(trimmed);
                output.push('\n');
            } else {
                output.push_str(line);
                output.push('\n');
            }
        } else if crate::config::panelmap::is_panel_line(trimmed) {
            if panel_line_is_safe(trimmed) {
                output.push_str(line);
                output.push('\n');
            } else {
                log::warn!(
                    "sandbox: stripped panel binding from project vimrc: {}",
                    trimmed
                );
                output.push_str("\" [sandbox] stripped: ");
                stripped += 1;
                output.push_str(trimmed);
                output.push('\n');
            }
        } else {
            log::warn!(
                "sandbox: stripped unrecognized line from project vimrc: {}",
                trimmed
            );
            output.push_str("\" [sandbox] stripped: ");
            stripped += 1;
            output.push_str(trimmed);
            output.push('\n');
        }
    }
    PolicyText {
        text: output,
        stripped,
    }
}

/// The first command of a line holding several that would be stripped on a
/// line of its own. vim-core splits a config line at every `|` not preceded
/// by a backslash and runs each command, so every one must pass, not only
/// the first. Splitting inside a pattern or a string only strips more.
fn unsafe_bar_command(trimmed: &str) -> Option<&str> {
    if trimmed.starts_with('"') {
        return None;
    }
    let mut commands = Vec::new();
    let mut start = 0;
    for (at, _) in trimmed.match_indices('|') {
        if !trimmed[..at].ends_with('\\') {
            commands.push(&trimmed[start..at]);
            start = at + 1;
        }
    }
    if commands.is_empty() {
        return None;
    }
    commands.push(&trimmed[start..]);
    commands
        .into_iter()
        .map(str::trim)
        .find(|command| !is_safe_command(command))
}

/// Whether `trimmed`, as a line of its own, passes the sandbox unchanged.
fn is_safe_command(trimmed: &str) -> bool {
    if is_mapping_line(trimmed) {
        return !is_recursive_map_line(trimmed) && !contains_shell_pattern(trimmed);
    }
    is_safe_non_mapping_line(trimmed)
        || (crate::config::panelmap::is_panel_line(trimmed) && panel_line_is_safe(trimmed))
}

/// Options that can execute shell commands or exfiltrate data, and the
/// `langmap` family, which turns a key a right-hand side types into `:`.
/// Both long and short forms must be listed (Vim accepts either).
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
/// (`=`, `?`, `!`, etc.) and the `no` prefix (`noshell` -> `shell`).
fn extract_option_name_from_token(token: &str) -> &str {
    let name = token
        .split(['=', '?', '!', '+', '-', '^', ':'])
        .next()
        .unwrap_or("");
    name.strip_prefix("no").unwrap_or(name)
}

/// Whether a `set` token, or an option name, names a blocked option.
fn is_blocked_option(token: &str) -> bool {
    let name = extract_option_name_from_token(token);
    BLOCKED_SET_OPTIONS
        .iter()
        .any(|blocked| name.eq_ignore_ascii_case(blocked))
}

/// Whitelist check for non-mapping lines that are safe to pass through.
/// Whether a `panelmap` / `panelunmap` line is safe to honour from a
/// project-level vimrc, which is committed and therefore attacker-supplied.
///
/// The parser is the arbiter, not a regex: a line that does not parse is not
/// safe, because "unparseable" and "harmless" are different claims.
///
/// The rule is about the RIGHT-hand side, and it holds because that side is a
/// **closed vocabulary**:
///
/// - `Action(id)` — a registered action id plus integer-only params. It
///   cannot expand into `:!`, `:source`, or a recursive mapping chain, which
///   is precisely why this module strips recursive maps unconditionally. An
///   unregistered id is rejected at load, so the worst case is a dead key.
/// - `native` — the give-back. Legal at EVERY trust tier because it can only
///   *reduce* what the plugin consumes; a hostile project can at most hand
///   keys back to Godot.
/// - `<Shortcut>(path)` — DENIED here. It invokes an arbitrary Godot editor
///   command by name: an open vocabulary, and outside anything this plugin
///   can reason about.
///
/// `panelunmap` is always safe — it can only remove a binding.
fn panel_line_is_safe(trimmed: &str) -> bool {
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
                // this module's own invariant that a hostile project can at
                // worst produce a DEAD KEY.
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

fn is_safe_non_mapping_line(trimmed: &str) -> bool {
    if trimmed.is_empty() {
        return true;
    }
    if trimmed.starts_with('"') {
        return true;
    }
    // Case-insensitive: the engine may accept `Set`, `SET`, etc.
    let trimmed_lower = trimmed.to_ascii_lowercase();
    // `:set` and `:setl[ocal]` take the same options, so they get the same
    // blocklist. A filetype plugin line in a project vimrc is `setlocal`.
    let after_set = trimmed_lower
        .split_once([' ', '\t'])
        .filter(|(cmd, _)| *cmd == "set" || *cmd == "se" || matches_abbrev(cmd, "setl", "setlocal"))
        .map(|(_, rest)| rest);
    if let Some(options_str) = after_set {
        // Check ALL space-separated options -- `set scrolloff=5 shell=/bin/evil`
        // must be blocked because `shell` is dangerous even if `scrolloff` is safe.
        return !options_str.split_whitespace().any(is_blocked_option);
    }
    // `:filetype plugin off` and the rest of `:filetype` only switch the
    // built-in filetype plugins on or off; nothing in them runs code. What
    // follows a `|` is another command, so the line is safe only when that
    // command is too. A loop, not recursion, as a line may chain any number.
    if let Some((_, mut rest)) = crate::ftplugin::split_filetype_line(trimmed) {
        while let Some(next) = rest.map(str::trim) {
            match crate::ftplugin::split_filetype_line(next) {
                Some((_, after)) => rest = after,
                None => return is_safe_non_mapping_line(next),
            }
        }
        return true;
    }
    // Only allow `let mapleader` / `let g:mapleader` -- NOT `let mapleader_hack`
    // (which could contain `system(...)` in the value expression).
    if let Some(after_prefix) = trimmed.strip_prefix("let ") {
        let after_let = after_prefix.trim_start();
        let var = if after_let.starts_with("g:mapleader") {
            Some("g:mapleader")
        } else if after_let.starts_with("mapleader") {
            Some("mapleader")
        } else {
            None
        };
        if let Some(name) = var {
            let rest = &after_let[name.len()..];
            if rest.is_empty() || rest.starts_with(|c: char| c.is_ascii_whitespace() || c == '=') {
                return true;
            }
        }
        return false;
    }
    false
}

/// Check if a line is a mapping command, using the same abbreviation matching
/// as the engine's ex_parser (`matches_abbrev`). Without this, abbreviated
/// forms like `nn`, `ino`, `cm` would bypass the sandbox.
fn is_mapping_line(trimmed: &str) -> bool {
    let cmd = match trimmed.split_once(|c: char| c.is_ascii_whitespace()) {
        Some((word, _)) => word,
        None => return false,
    };

    // The engine strips trailing `!` before abbreviation matching, so
    // `nnoremap!` would install a mapping. We must strip it too.
    let cmd = cmd.trim_end_matches('!');

    is_map_or_noremap_abbrev(cmd)
}

/// Recursive mappings (`map`, `nmap`, `vmap`, `imap`, `omap`, `cmap`) are
/// dangerous in sandbox mode because their RHS is expanded through the mapping
/// chain at runtime. Even if the immediate RHS looks safe, a chain of recursive
/// mappings can compose to produce shell-invoking sequences that bypass the
/// single-line pattern checks.
///
/// Returns `true` for recursive map commands, `false` for `noremap` variants
/// and unmap commands (which are not recursive by definition).
fn is_recursive_map_line(trimmed: &str) -> bool {
    let cmd = match trimmed.split_once(|c: char| c.is_ascii_whitespace()) {
        Some((word, _)) => word,
        None => return false,
    };

    let cmd = cmd.trim_end_matches('!');

    is_recursive_map_abbrev(cmd)
}

/// Vim abbreviation matching: `name` matches if it's a prefix of `full`
/// with at least `min.len()` characters. Case-insensitive.
fn matches_abbrev(name: &str, min: &str, full: &str) -> bool {
    let n = name.len();
    n >= min.len() && n <= full.len() && name.eq_ignore_ascii_case(&full[..n])
}

/// Matches recursive map commands only (`map`, `nm[ap]`, etc.), NOT `noremap`
/// variants or `unmap`. This distinction is critical: recursive maps are
/// unconditionally stripped in sandbox mode.
fn is_recursive_map_abbrev(name: &str) -> bool {
    if name.eq_ignore_ascii_case("map") {
        return true;
    }
    if matches_abbrev(name, "nm", "nmap") {
        return true;
    }
    if matches_abbrev(name, "vm", "vmap") {
        return true;
    }
    if matches_abbrev(name, "im", "imap") {
        return true;
    }
    if matches_abbrev(name, "om", "omap") {
        return true;
    }
    if matches_abbrev(name, "cm", "cmap") {
        return true;
    }

    false
}

/// Matches any map/noremap/unmap abbreviation. Must stay in sync with the
/// engine's `matches_abbrev` logic to prevent sandbox bypasses.
fn is_map_or_noremap_abbrev(name: &str) -> bool {
    if name.eq_ignore_ascii_case("map") {
        return true;
    }
    if matches_abbrev(name, "nm", "nmap") {
        return true;
    }
    if matches_abbrev(name, "vm", "vmap") {
        return true;
    }
    if matches_abbrev(name, "im", "imap") {
        return true;
    }
    if matches_abbrev(name, "om", "omap") {
        return true;
    }
    if matches_abbrev(name, "cm", "cmap") {
        return true;
    }

    if matches_abbrev(name, "no", "noremap") {
        return true;
    }
    if matches_abbrev(name, "nn", "nnoremap") {
        return true;
    }
    if matches_abbrev(name, "vn", "vnoremap") {
        return true;
    }
    if matches_abbrev(name, "ino", "inoremap") {
        return true;
    }
    if matches_abbrev(name, "ono", "onoremap") {
        return true;
    }
    if matches_abbrev(name, "cno", "cnoremap") {
        return true;
    }

    // Unmap could remove safety bindings the user relies on.
    if matches_abbrev(name, "unm", "unmap") {
        return true;
    }
    if matches_abbrev(name, "nun", "nunmap") {
        return true;
    }
    if matches_abbrev(name, "vu", "vunmap") {
        return true;
    }
    if matches_abbrev(name, "iu", "iunmap") {
        return true;
    }
    if matches_abbrev(name, "ou", "ounmap") {
        return true;
    }
    if matches_abbrev(name, "cu", "cunmap") {
        return true;
    }

    false
}

/// Scan a mapping line for dangerous patterns: case-sensitive function names
/// (Vim functions are case-sensitive) anywhere in the line, and
/// abbreviation-aware, case-insensitive ex-commands the right-hand side runs.
///
/// A blacklist, so not complete: register execution (`@a`, `:@a`), the `!`
/// filter operator, `<expr>` mappings and keys fed through `:normal` (from a
/// `:global` too) are not analysed (see the module docs).
fn contains_shell_pattern(line: &str) -> bool {
    if FUNCTION_PATTERNS.iter().any(|pat| line.contains(pat)) {
        return true;
    }
    let (cmd, rhs) = split_mapping(line);
    simulate_command_line(rhs, starts_in_command_line(cmd))
}

/// The map command and its right-hand side, split as the engine's
/// `parse_map_command` splits them: the `<nowait>`, `<expr>` and `<silent>`
/// flags, then the left-hand side up to the first blank.
fn split_mapping(line: &str) -> (&str, &str) {
    let (cmd, mut rest) = line
        .split_once(|c: char| c.is_ascii_whitespace())
        .unwrap_or((line, ""));
    loop {
        rest = rest.trim_start();
        let flag = rest.find('>').map(|end| &rest[..=end]);
        match flag {
            Some(flag)
                if ["<nowait>", "<expr>", "<silent>"]
                    .iter()
                    .any(|f| flag.eq_ignore_ascii_case(f)) =>
            {
                rest = &rest[flag.len()..];
            }
            _ => break,
        }
    }
    let rhs = rest
        .split_once(char::is_whitespace)
        .map_or("", |(_, rhs)| rhs);
    (cmd, rhs.trim_start())
}

/// `cmap`, `cnoremap` and the `!` forms (`map!`, `noremap!`) type their
/// right-hand side into the command line.
fn starts_in_command_line(cmd: &str) -> bool {
    cmd.ends_with('!') || cmd.starts_with(['c', 'C'])
}

/// Whether the right-hand side, run as keys, reaches a dangerous command.
///
/// Models the two ways a key sequence builds a command the engine executes:
/// a `:` command line (with its editing keys and cursor) and a `<Cmd>…<CR>`
/// sequence. A `/` or `?` search is read alongside the `:` line, as one, for
/// its bounds only; modes are not followed, so any `/` or `?` may start one.
/// Keys whose effect on the final text is not modelled (Tab, `<C-R>`, history
/// recall, word motions, …) strip the mapping, because the text cannot be
/// known, except that in a search `<C-r>` and the register it names insert
/// text the filter cannot see and only what it sees is bounded. Keys are read
/// as the engine reads a mapping, so `<Bar>`, `<Space>` and `<S-Space>` are
/// the characters `|` and ` `.
fn simulate_command_line(rhs: &str, starts_in_command_line: bool) -> bool {
    use vim_core::keymap::{Key, Modifiers};
    let mut ex = starts_in_command_line.then(CommandLine::default);
    let mut search: Option<CommandLine> = None;
    let mut cmd: Option<CommandLine> = None;

    for key in mapping_keys(rhs) {
        // `<Cmd>` starts (and restarts) a `<Cmd>…<CR>` sequence from any mode,
        // ahead of the current command line (vim-core engine.rs).
        if key.key() == Key::Cmd {
            cmd = Some(CommandLine::default());
            continue;
        }
        if let Some(line) = cmd.as_mut() {
            match key.key() {
                Key::Enter => {
                    if line_runs_dangerous(line) {
                        return true;
                    }
                    cmd = None;
                }
                Key::Escape => cmd = None,
                // Only characters reach the `<Cmd>` buffer; the engine drops
                // everything else, so modifiers on a character fall away too.
                Key::Char(c) => {
                    line.insert(c);
                    if line.exceeds_parse_bounds() {
                        return true;
                    }
                }
                _ => {}
            }
            continue;
        }
        match ex.as_mut() {
            None => {
                if matches!(key.key(), Key::Char(':'))
                    && (key.modifiers() - Modifiers::SHIFT).is_empty()
                {
                    ex = Some(CommandLine::default());
                }
            }
            Some(line) => match apply_cmdline_key(line, key) {
                CmdStep::Continue if line.exceeds_parse_bounds() => return true,
                CmdStep::Continue => {}
                CmdStep::Commit => {
                    if line_runs_dangerous(line) {
                        return true;
                    }
                    ex = None;
                }
                CmdStep::Cancel => ex = None,
                CmdStep::Unmodelled => return true,
            },
        }
        match search.as_mut() {
            None => {
                if matches!(key.key(), Key::Char('/' | '?'))
                    && (key.modifiers() - Modifiers::SHIFT).is_empty()
                {
                    search = Some(CommandLine::search());
                }
            }
            Some(line) => match apply_cmdline_key(line, key) {
                CmdStep::Continue if line.exceeds_parse_bounds() => return true,
                CmdStep::Continue => {}
                CmdStep::Commit | CmdStep::Cancel => search = None,
                CmdStep::Unmodelled => return true,
            },
        }
    }
    // A command line left open still strips: the branch strips bare and open
    // `:source` too, and a `<Cmd>` left open runs on the next key.
    cmd.is_some_and(|line| line_runs_dangerous(&line))
        || ex.is_some_and(|line| line_runs_dangerous(&line))
}

/// A command line as a right-hand side types it: the text, a byte cursor,
/// and how many of what vim-core nests the text names ([`count_nested`]),
/// kept current as the text changes so that a key takes time only in what it
/// changes.
#[derive(Default)]
struct CommandLine {
    text: String,
    cursor: usize,
    nested: usize,
    /// A `/` or `?` line, which runs only its pattern.
    search: bool,
    /// After `<C-r>` in a search: the next key names a register.
    register: bool,
}

impl CommandLine {
    fn search() -> Self {
        Self {
            search: true,
            ..Self::default()
        }
    }

    /// Whether the text is too long or too deeply nested for vim-core's
    /// parser or vim-regex. The parser reads a `:` line after every key for
    /// the live `:s` preview, which compiles the pattern, so a line that is
    /// cleared or cancelled is held to it too.
    fn exceeds_parse_bounds(&self) -> bool {
        self.text.len() > MAX_COMMAND_LINE || self.nested > MAX_NESTED
    }

    /// Replaces `range` with `with`. A name reads the 2 bytes before it and
    /// the 5 from it, so only names from 4 bytes before the change to 2 after
    /// it are counted again.
    fn splice(&mut self, range: std::ops::Range<usize>, with: &str) {
        let from = range.start.saturating_sub(4);
        let to = range.start + with.len() + 2;
        self.nested -= count_nested(&self.text, from..range.end + 2, self.search);
        self.text.replace_range(range, with);
        self.nested += count_nested(&self.text, from..to, self.search);
    }

    fn insert(&mut self, c: char) {
        self.splice(self.cursor..self.cursor, c.encode_utf8(&mut [0; 4]));
        self.cursor += c.len_utf8();
    }
}

/// The longest command line a right-hand side may type, in bytes. vim-core's
/// `split_ex_pipeline` takes time quadratic in a run of range characters.
const MAX_COMMAND_LINE: usize = 1024;

/// The most a command line a right-hand side types may name of what vim-core
/// nests ([`count_nested`]). vim-core's parser and vim-regex recurse once for
/// each, and a stack overflow aborts the editor.
const MAX_NESTED: usize = 8;

/// The commands vim-core's parser reads another command inside, and whether
/// a delimiter follows the name: `:g/pat/cmd`, `:sx/pat/cmd`, `:windo cmd`.
const NESTING_COMMANDS: &[(&str, bool)] = &[
    ("g", true),
    ("v", true),
    ("sx", true),
    ("sy", true),
    ("windo", false),
    ("bufdo", false),
    ("tabdo", false),
];

/// One committed command line, judged by the text check and by vim-core's
/// parser; either one strips. The text check takes a command to start after
/// every `|`, an over-approximation (a `|` inside a pattern or string makes
/// extra starts) which can only strip more, never less. A line too long or
/// too deeply nested to judge is stripped before either runs.
fn line_runs_dangerous(line: &CommandLine) -> bool {
    if line.exceeds_parse_bounds() {
        return true;
    }
    let line = line.text.as_str();
    let after_bars = line.match_indices('|').map(|(at, _)| &line[at + 1..]);
    starts_dangerous_command(std::iter::once(line).chain(after_bars).collect())
        || parses_to_unsafe_command(line)
}

/// How many `%(`, `%[` and (outside a search) [`NESTING_COMMANDS`] start in `starts`.
fn count_nested(line: &str, starts: std::ops::Range<usize>, search: bool) -> usize {
    let bytes = line.as_bytes();
    let is_delimiter = |b: &u8| !b.is_ascii_alphanumeric() && *b != b'_';
    // After a non-letter, or after the letter of a mark (`'ag`).
    let starts_command = |at: usize| {
        at == 0 || !bytes[at - 1].is_ascii_alphabetic() || (at > 1 && bytes[at - 2] == b'\'')
    };
    let names_command = |at: usize| {
        NESTING_COMMANDS.iter().any(|&(name, delimited)| {
            let end = at + name.len();
            bytes
                .get(at..end)
                .is_some_and(|word| word.eq_ignore_ascii_case(name.as_bytes()))
                && (!delimited || bytes.get(end).is_some_and(is_delimiter))
        })
    };
    (starts.start..starts.end.min(bytes.len()))
        .filter(|&at| {
            matches!(bytes[at..], [b'%', b'(' | b'[', ..])
                || (!search && starts_command(at) && names_command(at))
        })
        .count()
}

/// Whether one of the commands vim-core runs for `line` is unsafe, split and
/// parsed as `execute_ex_line` does. A piece that fails to parse runs nothing;
/// a line the parser panics on is unsafe.
fn parses_to_unsafe_command(line: &str) -> bool {
    use vim_core::grammar::{parse_ex_command_with_modifiers, split_ex_pipeline};
    std::panic::catch_unwind(|| {
        let pieces = split_ex_pipeline(line);
        let pieces = if pieces.len() <= 1 {
            vec![line]
        } else {
            pieces
        };
        pieces.into_iter().any(|piece| {
            parse_ex_command_with_modifiers(piece)
                .is_ok_and(|(_, command)| is_unsafe_command(&command))
        })
    })
    .unwrap_or(true)
}

/// A command a right-hand side may not run: the shell, `:source`, one that
/// changes mappings, abbreviations, handlers or the leader, or a `:set` of a
/// blocked option.
fn is_unsafe_command(command: &vim_core::grammar::types::ExCommand) -> bool {
    use vim_core::grammar::types::{ExCommand, SetAssignment};
    match command {
        ExCommand::External { .. }
        | ExCommand::Filter { .. }
        | ExCommand::Source { .. }
        | ExCommand::Map { .. }
        | ExCommand::Unmap { .. }
        | ExCommand::MapClear { .. }
        | ExCommand::Abbreviate { .. }
        | ExCommand::Unabbreviate { .. }
        | ExCommand::AbClear { .. }
        | ExCommand::SetHandler { .. }
        | ExCommand::LetMapleader { .. } => true,
        ExCommand::Set { assignments }
        | ExCommand::SetLocal { assignments }
        | ExCommand::SetGlobal { assignments } => {
            assignments.iter().any(|assignment| match assignment {
                SetAssignment::SetBool(name)
                | SetAssignment::UnsetBool(name)
                | SetAssignment::ToggleBool(name)
                | SetAssignment::Query(name)
                | SetAssignment::Assign(name, _)
                | SetAssignment::Append(name, _)
                | SetAssignment::Remove(name, _)
                | SetAssignment::Prepend(name, _) => is_blocked_option(name),
                _ => false,
            })
        }
        ExCommand::Read { path, .. } => path.starts_with('!'),
        ExCommand::WinDo { command, .. }
        | ExCommand::BufDo { command, .. }
        | ExCommand::TabDo { command, .. }
        | ExCommand::Global { command, .. }
        | ExCommand::StructuralExtract { command, .. }
        | ExCommand::StructuralComplement { command, .. } => is_unsafe_command(command),
        _ => false,
    }
}

/// How a key acts on a `:` command line.
enum CmdStep {
    /// Edit applied; keep collecting.
    Continue,
    /// Enter: run the line.
    Commit,
    /// Esc / `<C-c>`: discard the line.
    Cancel,
    /// A key whose effect on the final text is not modelled: strip, since the
    /// text cannot be known.
    Unmodelled,
}

/// Apply one key to a `:` command line, mirroring vim-core's command-line
/// handler and edit state (`mode/command_line.rs`, `state/command_line.rs`).
/// Only the editing keys that have a definite effect on the text are
/// modelled; any other key returns [`CmdStep::Unmodelled`].
fn apply_cmdline_key(line: &mut CommandLine, key: vim_core::keymap::KeyEvent) -> CmdStep {
    use vim_core::keymap::{Key, Modifiers};
    // vim-core reads any key as the register name, and `=` as the expression
    // register.
    if std::mem::take(&mut line.register) {
        return match key.key() {
            Key::Char('=') => CmdStep::Unmodelled,
            _ => CmdStep::Continue,
        };
    }
    let cursor = line.cursor;
    // The lengths of the characters before and after the cursor.
    let prev = line.text[..cursor].chars().last().map_or(0, char::len_utf8);
    let next = line.text[cursor..].chars().next().map_or(0, char::len_utf8);
    match key.key() {
        Key::Enter => return CmdStep::Commit,
        Key::Escape => return CmdStep::Cancel,
        Key::Backspace => return backspace(line, prev),
        Key::Delete => line.splice(cursor..cursor + next, ""),
        Key::Left if key.modifiers().is_empty() => line.cursor -= prev,
        Key::Right if key.modifiers().is_empty() => line.cursor += next,
        Key::Home if key.modifiers().is_empty() => line.cursor = 0,
        Key::End if key.modifiers().is_empty() => line.cursor = line.text.len(),
        Key::Char(c) if (key.modifiers() - Modifiers::SHIFT).is_empty() => line.insert(c),
        Key::Char(c) if key.modifiers() == Modifiers::CTRL => match c {
            'h' => return backspace(line, prev),
            'u' => {
                line.splice(0..cursor, "");
                line.cursor = 0;
            }
            'k' => line.splice(cursor..line.text.len(), ""),
            'c' | '[' => return CmdStep::Cancel,
            'r' if line.search => line.register = true,
            _ => return CmdStep::Unmodelled,
        },
        _ => return CmdStep::Unmodelled,
    }
    CmdStep::Continue
}

/// Delete the character before the cursor, `len` bytes long. An empty
/// command line cancels instead, as in Vim.
fn backspace(line: &mut CommandLine, len: usize) -> CmdStep {
    if line.text.is_empty() {
        return CmdStep::Cancel;
    }
    line.cursor -= len;
    line.splice(line.cursor..line.cursor + len, "");
    CmdStep::Continue
}

/// Mirrors vim-core's `key_notation::parse_impl`, the parser of mapping
/// right-hand sides, in linear time.
fn mapping_keys(rhs: &str) -> Vec<vim_core::keymap::KeyEvent> {
    use vim_core::keymap::{Key, KeyEvent};
    let mut keys = Vec::new();
    let [mut lt, mut gt, mut paren] = ['<', '>', ')'].map(|c| NextChar::new(rhs, c));
    let mut at = 0;
    while let Some(ch) = rhs[at..].chars().next() {
        if ch == '<' {
            // Only the key of a notation can be a `<` (`<<>`, `<C-<>`).
            let end = gt
                .at_or_after(at)
                .filter(|&end| lt.at_or_after(at + 1).is_none_or(|lt| lt + 1 >= end));
            if let Some(end) = end {
                if let Some(key) = KeyEvent::from_vim_notation(&rhs[at..=end]) {
                    at = end + 1;
                    // `<Action>(name)` and `<Plug>(name)` take their name.
                    if matches!(key.key(), Key::Action(_) | Key::Plug(_))
                        && rhs[at..].starts_with('(')
                    {
                        if let Some(close) = paren.at_or_after(at).filter(|&close| close > at + 1) {
                            at = close + 1;
                        }
                    }
                    keys.push(key);
                    continue;
                }
            }
        }
        keys.push(KeyEvent::char(ch));
        at += ch.len_utf8();
    }
    keys
}

/// The first `c` in `text` at or after an offset that only grows, searched
/// for again only once the offset passes it, so each byte is read once.
struct NextChar<'a> {
    text: &'a str,
    c: char,
    next: Option<usize>,
}

impl<'a> NextChar<'a> {
    fn new(text: &'a str, c: char) -> Self {
        Self {
            text,
            c,
            next: text.find(c),
        }
    }

    fn at_or_after(&mut self, offset: usize) -> Option<usize> {
        if self.next.is_some_and(|next| next < offset) {
            self.next = self.text[offset..].find(self.c).map(|at| offset + at);
        }
        self.next
    }
}

/// Whether a command typed at one of `starts`, each a tail of one line, runs
/// one of [`EX_COMMAND_PATTERNS`], also behind a range, [`COMMAND_PREFIXES`]
/// and the pattern of a [`GLOBAL_COMMANDS`] command.
fn starts_dangerous_command(mut starts: Vec<&str>) -> bool {
    // A command start is named by its length. Judging each once keeps the
    // readings of nested `:g` from multiplying, and starts that run into one
    // another (`sil '|sil '|`) linear.
    let mut judged = std::collections::HashSet::new();
    while let Some(start) = starts.pop() {
        let mut text = skip_range(start);
        while judged.insert(text.len()) {
            if EX_COMMAND_PATTERNS
                .iter()
                .any(|&(min, full, tail)| matches_ex_pattern(text, min, full, tail))
            {
                return true;
            }
            starts.extend(global_commands(text));
            match strip_command_prefix(text) {
                Some(rest) => text = skip_range(rest),
                None => break,
            }
        }
    }
    false
}

/// The commands a `:global`, `:sx` or `:sy` at the start of `text` runs
/// after its pattern. vim-core takes the character after the name (after
/// `g!` too) as the delimiter, a `!` or a blank too; Vim skips the `!` and
/// blanks first. Both are judged.
fn global_commands(text: &str) -> Vec<&str> {
    let (word, rest) = split_command_name(text);
    if !GLOBAL_COMMANDS
        .iter()
        .any(|&(min, full)| matches_abbrev(word, min, full))
    {
        return Vec::new();
    }
    let after_bang = rest.strip_prefix('!').unwrap_or(rest);
    [rest, after_bang, after_bang.trim_start()]
        .into_iter()
        .filter_map(after_pattern)
        .collect()
}

/// The text after the pattern `text` starts with, ended as vim-core's
/// `take_delimited` ends it: at the first delimiter a backslash does not
/// escape. `None` when no delimiter ends it, which runs nothing. Vim reads
/// `\/`, `\?` and `\&` as a previous pattern.
fn after_pattern(text: &str) -> Option<&str> {
    let mut chars = text.chars();
    let delim = chars.next().filter(|c| !c.is_ascii_alphabetic())?;
    let pattern = chars.as_str();
    if delim == '\\' {
        return pattern.strip_prefix(['/', '?', '&']);
    }
    let mut escaped = false;
    for (at, c) in pattern.char_indices() {
        if escaped {
            escaped = false;
        } else if c == '\\' {
            escaped = true;
        } else if c == delim {
            return Some(&pattern[at + c.len_utf8()..]);
        }
    }
    None
}

/// Skips blanks (any `char::is_whitespace`, as vim-core reads them), colons
/// and a range: line numbers, `%`, `*`, `$`, `.`, marks (`'a`, `'<`),
/// patterns (`/pat/`, `?pat?`, `\/`) and the `,;+-` between them.
fn skip_range(mut text: &str) -> &str {
    loop {
        text = text.trim_start_matches(|c: char| {
            c.is_ascii_digit()
                || c.is_whitespace()
                || matches!(c, ':' | '%' | '*' | '$' | '.' | ',' | ';' | '+' | '-')
        });
        let mut chars = text.chars();
        text = match chars.next() {
            Some('\'' | '\\') => {
                chars.next();
                chars.as_str()
            }
            Some(delim @ ('/' | '?')) => {
                let pattern = chars.as_str();
                pattern
                    .find(delim)
                    .map_or("", |end| &pattern[end + delim.len_utf8()..])
            }
            _ => return text,
        };
    }
}

/// The text after a leading command modifier, such as `silent` in
/// `silent so x`, or `None` when `text` starts with none.
fn strip_command_prefix(text: &str) -> Option<&str> {
    let (word, rest) = split_command_name(text);
    let rest = match rest.strip_prefix('!') {
        Some(after_bang) => after_bang,
        None if rest.starts_with(char::is_whitespace) => rest,
        None => return None,
    };
    COMMAND_PREFIXES
        .iter()
        .any(|&(min, full)| matches_abbrev(word, min, full))
        .then_some(rest)
}

/// A command name, which ends at the first non-letter, and the text after it.
fn split_command_name(text: &str) -> (&str, &str) {
    let end = text.find(|c: char| !c.is_ascii_alphabetic());
    text.split_at(end.unwrap_or(text.len()))
}

fn matches_ex_pattern(text: &str, min: &str, full: &str, tail: Tail) -> bool {
    let is_word = full.bytes().all(|b| b.is_ascii_alphabetic());
    (min.len()..=full.len()).any(|len| {
        // .get() avoids panic on multi-byte UTF-8 character boundaries.
        let (Some(name), Some(rest)) = (text.get(..len), text.get(len..)) else {
            return false;
        };
        // A command name ends at the first non-letter, so `:sort` is not
        // `:so` and `:source<Tab>x` is `:source`.
        if !name.eq_ignore_ascii_case(&full[..len])
            || (is_word && rest.starts_with(|c: char| c.is_ascii_alphabetic()))
        {
            return false;
        }
        match tail {
            Tail::Any => true,
            Tail::Bang => rest.trim_start().starts_with('!'),
        }
    })
}

/// Apply the project vimrc security policy. Returns `None` to block loading
/// entirely (Disabled), or the sanitized content (Sandbox) or unchanged text
/// (Trusted / user-level).
pub(crate) fn apply_vimrc_policy(
    text: &str,
    is_project_level: bool,
    policy: crate::settings::ProjectVimrc,
) -> Option<PolicyText> {
    let unchanged = || PolicyText {
        text: text.to_string(),
        stripped: 0,
    };
    if !is_project_level {
        return Some(unchanged());
    }
    match policy {
        crate::settings::ProjectVimrc::Disabled => {
            log::info!("project vimrc skipped: security/project_vimrc is set to Disabled");
            None
        }
        crate::settings::ProjectVimrc::Sandbox => {
            log::info!("project vimrc loaded in sandbox mode");
            Some(sandbox(text))
        }
        crate::settings::ProjectVimrc::Trusted => Some(unchanged()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sandbox_config_text(text: &str) -> String {
        sandbox(text).text
    }

    #[test]
    fn preserves_safe_mappings() {
        let input = "nnoremap j gj\nnnoremap k gk\n";
        let output = sandbox_config_text(input);
        assert!(output.contains("nnoremap j gj"));
        assert!(output.contains("nnoremap k gk"));
    }

    #[test]
    fn strips_shell_bang_mapping() {
        let input = "nnoremap <Leader>r :!python %<CR>\n";
        let output = sandbox_config_text(input);
        assert!(output.starts_with("\" [sandbox] stripped:"));
        assert!(!output.starts_with("nnoremap"));
    }

    #[test]
    fn strips_system_call_mapping() {
        let input = "nnoremap <Leader>x :call system('rm -rf /')<CR>\n";
        let output = sandbox_config_text(input);
        assert!(output.starts_with("\" [sandbox] stripped:"));
        assert!(!output.starts_with("nnoremap"));
    }

    #[test]
    fn preserves_set_commands() {
        let input = "set timeoutlen=500\n";
        let output = sandbox_config_text(input);
        assert!(output.contains("set timeoutlen=500"));
    }

    #[test]
    fn preserves_comments() {
        let input = "\" This is a comment with :! in it\n";
        let output = sandbox_config_text(input);
        assert!(output.contains("\" This is a comment with :! in it"));
    }

    #[test]
    fn preserves_blank_lines() {
        let input = "\n\n";
        let output = sandbox_config_text(input);
        assert_eq!(output, "\n\n");
    }

    #[test]
    fn mixed_config_partial_strip() {
        let input = "\
nnoremap j gj
nnoremap <Leader>r :!cargo run<CR>
set scrolloff=5
inoremap jk <Esc>
";
        let output = sandbox_config_text(input);
        assert!(output.contains("nnoremap j gj"));
        assert!(!output
            .lines()
            .any(|l| l.trim().starts_with("nnoremap <Leader>r :!cargo")));
        assert!(output.contains("set scrolloff=5"));
        assert!(output.contains("inoremap jk <Esc>"));
    }

    #[test]
    fn filter_range_bang() {
        // `:{range}!` style in a mapping RHS
        let input = "vnoremap <Leader>f :!sort<CR>\n";
        let output = sandbox_config_text(input);
        assert!(output.contains("[sandbox] stripped"));
    }

    #[test]
    fn strips_execute_mapping() {
        let input = "nnoremap <Leader>e :call execute('!rm -rf /')<CR>\n";
        let output = sandbox_config_text(input);
        assert!(output.starts_with("\" [sandbox] stripped:"));
    }

    #[test]
    fn strips_shellescape_mapping() {
        let input = "nnoremap <Leader>s :echo shellescape(expand('%'))<CR>\n";
        let output = sandbox_config_text(input);
        assert!(output.starts_with("\" [sandbox] stripped:"));
    }

    #[test]
    fn strips_readfile_mapping() {
        let input = "nnoremap <Leader>r :echo readfile('/etc/passwd')<CR>\n";
        let output = sandbox_config_text(input);
        assert!(output.starts_with("\" [sandbox] stripped:"));
    }

    #[test]
    fn strips_writefile_mapping() {
        let input = "nnoremap <Leader>w :call writefile(['pwned'], '/tmp/evil')<CR>\n";
        let output = sandbox_config_text(input);
        assert!(output.starts_with("\" [sandbox] stripped:"));
    }

    #[test]
    fn strips_read_bang_mapping() {
        let input = "nnoremap <Leader>r :read !cat /etc/passwd<CR>\n";
        let output = sandbox_config_text(input);
        assert!(output.contains("[sandbox] stripped"));
    }

    #[test]
    fn strips_r_bang_mapping() {
        let input = "nnoremap <Leader>r :r !cat /etc/passwd<CR>\n";
        let output = sandbox_config_text(input);
        assert!(output.contains("[sandbox] stripped"));
    }

    #[test]
    fn strips_glob_mapping() {
        let input = "nnoremap <Leader>g :echo glob('*')<CR>\n";
        let output = sandbox_config_text(input);
        assert!(output.contains("[sandbox] stripped"));
    }

    #[test]
    fn strips_getfperm_mapping() {
        let input = "nnoremap <Leader>p :echo getfperm('/etc/passwd')<CR>\n";
        let output = sandbox_config_text(input);
        assert!(output.contains("[sandbox] stripped"));
    }

    #[test]
    fn strips_getftype_mapping() {
        let input = "nnoremap <Leader>t :echo getftype('/tmp')<CR>\n";
        let output = sandbox_config_text(input);
        assert!(output.contains("[sandbox] stripped"));
    }

    #[test]
    fn strips_getfsize_mapping() {
        let input = "nnoremap <Leader>s :echo getfsize('/etc/passwd')<CR>\n";
        let output = sandbox_config_text(input);
        assert!(output.contains("[sandbox] stripped"));
    }

    #[test]
    fn strips_systemlist_mapping() {
        let input = "nnoremap <Leader>l :echo systemlist('whoami')<CR>\n";
        let output = sandbox_config_text(input);
        assert!(output.contains("[sandbox] stripped"));
    }

    #[test]
    fn strips_abbreviated_nn_shell_mapping() {
        let input = "nn <Leader>x :!rm -rf /<CR>\n";
        let output = sandbox_config_text(input);
        assert!(output.contains("[sandbox] stripped"));
    }

    #[test]
    fn strips_abbreviated_no_shell_mapping() {
        let input = "no <Leader>x :!rm -rf /<CR>\n";
        let output = sandbox_config_text(input);
        assert!(output.contains("[sandbox] stripped"));
    }

    #[test]
    fn strips_abbreviated_ino_shell_mapping() {
        let input = "ino <Leader>x :!rm -rf /<CR>\n";
        let output = sandbox_config_text(input);
        assert!(output.contains("[sandbox] stripped"));
    }

    #[test]
    fn strips_abbreviated_nm_shell_mapping() {
        let input = "nm <Leader>x :!rm -rf /<CR>\n";
        let output = sandbox_config_text(input);
        assert!(output.contains("[sandbox] stripped"));
    }

    // A `!`-suffix map and a cmap type their RHS into the command line, so a
    // shell command in it is checked and stripped.
    #[test]
    fn strips_bang_suffix_map_command() {
        let input = "nnoremap! <Leader>x !ls<CR>\n";
        let output = sandbox_config_text(input);
        assert!(output.contains("[sandbox] stripped"));
    }

    #[test]
    fn strips_abbreviated_bang_suffix() {
        let input = "nn! <Leader>x !ls<CR>\n";
        let output = sandbox_config_text(input);
        assert!(output.contains("[sandbox] stripped"));
    }

    #[test]
    fn strips_map_bang_suffix() {
        let input = "map! <Leader>x !ls<CR>\n";
        let output = sandbox_config_text(input);
        assert!(output.contains("[sandbox] stripped"));
    }

    #[test]
    fn strips_case_insensitive_map_command() {
        let input = "NNOREMAP <Leader>x :!ls<CR>\n";
        let output = sandbox_config_text(input);
        assert!(output.contains("[sandbox] stripped"));
    }

    #[test]
    fn strips_cmap_shell_mapping() {
        let input = "cmap <Leader>r <C-r>=system('whoami')<CR>\n";
        let output = sandbox_config_text(input);
        assert!(output.contains("[sandbox] stripped"));
    }

    #[test]
    fn strips_cnoremap_shell_mapping() {
        let input = "cnoremap <Leader>x !ls<CR>\n";
        let output = sandbox_config_text(input);
        assert!(output.contains("[sandbox] stripped"));
    }

    /// Vim skips colons where a command starts, so `::!ls` is `:!ls`.
    #[test]
    fn strips_shell_behind_extra_colons() {
        assert_all_stripped(&[
            "cnoremap <C-x> :!ls<CR>",
            "nnoremap x ::!ls<CR>",
            "nnoremap x <Cmd>:!ls<CR>",
            "nnoremap x :noh<Bar>:so f<CR>",
        ]);
    }

    #[test]
    fn non_mapping_with_pattern_passes_through_in_comments() {
        let input = "\" This comment mentions :read ! and glob(\n";
        let output = sandbox_config_text(input);
        assert!(output.contains(":read !"));
        assert!(output.contains("glob("));
    }

    #[test]
    fn strips_let_with_mapleader_prefix_in_variable_name() {
        // "mapleader_hack" contains "mapleader" as a prefix but is NOT
        // the mapleader variable — must be stripped by the sandbox.
        let input = "let mapleader_hack = 1\n";
        let output = sandbox_config_text(input);
        assert!(
            output.contains("[sandbox] stripped"),
            "mapleader_hack should be stripped"
        );
    }

    #[test]
    fn preserves_let_mapleader_exact() {
        let input = "let mapleader = \" \"\n";
        let output = sandbox_config_text(input);
        assert!(output.contains("let mapleader"));
        assert!(!output.contains("[sandbox] stripped"));
    }

    #[test]
    fn preserves_let_g_mapleader() {
        let input = "let g:mapleader = \",\"\n";
        let output = sandbox_config_text(input);
        assert!(output.contains("let g:mapleader"));
        assert!(!output.contains("[sandbox] stripped"));
    }

    #[test]
    fn strips_let_with_embedded_mapleader() {
        let input = "let g:x = \"mapleader\" | call system('whoami')\n";
        let output = sandbox_config_text(input);
        assert!(output.contains("[sandbox] stripped"));
    }

    #[test]
    fn strips_raw_source_outside_mapping() {
        let input = "source res://evil.vim\n";
        let output = sandbox_config_text(input);
        assert!(output.contains("[sandbox] stripped"));
    }

    #[test]
    fn strips_raw_shell_bang_outside_mapping() {
        let input = "!rm -rf /\n";
        let output = sandbox_config_text(input);
        assert!(output.contains("[sandbox] stripped"));
    }

    #[test]
    fn preserves_set_and_let_in_sandbox() {
        let input = "set scrolloff=5\nlet mapleader = \" \"\n";
        let output = sandbox_config_text(input);
        assert!(output.contains("set scrolloff=5"));
        assert!(output.contains("let mapleader"));
    }

    #[test]
    fn strips_source_mapping() {
        let input = "nnoremap <Leader>s :source res://evil.txt<CR>\n";
        let output = sandbox_config_text(input);
        assert!(output.contains("[sandbox] stripped"));
    }

    #[test]
    fn strips_so_abbreviated_mapping() {
        let input = "nnoremap <Leader>s :so res://evil.txt<CR>\n";
        let output = sandbox_config_text(input);
        assert!(output.contains("[sandbox] stripped"));
    }

    #[test]
    fn strips_source_intermediate_abbreviations() {
        for abbrev in &["sou", "sour", "sourc", "source"] {
            let input = format!("nnoremap <Leader>s :{} evil.txt<CR>\n", abbrev);
            let output = sandbox_config_text(&input);
            assert!(
                output.contains("[sandbox] stripped"),
                "failed for :{}",
                abbrev
            );
        }
    }

    const STRIPPED: &str = "\" [sandbox] stripped";

    fn assert_all_stripped(lines: &[&str]) {
        for line in lines {
            let out = sandbox_config_text(line);
            assert!(out.starts_with(STRIPPED), "kept: {line}");
        }
    }

    fn assert_all_kept(lines: &[&str]) {
        for line in lines {
            let out = sandbox_config_text(line);
            assert_eq!(out.trim_end(), *line, "stripped: {line}");
        }
    }

    /// The engine ends a command name at the first non-letter, so `so`
    /// followed by any blank, key or digit is `:source`.
    #[test]
    fn strips_source_however_its_name_ends() {
        assert_all_stripped(&[
            "nnoremap x :source<Tab>user://a.vim<CR>",
            "nnoremap x :source<Space>user://a.vim<CR>",
            "nnoremap x :so<S-Space>user://a.vim<CR>",
            "nnoremap x :sou\tuser://a.vim<CR>",
            "nnoremap x :so<C-I>user://a.vim<CR>",
            "nnoremap x :so1<CR>",
            "nnoremap x :source!<Space>user://a.vim<CR>",
            "nnoremap x :so<CR>",
            "nnoremap x :source",
        ]);
    }

    #[test]
    fn strips_source_wherever_a_command_starts() {
        assert_all_stripped(&[
            "nnoremap x :noh<Bar>so user://a.vim<CR>",
            "nnoremap x :noh\\|so user://a.vim<CR>",
            "nnoremap x :silent so user://a.vim<CR>",
            "nnoremap x :silent!so user://a.vim<CR>",
            "nnoremap x :sil so user://a.vim<CR>",
            "nnoremap x :keepjumps so user://a.vim<CR>",
            "nnoremap x :2verbose so user://a.vim<CR>",
            "nnoremap x :windo so user://a.vim<CR>",
            "nnoremap x :<Space>so<Space>user://a.vim<CR>",
            "nnoremap x f/:so user://a.vim<CR>",
            "nnoremap x r?:so user://a.vim<CR>",
            "nnoremap x :<Tab>so user://a.vim<CR>",
            "nnoremap x :\tso user://a.vim<CR>",
            "nnoremap <silent> x :so user://a.vim<CR>",
            "cnoremap <C-x> <Home>so<Space>user://a.vim",
        ]);
    }

    /// `<Cmd>…<CR>` runs its text as an ex line without a command line, so it
    /// reaches `:source` and `:!` the same way a typed `:` does.
    #[test]
    fn strips_source_and_shell_reached_through_cmd() {
        assert_all_stripped(&[
            "nnoremap x <Cmd>source user://a.vim<CR>",
            "nnoremap x <cmd>silent so user://a.vim<CR>",
            "nnoremap x <Cmd>so<Space>user://a.vim<CR>",
            "nnoremap x <Cmd>noh<Bar>so user://a.vim<CR>",
            "nnoremap x <Cmd>!ls<CR>",
        ]);
    }

    /// Editing keys move the cursor and delete, so a command word can be
    /// assembled from pieces. The command line is modelled as a buffer and a
    /// cursor, not cut at the editing keys.
    #[test]
    fn strips_source_assembled_by_editing_the_command_line() {
        assert_all_stripped(&[
            "nnoremap x :urce user://a.vim<Home>so<CR>",
            "nnoremap x :o user://a.vim<Home>s<CR>",
            "nnoremap x :soX<BS> user://a.vim<CR>",
            "nnoremap x :sox<C-h> user://a.vim<CR>",
        ]);
    }

    #[test]
    fn strips_shell_behind_a_modifier_a_bar_or_any_range() {
        assert_all_stripped(&[
            "nnoremap x :silent !ls<CR>",
            "nnoremap x :noh<Bar>!ls<CR>",
            "nnoremap x :r\t!ls<CR>",
            "nnoremap x :'a,'b!sort<CR>",
            "nnoremap x :/foo/!sort<CR>",
        ]);
    }

    /// vim-core reads any `char::is_whitespace` character as a blank, not
    /// only space and tab.
    #[test]
    fn strips_shell_and_source_behind_any_blank() {
        assert_all_stripped(&[
            "nnoremap x :\u{a0}!echo hi<CR>",
            "nnoremap x :\u{3000}!echo hi<CR>",
            "nnoremap x :\u{a0}so f<CR>",
            "nnoremap x :r\u{a0}!id<CR>",
            "nnoremap x :1,\u{a0}2!sort<CR>",
            "nnoremap x <Cmd>\u{b}!ls<CR>",
        ]);
    }

    /// Sources `line` into the engine as it is and after the sandbox: the
    /// engine maps `x` from the line itself, and nothing from what the sandbox
    /// leaves of it.
    fn assert_defines_nothing_in_the_engine(lines: &[&str]) {
        use vim_core::execution::VimEngine;
        let x = vim_core::keymap::KeyEvent::char('x');
        for line in lines {
            let mut trusted = VimEngine::new();
            let _ = trusted.source_config_text(line);
            assert!(trusted.could_start_mapping(x), "not a mapping: {line}");
            let mut engine = VimEngine::new();
            let _ = engine.source_config_text(&sandbox_config_text(line));
            assert!(!engine.could_start_mapping(x), "{line}");
        }
    }

    #[test]
    fn a_shell_line_behind_a_unicode_blank_defines_nothing_in_the_engine() {
        assert_defines_nothing_in_the_engine(&[
            "nnoremap x :\u{a0}!echo hi<CR>",
            "nnoremap x :\u{3000}!echo hi<CR>",
        ]);
    }

    /// vim-core's parser judges each committed line too, so a right-hand side
    /// cannot define a mapping, an abbreviation, a handler or the leader, or
    /// set a blocked option, however the text check reads it.
    #[test]
    fn strips_a_right_hand_side_that_changes_mappings_or_blocked_options() {
        assert_all_stripped(&[
            "nnoremap x :nnoremap y :!echo hi<lt>CR><CR>",
            "nnoremap x :nmap y Z<CR>",
            "nnoremap x :nunmap j<CR>",
            "nnoremap x :mapclear<CR>",
            "nnoremap x :iabbrev teh the<CR>",
            "nnoremap x :unabbreviate teh<CR>",
            "nnoremap x :abclear<CR>",
            "nnoremap x :sethandler n:ide<CR>",
            "nnoremap x :let mapleader = ','<CR>",
            "nnoremap x :set langmap=q:<CR>",
            "nnoremap x :setlocal shell=/bin/evil<CR>",
            "nnoremap x :setglobal kp?<CR>",
            "nnoremap x :noh<Bar>silent nnoremap y Z<CR>",
            "nnoremap x <Cmd>nnoremap y Z<CR>",
            "nnoremap x :windo nnoremap y Z<CR>",
            "nnoremap x :bufdo set mp=evil<CR>",
            "cnoremap <C-x> <C-u>nnoremap y Z<CR>",
            "nnoremap x :nnoremap y Z",
        ]);
    }

    #[test]
    fn keeps_ordinary_command_line_mappings() {
        assert_all_kept(&[
            "nnoremap <Leader>w :set wrap!<CR>",
            "nnoremap <Esc><Esc> :noh<CR>",
            "nnoremap x :echo 'hi'<CR>",
            "nnoremap x <Cmd>set hlsearch!<CR>",
            "nnoremap x :set so=5 tw=80 nonumber ic?<CR>",
            "nnoremap x :setlocal fo-=t<CR>",
            "nnoremap x :windo set wrap<CR>",
            "nnoremap x :r notes.txt<CR>",
            "nnoremap x :g/TODO/d<CR>",
            "nnoremap x :e res://main.gd<CR>",
            "nnoremap x :undefined<CR>",
        ]);
    }

    #[test]
    fn a_mapping_that_defines_a_mapping_defines_nothing_in_the_engine() {
        assert_defines_nothing_in_the_engine(&["nnoremap x :nnoremap y :!echo hi<lt>CR><CR>"]);
    }

    /// `:g`, `:g!` and `:v` run the command after their pattern, so they are
    /// judged by that command.
    #[test]
    fn strips_a_global_that_runs_a_stripped_command() {
        assert_all_stripped(&[
            "nnoremap x :g/x/so user://a.vim<CR>",
            "nnoremap x :g!/x/so user://a.vim<CR>",
            "nnoremap x :v/x/so user://a.vim<CR>",
            "nnoremap x :g/x/!ls<CR>",
            "nnoremap x :v/x/r !ls<CR>",
            "nnoremap x :g/x/nnoremap y Z<CR>",
            "nnoremap x :v/x/set shell=/bin/evil<CR>",
            "nnoremap x :g/x/windo so f<CR>",
            "nnoremap x :g/x/v/y/so f<CR>",
            "nnoremap x :gl/x/so f<CR>",
            "nnoremap x :global/x/so f<CR>",
            "nnoremap x :global!/x/so f<CR>",
            "nnoremap x :vg/x/so f<CR>",
            "nnoremap x :vglobal/x/!ls<CR>",
            "nnoremap x <Cmd>g/x/so f<CR>",
        ]);
    }

    #[test]
    fn strips_a_global_behind_a_range_a_modifier_or_a_bar() {
        assert_all_stripped(&[
            "nnoremap x :%g/x/so f<CR>",
            "nnoremap x :'<,'>v/x/so f<CR>",
            "nnoremap x :silent g/x/so f<CR>",
            "nnoremap x :windo g!/x/so f<CR>",
            "nnoremap x :noh<Bar>g/x/so f<CR>",
            "nnoremap x :noh\\|v/x/!ls<CR>",
            "nnoremap x :g/x/silent so f<CR>",
            "nnoremap x :g/x/ :.,+1so f<CR>",
        ]);
    }

    /// The pattern ends at the first delimiter a backslash does not escape. A
    /// `!` or a blank after `g` is read both as vim-core reads it, as the
    /// delimiter, and as Vim does, skipped.
    #[test]
    fn strips_a_global_however_its_pattern_is_delimited() {
        assert_all_stripped(&[
            "nnoremap x :g/a\\/b/so f<CR>",
            "nnoremap x :g#x#so f<CR>",
            "nnoremap x :v,a\\,b,!ls<CR>",
            "nnoremap x :g!x!so f<CR>",
            "nnoremap x :g /x/so f<CR>",
            "nnoremap x :g! /x/so f<CR>",
            "nnoremap x :g\\/so f<CR>",
            "nnoremap x :g/a<Bar>b/so f<CR>",
        ]);
    }

    /// `:normal` is not analysed, from a `:global` either.
    #[test]
    fn keeps_a_global_that_runs_a_safe_command() {
        assert_all_kept(&[
            "nnoremap x :g/x/d<CR>",
            "nnoremap x :g/x/s/a/b/<CR>",
            "nnoremap x :g!/x/d<CR>",
            "nnoremap x :v/x/normal dd<CR>",
            "nnoremap x :g/so f/d<CR>",
            "nnoremap x :g/a\\/so f/d<CR>",
            "nnoremap x :g/so f<CR>",
        ]);
    }

    #[test]
    fn keeps_commands_that_only_start_like_global() {
        assert_all_kept(&[
            "nnoremap x :goto 5<CR>",
            "nnoremap x :grep TODO<CR>",
            "nnoremap x :vsplit<CR>",
            "nnoremap x :vertical new<CR>",
            "nnoremap x :visual res://a.gd<CR>",
            "nnoremap x :view res://a.gd<CR>",
            "nnoremap x :vnew<CR>",
        ]);
    }

    /// vim-core's structural `:sx` and `:sy` run the command after their
    /// pattern, as `:g` does, and the text check and the parser each judge it.
    #[test]
    fn strips_a_structural_command_that_runs_a_stripped_command() {
        assert_all_stripped(&[
            "nnoremap x :sx/x/so user://a.vim<CR>",
            "nnoremap x :sy/x/!ls<CR>",
            "nnoremap x :sx#x#nnoremap y Z<CR>",
            "nnoremap x :sy/x/set shell=/bin/evil<CR>",
            "nnoremap x :silent sx/x/so f<CR>",
            "nnoremap x :g/x/sy/y/so f<CR>",
            "nnoremap x :windo sx/x/nnoremap y Z<CR>",
            "nnoremap x <Cmd>sy/x/so f<CR>",
        ]);
        for line in ["sx/x/so f", "sy#x#!ls", "windo sx/x/so f"] {
            assert!(starts_dangerous_command(vec![line]), "{line}");
            assert!(parses_to_unsafe_command(line), "{line}");
        }
    }

    #[test]
    fn keeps_a_structural_command_that_runs_a_safe_command() {
        assert_all_kept(&[
            "nnoremap x :sx/x/d<CR>",
            "nnoremap x :sy/x/d<CR>",
            "nnoremap x :sx/so f/d<CR>",
        ]);
    }

    #[test]
    fn a_global_that_runs_a_stripped_command_defines_nothing_in_the_engine() {
        assert_defines_nothing_in_the_engine(&[
            "nnoremap x :g/x/so user://a.vim<CR>",
            "nnoremap x :v/x/nnoremap y Z<CR>",
            "nnoremap x :global!/x/!ls<CR>",
        ]);
    }

    /// Runs `f` on a 1 MiB stack, the main thread's on Windows.
    fn on_a_small_stack<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
        std::thread::Builder::new()
            .stack_size(1 << 20)
            .spawn(f)
            .unwrap()
            .join()
            .unwrap()
    }

    /// vim-core's parser recurses once for each command run inside another.
    #[test]
    fn strips_a_command_line_too_deeply_nested_or_too_long_to_judge() {
        for (unit, depth) in [
            ("g/x/", 200),
            ("g//", 300),
            ("g!/x/", 200),
            ("v/x/", 200),
            ("sx/x/", 200),
            ("sy/x/", 200),
            ("windo ", 100),
            ("bufdo ", 100),
            ("TabDo ", 100),
            ("silent windo g/x/", 50),
            ("tabdo sx/x/bufdo v/x/", 40),
            ("1g/x/", 200),
            ("'ag/x/", 160),
            ("g/x/", 2000),
            ("windo ", 2000),
        ] {
            let commands = format!("{}d", unit.repeat(depth));
            for line in [
                format!("nnoremap x :{commands}<CR>"),
                format!("nnoremap x <Cmd>{commands}<CR>"),
            ] {
                let out = on_a_small_stack(move || sandbox_config_text(&line));
                assert!(out.starts_with(STRIPPED), "kept: {unit} x {depth}");
            }
        }
    }

    #[test]
    fn strips_a_command_line_past_either_bound() {
        for (start, end) in [
            (":", "<CR>"),
            (":", "<Esc>"),
            (":", "<C-u>noh<CR>"),
            ("<Cmd>", "<Esc>"),
        ] {
            let nested = |commands: usize, groups: usize, sequences: usize| {
                format!(
                    "nnoremap x {start}{}s/{}{}a{}{}/b/{end}",
                    "windo ".repeat(commands),
                    "\\%(".repeat(groups),
                    "\\%[".repeat(sequences),
                    "]".repeat(sequences),
                    "\\)".repeat(groups),
                )
            };
            let long = |len: usize| format!("nnoremap x {start}{:len$}{end}", "noh");
            assert_all_kept(&[
                nested(MAX_NESTED, 0, 0).as_str(),
                nested(0, MAX_NESTED, 0).as_str(),
                nested(0, 0, MAX_NESTED).as_str(),
                nested(4, 2, 2).as_str(),
                long(MAX_COMMAND_LINE).as_str(),
            ]);
            assert_all_stripped(&[
                nested(MAX_NESTED + 1, 0, 0).as_str(),
                nested(0, MAX_NESTED + 1, 0).as_str(),
                nested(0, 0, MAX_NESTED + 1).as_str(),
                nested(4, 2, 3).as_str(),
                long(MAX_COMMAND_LINE + 1).as_str(),
            ]);
        }
    }

    /// A `%(` or `%[` opens a group or an optional sequence wherever it is.
    /// The `v` of `\v` before a delimiter could start a `:v`, so it counts too.
    #[test]
    fn counts_the_groups_and_sequences_a_pattern_opens() {
        for (line, count) in [
            ("s/\\%(a\\)\\%(b\\)/c/", 2),
            ("s/\\(a\\)\\(b\\)/c/", 0),
            ("s/\\v%(a)%(b)/c/", 3),
            ("s/\\va%(b)/c/", 1),
            ("s/fu\\%[nction]/c/", 1),
            ("s/\\%[a\\%[b]]/c/", 2),
            ("s/\\v%[ab]/c/", 2),
            ("s/[ab]/c/", 0),
        ] {
            assert_eq!(count_nested(line, 0..line.len(), false), count, "{line}");
        }
    }

    /// Sources `config` into `one` and presses `x`, on a 1 MiB stack.
    fn press_x_on_a_small_stack(config: String) -> String {
        use vim_core::execution::HostSession;
        use vim_core::keymap::KeyEvent;
        on_a_small_stack(move || {
            let mut session = HostSession::new("one\n");
            let _ = session.source_config(&config);
            let _ = session.process_key_host(KeyEvent::char('x'));
            session.text().to_string()
        })
    }

    /// The live `:s` preview parses a `:` line after every key.
    #[test]
    fn strips_a_command_line_cleared_after_it_was_too_deeply_nested() {
        for typed in [
            format!("{}d", "g/x/".repeat(200)),
            format!("s/{}{}/y/", "\\%(".repeat(200), "\\)".repeat(200)),
        ] {
            let each = |key: &str| key.repeat(typed.len());
            for end in [
                "<Esc>".to_string(),
                "<C-c>".to_string(),
                "<C-[>".to_string(),
                "<C-u>".to_string(),
                "<C-u>noh<CR>".to_string(),
                "<Home><C-k>noh<CR>".to_string(),
                format!("{}noh<CR>", each("<BS>")),
                format!("{}<BS>", each("<C-h>")),
                format!("<Home>{}noh<CR>", each("<Del>")),
            ] {
                let out = sandbox_config_text(&format!("nnoremap x :{typed}{end}"));
                assert!(out.starts_with(STRIPPED), "kept: {end}");
                assert_eq!(press_x_on_a_small_stack(out), "ne\n", "{end}");
            }
        }
    }

    /// vim-regex recurses once for each group and optional sequence.
    #[test]
    fn strips_a_command_line_whose_pattern_is_too_deeply_nested() {
        let groups = format!("{}{}", "\\%(".repeat(200), "\\)".repeat(200));
        let sequences = "\\%[".repeat(339);
        for rhs in [
            format!(":s/{sequences}a/y/<CR>"),
            format!(":s/{sequences}a/y/<Esc>"),
            format!(":s/{sequences}a/y/<C-u>noh<CR>"),
            format!(":g/{sequences}a/d<CR>"),
            format!("<Cmd>s/{sequences}a/y/<CR>"),
            format!(":s/{groups}/y/<CR>"),
            format!(":s/\\v{groups}/y/<CR>"),
            format!(":s/\\V{groups}/y/<CR>"),
            format!(":g/{groups}/d<CR>"),
            format!(":sort /{groups}/<CR>"),
            format!(":/{groups}/d<CR>"),
            format!("<Cmd>s/{groups}/y/<CR>"),
            format!(":s/{}a<Esc>", "\\%(".repeat(200)),
        ] {
            let out = sandbox_config_text(&format!("nnoremap x {rhs}"));
            assert!(out.starts_with(STRIPPED), "kept: {rhs:.20}");
            assert_eq!(press_x_on_a_small_stack(out), "ne\n", "{rhs:.20}");
        }
    }

    /// vim-core compiles the pattern of a search when it runs.
    #[test]
    fn strips_a_search_whose_pattern_is_too_deeply_nested() {
        let groups = format!("{}a{}", "\\%(".repeat(200), "\\)".repeat(200));
        let assembled = format!("{}a{}", "\\(<Left>%<End>".repeat(200), "\\)".repeat(200));
        for rhs in [
            format!("/{groups}<CR>"),
            format!("?{groups}<CR>"),
            format!("d/{groups}<CR>"),
            format!("i<C-o>/{groups}<CR>"),
            format!("/{assembled}<CR>"),
            format!("/{}a<CR>", "\\%[".repeat(400)),
            format!("/\\v{}a{}<CR>", "%(".repeat(200), ")".repeat(200)),
            format!("/{groups}"),
            format!("/{groups}<Esc>"),
        ] {
            let out = sandbox_config_text(&format!("nnoremap x {rhs}"));
            assert!(out.starts_with(STRIPPED), "kept: {rhs:.20}");
            assert_eq!(press_x_on_a_small_stack(out), "ne\n", "{rhs:.20}");
        }
    }

    /// A search counts only the groups and sequences of its pattern.
    #[test]
    fn strips_a_search_past_either_bound() {
        let nested = |groups: usize| {
            let (open, close) = ("\\%(".repeat(groups), "\\)".repeat(groups));
            format!("nnoremap x /{open}a{close}<CR>")
        };
        let long = |len: usize| format!("nnoremap x /{}<CR>", "a".repeat(len));
        assert_all_kept(&[
            nested(MAX_NESTED).as_str(),
            long(MAX_COMMAND_LINE).as_str(),
            "nnoremap x /\\v<(g<Bar>v<Bar>sx<Bar>sy<Bar>windo<Bar>bufdo<Bar>tabdo<Bar>g<Bar>v)><CR>",
        ]);
        assert_all_stripped(&[
            nested(MAX_NESTED + 1).as_str(),
            long(MAX_COMMAND_LINE + 1).as_str(),
            "nnoremap x :s/\\v<(g<Bar>v<Bar>sx<Bar>sy<Bar>windo<Bar>bufdo<Bar>tabdo<Bar>g<Bar>v)>//<CR>",
        ]);
    }

    #[test]
    fn keeps_mappings_that_search_a_few_groups_or_sequences() {
        assert_all_kept(&[
            "nnoremap x /\\%(foo\\|bar\\)\\(baz\\)<CR>",
            "nnoremap x ?fu\\%[nction]<CR>",
            "nnoremap x d/\\v<end%[if]><CR>",
            "nnoremap x /foo<BS><BS>ar<Home>\\<<End>\\><CR>",
            "nnoremap <Leader>s /\\v",
            "inoremap <C-l> /* */<Left><Left><Left>",
        ]);
    }

    /// In a search, `<C-r>` and the register it names insert text the filter
    /// cannot see, and what it sees is still bounded.
    #[test]
    fn keeps_a_search_that_inserts_a_register() {
        assert_all_kept(&[
            "vnoremap // y/<C-r>\"<CR>",
            "nnoremap x /<C-r><C-w><CR>",
            "nnoremap x ?\\<<C-r><C-a>\\><CR>",
            "nnoremap x /<C-r>/<BS><CR>",
            "nnoremap x /<C-r><Esc>a<CR>",
        ]);
        let groups = |n: usize| "\\%(".repeat(n);
        assert_all_stripped(&[
            format!("nnoremap x /{}<C-r>a{}<CR>", groups(4), groups(5)).as_str(),
            format!("nnoremap x /<C-r>a{}<CR>", "a".repeat(MAX_COMMAND_LINE + 1)).as_str(),
        ]);
    }

    #[test]
    fn keeps_mappings_that_type_and_correct_a_short_command_line() {
        assert_all_kept(&[
            "nnoremap x :windo set nu<C-u>noh<CR>",
            "nnoremap x :g/x/d<BS>s/a/b/<CR>",
            "nnoremap x :bufdo e<Esc>:noh<CR>",
            "nnoremap x :tabdo windo set nu<Home><C-k>noh<CR>",
            "nnoremap x :%s/a/b/g<Left><Left><BS>c<CR>",
            "nnoremap x <Cmd>windo set nu<Esc><Cmd>noh<CR>",
            "cnoremap <C-x> <C-u>windo set nu",
        ]);
    }

    /// The count a command line keeps as it is edited is the count of its
    /// text, whatever the edit.
    #[test]
    fn a_command_line_keeps_count_of_what_it_nests() {
        let edits = mapping_keys("<Left><Right><Home><End><BS><Del><C-u><C-k>");
        let words: Vec<&str> = NESTING_COMMANDS
            .iter()
            .map(|&(name, _)| name)
            .chain(["G", "Windo", " ", "'", "a", "/", "!", "_", "|", "1", "é"])
            .chain(["%(", "%[", "%", "(", "[", "\\"])
            .collect();
        for mut line in [CommandLine::default(), CommandLine::search()] {
            let mut seed = 1_u32;
            for _ in 0..20_000 {
                seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12_345);
                let pick = (seed >> 17) as usize;
                let keys = if seed & (1 << 16) == 0 {
                    vec![edits[pick % edits.len()]]
                } else {
                    mapping_keys(words[pick % words.len()])
                };
                for key in keys {
                    let _ = apply_cmdline_key(&mut line, key);
                }
                let all = count_nested(&line.text, 0..line.text.len(), line.search);
                assert_eq!(line.nested, all, "{}", line.text);
            }
        }
    }

    /// Capturing groups do not count: vim-regex allows no more than nine.
    #[test]
    fn keeps_mappings_that_nest_a_few_commands_or_groups() {
        assert_all_kept(&[
            "nnoremap x :g/^$/d<CR>",
            "nnoremap x :windo set nu<CR>",
            "nnoremap x :tabdo windo set nu<CR>",
            "nnoremap x :silent keepjumps g/x/d<CR>",
            "nnoremap x :silent! windo g/x/s/a/b/g<CR>",
            "nnoremap x :bufdo silent %s/a/b/ge<CR>",
            "nnoremap x <Cmd>windo diffthis<CR>",
            "nnoremap x :%s/\\%(foo\\|bar\\)\\(baz\\)/\\1/g<CR>",
            "nnoremap x :g/\\%(a\\|b\\)/s/\\v%(c\\|d)+/e/<CR>",
            "nnoremap x :%s/fu\\%[nction]/func/g<CR>",
            "nnoremap x :g/\\v<end%[if]>/d<CR>",
            "nnoremap x :s/\\(\\(\\(\\(\\(\\(\\(\\(\\(a\\)\\)\\)\\)\\)\\)\\)\\)\\)/\\9/<CR>",
        ]);
    }

    /// A `g` or `v` that ends a word cannot start a command.
    #[test]
    fn keeps_a_command_line_with_words_that_end_in_g_or_v() {
        assert_all_kept(&[
            "nnoremap x :e res://debug/log/img/svg/config/rev/nav/dev/env/tag.gd<CR>",
        ]);
    }

    /// `:filetype` lines chain without recursion, so a long chain still passes.
    #[test]
    fn keeps_a_long_chain_of_filetype_commands() {
        let line = format!("{}filetype on", "filetype plugin on|".repeat(5_000));
        let out = on_a_small_stack(move || {
            let out = sandbox_config_text(&line);
            out.trim_end() == line
        });
        assert!(out, "a :filetype chain was stripped");
    }

    /// vim-core's `split_ex_pipeline` may panic on a mark followed by a
    /// character of more than one byte. Judging such a line never crashes,
    /// and stripped or kept, it reaches no shell and sources nothing.
    #[test]
    fn a_command_line_the_parser_may_panic_on_is_judged_without_crashing() {
        use vim_core::execution::{HostRequest, HostSession};
        for line in [
            "nnoremap x :'é<CR>",
            "nnoremap x :1,'éd<CR>",
            "nnoremap x <Cmd>'é<CR>",
            "nnoremap x :'é",
        ] {
            let mut session = HostSession::new("one\ntwo\n");
            let _ = session.source_config(&sandbox_config_text(line));
            for key in [
                vim_core::keymap::KeyEvent::char('x'),
                vim_core::keymap::KeyEvent::enter(),
            ] {
                for request in session.process_key_host(key).host_requests {
                    assert!(
                        !matches!(
                            request,
                            HostRequest::ExternalCommand { .. }
                                | HostRequest::ReadConfigFile { .. }
                        ),
                        "{line}"
                    );
                }
            }
        }
        assert_all_kept(&["nnoremap x :s/'é/e/<CR>", "nnoremap x :'a,'bs/é/e/<CR>"]);
    }

    /// Each vimrc runs `:!echo hi` when its keys are pressed, and runs nothing
    /// after the sandbox.
    #[test]
    fn the_known_routes_to_the_shell_run_nothing_after_the_sandbox() {
        use vim_core::execution::{HostRequest, HostSession};
        let shell_commands = |text: &str, keys: &str| {
            let mut session = HostSession::new("a/b?c\ntwo\n");
            let _ = session.source_config(text);
            let mut commands = Vec::new();
            for ch in keys.chars() {
                let response = session.process_key_host(vim_core::keymap::KeyEvent::char(ch));
                for request in response.host_requests {
                    if let HostRequest::ExternalCommand { command, .. } = request {
                        commands.push(command.to_string());
                    }
                }
            }
            commands
        };
        for (text, keys) in [
            ("nnoremap x :!echo hi<CR>\n", "x"),
            ("nnoremap x :\u{a0}!echo hi<CR>\n", "x"),
            ("nnoremap x :\u{3000}!echo hi<CR>\n", "x"),
            ("nnoremap x :nnoremap y :!echo hi<lt>CR><CR>\n", "xy"),
            ("set langmap=q:\nnnoremap x 0q!echo hi<CR>\n", "x"),
            ("nnoremap x f/:!echo hi<CR>\n", "x"),
            ("nnoremap x t/:!echo hi<CR>\n", "x"),
            ("nnoremap x $F/:!echo hi<CR>\n", "x"),
            ("nnoremap x $T/:!echo hi<CR>\n", "x"),
            ("nnoremap x f?:!echo hi<CR>\n", "x"),
            ("nnoremap x df/:!echo hi<CR>\n", "x"),
            ("nnoremap x r/:!echo hi<CR>\n", "x"),
            ("nnoremap x m/:!echo hi<CR>\n", "x"),
            ("nnoremap x @/:!echo hi<CR>\n", "x"),
            ("nnoremap x \"/:!echo hi<CR>\n", "x"),
            ("nnoremap x `/:!echo hi<CR>\n", "x"),
            ("nnoremap x '/:!echo hi<CR>\n", "x"),
            ("nnoremap x $f/:!echo hi<CR>\n", "x"),
            ("nnoremap x r?:!echo hi<CR>\n", "x"),
            ("nnoremap x f/:nnoremap y :!echo hi<lt>CR><CR>\n", "xy"),
        ] {
            assert_eq!(shell_commands(text, keys), ["echo hi"], "{text}");
            assert!(
                shell_commands(&sandbox_config_text(text), keys).is_empty(),
                "{text}"
            );
        }
    }

    #[test]
    fn keeps_mappings_that_only_resemble_a_dangerous_command() {
        assert_all_kept(&[
            "nnoremap x :sort<CR>",
            "nnoremap x :set so=5<CR>",
            "nnoremap x :echo 'so far'<CR>",
            "nnoremap x :tabnew so.gd<CR>",
            "nnoremap x :retab<CR>",
            "nnoremap <Leader>so :w<CR>",
            "nnoremap x <C-w>so",
            "nnoremap x :w<CR><C-w>so",
            "nnoremap x :%s/a/so/g<Left><Left>",
            "nnoremap x <Cmd>noh<CR>",
            "nnoremap x <Cmd>w<CR>",
        ]);
    }

    /// A command-line key whose effect on the final text is not modelled
    /// (`<Tab>`, `<C-r>`, history recall, …) strips the mapping, since the
    /// text that would run cannot be known.
    #[test]
    fn strips_command_line_with_an_unmodelled_key() {
        assert_all_stripped(&[
            "nnoremap x :echo<C-r>a<CR>",
            "nnoremap x :echo<Up><CR>",
            "nnoremap x :echo<C-f>",
            "cnoremap <C-x> <C-r>a",
            "nnoremap x ?<Up><CR>",
            "nnoremap x /<C-r>=@a<CR><CR>",
        ]);
    }

    #[test]
    fn strips_read_intermediate_abbreviations() {
        for abbrev in &["r", "re", "rea", "read"] {
            let input = format!("nnoremap <Leader>r :{} !cat /etc/passwd<CR>\n", abbrev);
            let output = sandbox_config_text(&input);
            assert!(
                output.contains("[sandbox] stripped"),
                "failed for :{}",
                abbrev
            );
        }
    }

    #[test]
    fn strips_case_insensitive_ex_commands() {
        let cases = &[
            "nnoremap <Leader>r :Read !cat /etc/passwd<CR>\n",
            "nnoremap <Leader>r :READ !cat /etc/passwd<CR>\n",
            "nnoremap <Leader>r :R !cat /etc/passwd<CR>\n",
            "nnoremap <Leader>s :Source evil.txt<CR>\n",
            "nnoremap <Leader>s :SOURCE evil.txt<CR>\n",
            "nnoremap <Leader>s :So evil.txt<CR>\n",
            "nnoremap <Leader>s :SOU evil.txt<CR>\n",
        ];
        for case in cases {
            let output = sandbox_config_text(case);
            assert!(
                output.contains("[sandbox] stripped"),
                "failed for: {}",
                case.trim()
            );
        }
    }

    #[test]
    fn strips_range_bang_patterns() {
        let cases = &[
            "nnoremap <Leader>x :%!sort<CR>\n",
            "nnoremap <Leader>x :'<,'>!sort<CR>\n",
            "nnoremap <Leader>x :1,$!sort<CR>\n",
            "nnoremap <Leader>x :.!sh<CR>\n",
        ];
        for case in cases {
            let output = sandbox_config_text(case);
            assert!(
                output.contains("[sandbox] stripped"),
                "failed for: {}",
                case.trim()
            );
        }
    }

    #[test]
    fn no_panic_on_multibyte_after_colon() {
        // Should not panic on multi-byte UTF-8 after `:` — just pass through.
        let input = "nnoremap <Leader>x :ñfoo<CR>\n";
        let output = sandbox_config_text(input);
        assert!(output.contains("nnoremap")); // safe mapping, not stripped
    }

    /// A `<` inside a notation is read only as its key, as vim-core reads it.
    #[test]
    fn reads_a_notation_as_the_engine_does() {
        let read = |rhs: &str| -> String {
            mapping_keys(rhs)
                .iter()
                .map(|key| key.to_vim_notation())
                .collect()
        };
        for (rhs, keys) in [
            ("<<>", "<lt>"),
            ("<<<>", "<lt><lt>"),
            ("<C-<>", "<C-<>"),
            ("<a<CR>", "<lt>a<CR>"),
            ("<<CR>>", "<lt><CR><gt>"),
            ("<C-<C-x>", "<lt>C-<C-x>"),
            ("<F0001>", "<F1>"),
            ("<lt", "<lt>lt"),
        ] {
            assert_eq!(read(rhs), keys, "{rhs}");
        }
        assert_eq!(mapping_keys("<Plug>(a)x").len(), 2);
        assert_eq!(mapping_keys("<Plug>()x").len(), 4);
        assert_eq!(mapping_keys("<Plug>(<Plug>(a)x").len(), 2);
    }

    #[test]
    fn reads_a_long_run_of_angle_brackets_in_linear_time() {
        let n = 1 << 20;
        assert_eq!(mapping_keys(&"<".repeat(n)).len(), n);
        assert_eq!(mapping_keys(&format!("{}>", "<".repeat(n))).len(), n - 1);
        assert_eq!(mapping_keys(&"<Plug>(".repeat(n / 8)).len(), n / 4);
    }

    // --- Recursive mapping stripping tests ---

    #[test]
    fn strips_recursive_nmap_with_safe_rhs() {
        // Even though the RHS is safe, recursive `nmap` is stripped because
        // chained recursive mappings can compose dangerous expansions.
        let input = "nmap j gj\n";
        let output = sandbox_config_text(input);
        assert!(
            output.contains("[sandbox] stripped recursive map"),
            "recursive nmap with safe RHS should be stripped, got: {}",
            output
        );
        assert!(!output.starts_with("nmap"));
    }

    #[test]
    fn preserves_nnoremap_with_safe_rhs() {
        // Non-recursive `nnoremap` with safe RHS should pass through.
        let input = "nnoremap j gj\n";
        let output = sandbox_config_text(input);
        assert!(
            output.contains("nnoremap j gj"),
            "non-recursive nnoremap with safe RHS should pass through, got: {}",
            output
        );
        assert!(!output.contains("[sandbox] stripped"));
    }

    #[test]
    fn strips_recursive_map_with_safe_rhs() {
        // `map` (mode-agnostic recursive) is stripped even with safe RHS.
        let input = "map j gj\n";
        let output = sandbox_config_text(input);
        assert!(
            output.contains("[sandbox] stripped recursive map"),
            "recursive map with safe RHS should be stripped, got: {}",
            output
        );
        assert!(!output.starts_with("map j"));
    }

    #[test]
    fn strips_recursive_vmap_with_safe_rhs() {
        let input = "vmap <Leader>y \"+y\n";
        let output = sandbox_config_text(input);
        assert!(output.contains("[sandbox] stripped recursive map"));
    }

    #[test]
    fn strips_recursive_imap_with_safe_rhs() {
        let input = "imap jk <Esc>\n";
        let output = sandbox_config_text(input);
        assert!(output.contains("[sandbox] stripped recursive map"));
    }

    #[test]
    fn strips_recursive_omap_with_safe_rhs() {
        let input = "omap p ip\n";
        let output = sandbox_config_text(input);
        assert!(output.contains("[sandbox] stripped recursive map"));
    }

    #[test]
    fn strips_recursive_cmap_with_safe_rhs() {
        let input = "cmap <C-a> <Home>\n";
        let output = sandbox_config_text(input);
        assert!(output.contains("[sandbox] stripped recursive map"));
    }

    #[test]
    fn strips_abbreviated_recursive_nm() {
        let input = "nm j gj\n";
        let output = sandbox_config_text(input);
        assert!(output.contains("[sandbox] stripped recursive map"));
    }

    #[test]
    fn strips_abbreviated_recursive_vm() {
        let input = "vm <Leader>y \"+y\n";
        let output = sandbox_config_text(input);
        assert!(output.contains("[sandbox] stripped recursive map"));
    }

    #[test]
    fn strips_abbreviated_recursive_im() {
        let input = "im jk <Esc>\n";
        let output = sandbox_config_text(input);
        assert!(output.contains("[sandbox] stripped recursive map"));
    }

    #[test]
    fn strips_recursive_map_with_bang_suffix() {
        let input = "nmap! j gj\n";
        let output = sandbox_config_text(input);
        assert!(output.contains("[sandbox] stripped recursive map"));
    }

    #[test]
    fn strips_case_insensitive_recursive_nmap() {
        let input = "NMAP j gj\n";
        let output = sandbox_config_text(input);
        assert!(output.contains("[sandbox] stripped recursive map"));
    }

    #[test]
    fn preserves_noremap_variants() {
        // All noremap variants with safe RHS should pass through.
        let cases = &[
            "noremap j gj\n",
            "nnoremap j gj\n",
            "vnoremap <Leader>y \"+y\n",
            "inoremap jk <Esc>\n",
            "onoremap p ip\n",
            "cnoremap <C-a> <Home>\n",
        ];
        for case in cases {
            let output = sandbox_config_text(case);
            assert!(
                !output.contains("[sandbox] stripped"),
                "noremap variant should not be stripped: {}",
                case.trim()
            );
        }
    }

    #[test]
    fn preserves_abbreviated_noremap_variants() {
        let cases = &[
            "no j gj\n",
            "nn j gj\n",
            "vn <Leader>y \"+y\n",
            "ino jk <Esc>\n",
            "ono p ip\n",
            "cno <C-a> <Home>\n",
        ];
        for case in cases {
            let output = sandbox_config_text(case);
            assert!(
                !output.contains("[sandbox] stripped"),
                "abbreviated noremap should not be stripped: {}",
                case.trim()
            );
        }
    }

    #[test]
    fn mixed_recursive_and_noremap() {
        let input = "\
nnoremap j gj
nmap k gk
set scrolloff=5
inoremap jk <Esc>
imap jj <Esc>
";
        let output = sandbox_config_text(input);
        // nnoremap and inoremap pass through
        assert!(output.contains("nnoremap j gj"));
        assert!(output.contains("inoremap jk <Esc>"));
        assert!(output.contains("set scrolloff=5"));
        // nmap and imap are stripped
        assert!(output
            .lines()
            .any(|l| l.contains("stripped recursive map") && l.contains("nmap k gk")));
        assert!(output
            .lines()
            .any(|l| l.contains("stripped recursive map") && l.contains("imap jj <Esc>")));
    }

    // --- M21: Multi-option `set` line blocking ---

    #[test]
    fn strips_multi_option_set_with_blocked_second_option() {
        // `set scrolloff=5 shell=/bin/evil` — `scrolloff` is safe but `shell` is blocked.
        let input = "set scrolloff=5 shell=/bin/evil\n";
        let output = sandbox_config_text(input);
        assert!(
            output.contains("[sandbox] stripped"),
            "multi-option set with blocked shell should be stripped, got: {}",
            output
        );
    }

    #[test]
    fn strips_multi_option_set_with_blocked_third_option() {
        let input = "set scrolloff=5 number makeprg=evil\n";
        let output = sandbox_config_text(input);
        assert!(
            output.contains("[sandbox] stripped"),
            "multi-option set with blocked makeprg should be stripped, got: {}",
            output
        );
    }

    #[test]
    fn strips_multi_option_set_blocked_first_option() {
        let input = "set shell=/bin/evil scrolloff=5\n";
        let output = sandbox_config_text(input);
        assert!(
            output.contains("[sandbox] stripped"),
            "multi-option set with blocked shell first should be stripped, got: {}",
            output
        );
    }

    #[test]
    fn preserves_multi_option_set_all_safe() {
        let input = "set scrolloff=5 number relativenumber\n";
        let output = sandbox_config_text(input);
        assert!(
            output.contains("set scrolloff=5 number relativenumber"),
            "multi-option set with all safe options should pass through, got: {}",
            output
        );
        assert!(!output.contains("[sandbox] stripped"));
    }

    #[test]
    fn strips_multi_option_set_with_short_blocked_name() {
        // `sh` is the short form of `shell`.
        let input = "set scrolloff=5 sh=/bin/evil\n";
        let output = sandbox_config_text(input);
        assert!(
            output.contains("[sandbox] stripped"),
            "multi-option set with blocked sh should be stripped, got: {}",
            output
        );
    }

    #[test]
    fn strips_multi_option_set_with_no_prefix() {
        // `set noshell` — the `no` prefix targets `shell`.
        let input = "set scrolloff=5 noshell\n";
        let output = sandbox_config_text(input);
        assert!(
            output.contains("[sandbox] stripped"),
            "multi-option set with noshell should be stripped, got: {}",
            output
        );
    }

    // --- M22: Case-insensitive `set`/`se` prefix detection ---

    #[test]
    fn strips_uppercase_set_with_blocked_option() {
        let input = "SET shell=/bin/evil\n";
        let output = sandbox_config_text(input);
        assert!(
            output.contains("[sandbox] stripped"),
            "uppercase SET with blocked option should be stripped, got: {}",
            output
        );
    }

    #[test]
    fn strips_mixed_case_set_with_blocked_option() {
        let input = "Set shell=/bin/evil\n";
        let output = sandbox_config_text(input);
        assert!(
            output.contains("[sandbox] stripped"),
            "mixed case Set with blocked option should be stripped, got: {}",
            output
        );
    }

    #[test]
    fn strips_uppercase_se_with_blocked_option() {
        let input = "SE shell=/bin/evil\n";
        let output = sandbox_config_text(input);
        assert!(
            output.contains("[sandbox] stripped"),
            "uppercase SE with blocked option should be stripped, got: {}",
            output
        );
    }

    #[test]
    fn preserves_uppercase_set_with_safe_option() {
        let input = "SET scrolloff=5\n";
        let output = sandbox_config_text(input);
        assert!(
            !output.contains("[sandbox] stripped"),
            "uppercase SET with safe option should pass through, got: {}",
            output
        );
    }

    #[test]
    fn strips_uppercase_set_multi_option_with_blocked() {
        // M21 + M22 combined: case-insensitive prefix AND multi-option check.
        let input = "SET scrolloff=5 shell=/bin/evil\n";
        let output = sandbox_config_text(input);
        assert!(
            output.contains("[sandbox] stripped"),
            "uppercase SET with multi-option blocked should be stripped, got: {}",
            output
        );
    }

    // ── :setlocal and :filetype ──────────────────────────────────────

    #[test]
    fn preserves_setlocal_with_safe_options() {
        for line in [
            "setlocal fo-=t",
            "setl fo+=cql comments=b:##,b:#",
            r"setlocal commentstring=#\ %s",
            "setlo tw=72",
            "SETLOCAL fo-=t",
        ] {
            let output = sandbox_config_text(&format!("{line}\n"));
            assert!(!output.contains("[sandbox] stripped"), "{line}: {output}");
        }
    }

    #[test]
    fn strips_setlocal_with_a_blocked_option() {
        for line in [
            "setlocal shell=/bin/evil",
            "setl fo-=t makeprg=evil",
            "setlocal keywordprg^=evil",
            "setl noshell",
        ] {
            let output = sandbox_config_text(&format!("{line}\n"));
            assert!(output.contains("[sandbox] stripped"), "{line}: {output}");
        }
    }

    #[test]
    fn strips_words_that_only_look_like_setlocal() {
        for line in ["setlocalx fo-=t", "setg shell=x", "sethandler <C-a> n:host"] {
            let output = sandbox_config_text(&format!("{line}\n"));
            assert!(output.contains("[sandbox] stripped"), "{line}: {output}");
        }
    }

    #[test]
    fn preserves_filetype_switches() {
        for line in [
            "filetype plugin off",
            "filetype plugin on",
            "filetype plugin indent on",
            "filet off",
            ":filetype plugin off",
            "filetype plugin off \" no plugins",
            "filetype off | set tw=10",
        ] {
            let output = sandbox_config_text(&format!("{line}\n"));
            assert!(!output.contains("[sandbox] stripped"), "{line}: {output}");
        }
    }

    #[test]
    fn strips_malformed_filetype_lines() {
        for line in [
            "filetype plugin off | !rm",
            "filetype plugin",
            "filetype plugin off | set shell=/bin/evil",
            "filetype plugin indent on | syntax on",
            "filetype on | filetype on | !rm",
        ] {
            let output = sandbox_config_text(&format!("{line}\n"));
            assert!(output.contains("[sandbox] stripped"), "{line}: {output}");
        }
    }

    // ── Several commands on one line ─────────────────────────────────

    #[test]
    fn strips_a_line_whose_later_bar_command_is_unsafe() {
        // vim-core runs every `|` command of a config line; these passed
        // because the line starts with a safe command.
        for line in [
            "set so=5 | nnoremap x :!id<CR>",
            "set so=5 | nmap x :action foo<CR>",
            "let mapleader=\",\" | nnoremap x :!id<CR>",
            "nnoremap x y | !id",
            "set so=5 \" comment | !id",
            "set so=5 | set sw=4 | so x.vim",
        ] {
            let output = sandbox_config_text(&format!("{line}\n"));
            assert_eq!(output, format!("\" [sandbox] stripped: {line}\n"));
        }
    }

    #[test]
    fn preserves_a_line_whose_bar_commands_are_all_safe() {
        for line in [
            "set so=5 | set sw=4",
            "let mapleader=\",\" | set so=5",
            "filetype plugin on | set et",
            "nnoremap x :echo 1 \\| echo 2<CR>",
            "\" a comment | !id",
        ] {
            let output = sandbox_config_text(&format!("{line}\n"));
            assert_eq!(output, format!("{line}\n"));
        }
    }

    #[test]
    fn a_stripped_bar_line_defines_nothing_in_the_engine() {
        // Once vim-core runs a config line as a command line, the commands
        // after a `|` run too.
        for line in [
            "set so=5 | nnoremap x :!id<CR>",
            "set so=5 | nmap x :action foo<CR>",
            "let mapleader=\",\" | nnoremap x :!id<CR>",
        ] {
            let mut engine = vim_core::execution::VimEngine::new();
            let _ = engine.source_config_text(&sandbox_config_text(&format!("{line}\n")));
            let x = vim_core::keymap::KeyEvent::char('x');
            assert!(!engine.could_start_mapping(x), "{line}");
        }
    }

    // ── The blocklist's delimiter split ──────────────────────────────

    #[test]
    fn every_set_delimiter_normalises_to_the_bare_option_name() {
        // `extract_option_name_from_token` is the last line of defence in front
        // of BLOCKED_SET_OPTIONS: the blocklist is matched against its OUTPUT,
        // so a delimiter it does not split on is an option name it does not
        // recognise. Narrowing the split set to `['=']` alone whitelists
        // `set shell!` from a committed project vimrc.
        //
        // One token per delimiter Vim accepts, plus the `no` prefix.
        for token in [
            "shell=x", "shell?", "shell!", "shell:x", "shell+=x", "shell-=x", "shell^=x", "noshell",
        ] {
            assert_eq!(
                extract_option_name_from_token(token),
                "shell",
                "'{token}' must normalise to the bare option name"
            );
        }
    }

    #[test]
    fn a_blocked_option_is_stripped_through_every_delimiter() {
        // The same split, end to end: each of these reaches
        // `BLOCKED_SET_OPTIONS` only if the delimiter was stripped first.
        for line in ["set shell!\n", "set kp?\n", "set ep+=x\n", "set noshell\n"] {
            let out = sandboxed(line);
            assert!(
                out.contains("[sandbox] stripped"),
                "'{}' must not reach the engine: {out}",
                line.trim()
            );
        }
    }

    /// `set langmap=q:` makes `q` type `:`, so `nnoremap x 0q!cmd<CR>` runs
    /// `:!cmd` with no `:` in its right-hand side.
    #[test]
    fn strips_langmap_lines() {
        assert_all_stripped(&[
            "set langmap=q:",
            "se lmap=q:",
            "setlocal langmap+=q:",
            "set so=5 langremap",
            "set lrm",
            "set nolangremap",
            "set langnoremap",
            "set nolnr",
            "set so=5 | set langmap=q:",
        ]);
    }

    #[test]
    fn a_stripped_langmap_line_leaves_the_engine_langmap_empty() {
        use vim_core::execution::VimEngine;
        let text = "set langmap=q:\nnnoremap x 0q!echo hi<CR>\n";
        let mut trusted = VimEngine::new();
        let _ = trusted.source_config_text(text);
        assert_eq!(trusted.options().langmap(), "q:");
        let mut engine = VimEngine::new();
        let _ = engine.source_config_text(&sandbox_config_text(text));
        assert_eq!(engine.options().langmap(), "");
    }

    // ── panelmap trust tiers (P7) ────────────────────────────────────

    fn sandboxed(text: &str) -> String {
        apply_vimrc_policy(text, true, crate::settings::ProjectVimrc::Sandbox)
            .unwrap_or_default()
            .text
    }

    /// Assert that `line` came through **live**, not commented out.
    ///
    /// Whole-line, and that is the entire point. A stripped line is emitted as
    /// `" [sandbox] stripped: <trimmed>` — which *contains* the original line
    /// verbatim — so `out.contains("panelmap …")` holds whether the binding was
    /// honoured or stripped, and every such assertion passed while
    /// `panel_line_is_safe` returned `false` for all three of its arms.
    /// Comparing the trimmed line for equality is what makes the leading `" `
    /// visible to the test. The negative twin is belt and braces: a future
    /// output shape that both keeps the line and reports a strip would still
    /// fail here.
    fn assert_survives(out: &str, line: &str) {
        assert!(
            out.lines().any(|l| l.trim() == line),
            "'{line}' must survive as a live line: {out}"
        );
        assert!(
            !out.contains("[sandbox] stripped"),
            "'{line}' must not be stripped: {out}"
        );
    }

    #[test]
    fn an_action_binding_survives_a_project_vimrc() {
        // The RHS is a closed vocabulary — a registered action id plus
        // integer-only params — so it cannot expand into `:!`, `:source`, or
        // a recursive chain. That is the entire justification for letting it
        // through where recursive maps are stripped unconditionally.
        let out = sandboxed("panelmap dock n godotvim.item.next\n");
        assert_survives(&out, "panelmap dock n godotvim.item.next");
    }

    #[test]
    fn a_give_back_survives_at_every_tier() {
        // `native` can only REDUCE what the plugin consumes, so a hostile
        // project can at most hand keys back to Godot.
        let out = sandboxed("panelmap dock j native\n");
        assert_survives(&out, "panelmap dock j native");
    }

    #[test]
    fn an_unmap_survives_at_every_tier() {
        let out = sandboxed("panelunmap dock j\n");
        assert_survives(&out, "panelunmap dock j");
    }

    #[test]
    fn a_shortcut_target_is_stripped_from_a_project_vimrc() {
        // `<Shortcut>(path)` invokes an arbitrary Godot editor command by
        // name — an open vocabulary, outside anything this plugin can reason
        // about. Denied at project tier.
        let out = sandboxed("panelmap dock d <Shortcut>(filesystem_dock/delete)\n");
        // Stripped means commented out with a reason, not deleted — the user
        // can see what happened and why, which a silent drop would deny them.
        assert!(out.contains("[sandbox] stripped"), "must say why: {out}");
        assert!(
            out.lines().all(|l| !l.starts_with("panelmap")),
            "no live panelmap line may survive: {out}"
        );
    }

    #[test]
    fn an_unparseable_panel_line_is_stripped_not_trusted() {
        // "does not parse" and "is harmless" are different claims: this is the
        // `Err(_) => false` arm of `panel_line_is_safe`, and flipping it to
        // `true` passes an unparseable line through verbatim.
        //
        // The old needle here was `"\npanelmap dock\n"` — a leading newline the
        // single-line input can never produce, so the assertion held no matter
        // what the sandbox did.
        let out = sandboxed("panelmap dock\n");
        assert!(out.contains("[sandbox] stripped"), "must say why: {out}");
        assert!(
            out.lines().all(|l| !l.starts_with("panelmap")),
            "no live panelmap line may survive: {out}"
        );
    }

    #[test]
    fn a_binding_on_an_editor_surface_is_stripped_from_a_project_vimrc() {
        // The overlay is the first rule-bearing surface live while the vim
        // engine owns the keyboard, and a rule there that accepts consumes
        // the key. A committed vimrc binding Escape to a completion verb
        // would make Insert mode inescapable, which breaks the invariant that
        // a hostile project can at worst produce a dead key.
        let out = sandboxed("panelmap editor.completion <Esc> godotvim.completion.trigger\n");
        assert!(out.contains("[sandbox] stripped"), "must say why: {out}");
        assert!(
            out.lines().all(|l| !l.starts_with("panelmap")),
            "no live panelmap line may survive: {out}"
        );
    }

    #[test]
    fn a_binding_on_a_panel_surface_still_survives() {
        // The guard on the guard: the strip is scoped to `editor.*`, not to
        // panel lines at large.
        let out = sandboxed("panelmap dock n godotvim.item.next\n");
        assert_survives(&out, "panelmap dock n godotvim.item.next");
    }

    #[test]
    fn an_unmap_on_an_editor_surface_still_survives() {
        // Removing a binding can only reduce what the plugin consumes.
        let out = sandboxed("panelunmap editor.completion <Tab>\n");
        assert_survives(&out, "panelunmap editor.completion <Tab>");
    }

    /// A comment that already reads like a stripped line is not counted.
    #[test]
    fn the_policy_counts_the_lines_it_disables() {
        use crate::settings::ProjectVimrc;
        let text = "nnoremap j gj\nnmap Q iBAD<Esc>\nnnoremap K :!ls<CR>\nset tw=10\n\
                    set so=5 | !ls\n\" [sandbox] stripped: nmap Q x\n";
        let sandboxed = apply_vimrc_policy(text, true, ProjectVimrc::Sandbox).unwrap();
        assert_eq!(sandboxed.stripped, 3, "{}", sandboxed.text);
        for (is_project_level, policy) in [
            (true, ProjectVimrc::Trusted),
            (false, ProjectVimrc::Sandbox),
            (false, ProjectVimrc::Disabled),
        ] {
            let out = apply_vimrc_policy(text, is_project_level, policy).unwrap();
            assert_eq!((out.text.as_str(), out.stripped), (text, 0));
        }
        assert!(apply_vimrc_policy(text, true, ProjectVimrc::Disabled).is_none());
    }

    #[test]
    fn a_trusted_vimrc_keeps_editor_surface_bindings() {
        let out = apply_vimrc_policy(
            "panelmap editor.completion <Esc> godotvim.completion.dismiss\n",
            true,
            crate::settings::ProjectVimrc::Trusted,
        )
        .unwrap_or_default()
        .text;
        assert!(out.contains("editor.completion <Esc>"), "{out}");
        assert!(!out.contains("[sandbox] stripped"), "{out}");
    }

    #[test]
    fn a_trusted_vimrc_keeps_shortcut_targets() {
        let out = apply_vimrc_policy(
            "panelmap dock d <Shortcut>(filesystem_dock/delete)\n",
            true,
            crate::settings::ProjectVimrc::Trusted,
        )
        .unwrap_or_default()
        .text;
        assert!(out.contains("<Shortcut>"), "{out}");
    }

    #[test]
    fn a_user_level_vimrc_is_never_sandboxed() {
        // user:// is outside any repository, so it is the user's own machine.
        let out = apply_vimrc_policy(
            "panelmap dock d <Shortcut>(filesystem_dock/delete)\n",
            false,
            crate::settings::ProjectVimrc::Sandbox,
        )
        .unwrap_or_default()
        .text;
        assert!(out.contains("<Shortcut>"), "{out}");
    }

    #[test]
    fn the_security_model_is_not_reachable_from_a_vimrc_at_all() {
        // A `set` line that turned off the Foreign hard stop from a cloned
        // project would be a real hole. It is closed by construction rather
        // than by a denylist: no shipped option names a dispatcher guard, so
        // there is nothing for a project vimrc to set. This test fails the
        // day someone adds one.
        let guard_words = ["navsafety", "panelsafety", "foreignstop", "barrier"];
        let settings_src = include_str!("../settings/keys.rs");
        for word in guard_words {
            assert!(
                !settings_src.contains(word),
                "'{word}' became a setting — it must live in EditorSettings, \
                 which res:// cannot reach, not in a vimrc-settable option"
            );
        }
    }
}
