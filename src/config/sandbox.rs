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
/// Format: `(min_prefix, full_command, required_suffix)`.
const EX_COMMAND_PATTERNS: &[(&str, &str, &str)] = &[
    // `:!` — shell execution (no abbreviation, just literal)
    ("!", "!", ""),
    // `:read !` — shell via read (abbreviation: r[ead])
    ("r", "read", " !"),
    // `:source ` — config chain-loading (abbreviation: so[urce])
    ("so", "source", " "),
];

/// Filter config text for sandbox mode using a **whitelist** approach.
///
/// Only known-safe constructs pass through (comments, blanks, safe `:set`,
/// `:let mapleader`, non-recursive mappings with clean RHS). Everything else
/// -- including raw ex-commands like `:source` or `:!` -- is replaced with
/// a diagnostic comment. This prevents a malicious `res://.godot-vimrc`
/// from executing arbitrary commands.
pub(crate) fn sandbox_config_text(text: &str) -> String {
    let mut output = String::with_capacity(text.len());
    for line in text.lines() {
        let trimmed = line.trim();
        if is_safe_non_mapping_line(trimmed) {
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
                output.push_str(trimmed);
                output.push('\n');
            } else if contains_shell_pattern(trimmed) {
                log::warn!(
                    "sandbox: stripped dangerous mapping from project vimrc: {}",
                    trimmed
                );
                output.push_str("\" [sandbox] stripped: ");
                output.push_str(trimmed);
                output.push('\n');
            } else {
                output.push_str(line);
                output.push('\n');
            }
        } else {
            log::warn!(
                "sandbox: stripped unrecognized line from project vimrc: {}",
                trimmed
            );
            output.push_str("\" [sandbox] stripped: ");
            output.push_str(trimmed);
            output.push('\n');
        }
    }
    output
}

/// Options that can execute shell commands or exfiltrate data.
/// Both long and short forms must be listed (Vim accepts either).
const BLOCKED_SET_OPTIONS: &[&str] = &[
    "shell", "sh",
    "shellcmdflag", "shcf",
    "shellpipe", "sp",
    "shellredir", "srr",
    "shellquote", "shq",
    "shellxquote", "sxq",
    "shellxescape", "sxe",
    "makeprg", "mp",
    "grepprg", "gp",
    "equalprg", "ep",
    "formatprg", "fp",
    "keywordprg", "kp",
];

/// Extract the option name from a `set` token, stripping value delimiters
/// (`=`, `?`, `!`, etc.) and the `no` prefix (`noshell` -> `shell`).
fn extract_option_name_from_token(token: &str) -> &str {
    let name = token
        .split(['=', '?', '!', '+', '-', ':'])
        .next()
        .unwrap_or("");
    name.strip_prefix("no").unwrap_or(name)
}

/// Whitelist check for non-mapping lines that are safe to pass through.
fn is_safe_non_mapping_line(trimmed: &str) -> bool {
    if trimmed.is_empty() {
        return true;
    }
    if trimmed.starts_with('"') {
        return true;
    }
    // Case-insensitive: the engine may accept `Set`, `SET`, etc.
    let trimmed_lower = trimmed.to_ascii_lowercase();
    let after_set = trimmed_lower.strip_prefix("set ")
        .or_else(|| trimmed_lower.strip_prefix("se "));
    if let Some(options_str) = after_set {
        // Check ALL space-separated options -- `set scrolloff=5 shell=/bin/evil`
        // must be blocked because `shell` is dangerous even if `scrolloff` is safe.
        let has_blocked = options_str.split_whitespace().any(|token| {
            let name = extract_option_name_from_token(token);
            BLOCKED_SET_OPTIONS.iter().any(|blocked| name.eq_ignore_ascii_case(blocked))
        });
        return !has_blocked;
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
    if name.eq_ignore_ascii_case("map") { return true; }
    if matches_abbrev(name, "nm", "nmap") { return true; }
    if matches_abbrev(name, "vm", "vmap") { return true; }
    if matches_abbrev(name, "im", "imap") { return true; }
    if matches_abbrev(name, "om", "omap") { return true; }
    if matches_abbrev(name, "cm", "cmap") { return true; }

    false
}

/// Matches any map/noremap/unmap abbreviation. Must stay in sync with the
/// engine's `matches_abbrev` logic to prevent sandbox bypasses.
fn is_map_or_noremap_abbrev(name: &str) -> bool {
    if name.eq_ignore_ascii_case("map") { return true; }
    if matches_abbrev(name, "nm", "nmap") { return true; }
    if matches_abbrev(name, "vm", "vmap") { return true; }
    if matches_abbrev(name, "im", "imap") { return true; }
    if matches_abbrev(name, "om", "omap") { return true; }
    if matches_abbrev(name, "cm", "cmap") { return true; }

    if matches_abbrev(name, "no", "noremap") { return true; }
    if matches_abbrev(name, "nn", "nnoremap") { return true; }
    if matches_abbrev(name, "vn", "vnoremap") { return true; }
    if matches_abbrev(name, "ino", "inoremap") { return true; }
    if matches_abbrev(name, "ono", "onoremap") { return true; }
    if matches_abbrev(name, "cno", "cnoremap") { return true; }

    // Unmap could remove safety bindings the user relies on.
    if matches_abbrev(name, "unm", "unmap") { return true; }
    if matches_abbrev(name, "nun", "nunmap") { return true; }
    if matches_abbrev(name, "vu", "vunmap") { return true; }
    if matches_abbrev(name, "iu", "iunmap") { return true; }
    if matches_abbrev(name, "ou", "ounmap") { return true; }
    if matches_abbrev(name, "cu", "cunmap") { return true; }

    false
}

/// Scan a mapping line for dangerous patterns: case-sensitive function names
/// (Vim functions are case-sensitive) and abbreviation-aware, case-insensitive
/// ex-commands after each `:`.
fn contains_shell_pattern(line: &str) -> bool {
    if FUNCTION_PATTERNS.iter().any(|pat| line.contains(pat)) {
        return true;
    }
    for (i, _) in line.match_indices(':') {
        let after_colon = &line[i + 1..];
        // Skip range chars so `:%!sort` and `:'<,'>!cmd` are caught.
        let after_range = after_colon.trim_start_matches(|c: char| {
            c.is_ascii_digit() || matches!(c, '%' | '$' | '.' | '\'' | ',' | '+' | '-' | '<' | '>' | ' ')
        });
        for &(min, full, suffix) in EX_COMMAND_PATTERNS {
            if matches_ex_pattern(after_range, min, full, suffix) {
                return true;
            }
        }
    }
    false
}

fn matches_ex_pattern(text: &str, min: &str, full: &str, suffix: &str) -> bool {
    for len in min.len()..=full.len() {
        if text.len() < len + suffix.len() {
            continue;
        }
        // .get() avoids panic on multi-byte UTF-8 character boundaries.
        let Some(prefix) = text.get(..len) else { continue };
        if prefix.eq_ignore_ascii_case(&full[..len])
            && text[len..].starts_with(suffix)
        {
            return true;
        }
    }
    false
}

/// Apply the project vimrc security policy. Returns `None` to block loading
/// entirely (Disabled), or `Some(text)` with sanitized content (Sandbox)
/// or unchanged text (Trusted / user-level).
pub(crate) fn apply_vimrc_policy(
    text: &str,
    is_project_level: bool,
    policy: crate::settings::ProjectVimrc,
) -> Option<String> {
    if !is_project_level {
        return Some(text.to_string());
    }
    match policy {
        crate::settings::ProjectVimrc::Disabled => {
            log::info!(
                "project vimrc skipped: security/project_vimrc is set to Disabled"
            );
            None
        }
        crate::settings::ProjectVimrc::Sandbox => {
            log::info!("project vimrc loaded in sandbox mode");
            Some(sandbox_config_text(text))
        }
        crate::settings::ProjectVimrc::Trusted => {
            Some(text.to_string())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert!(!output.lines().any(|l| l.trim().starts_with("nnoremap <Leader>r :!cargo")));
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

    #[test]
    fn strips_bang_suffix_map_command() {
        let input = "nnoremap! <Leader>x :!rm -rf /<CR>\n";
        let output = sandbox_config_text(input);
        assert!(output.contains("[sandbox] stripped"));
    }

    #[test]
    fn strips_abbreviated_bang_suffix() {
        let input = "nn! <Leader>x :!rm -rf /<CR>\n";
        let output = sandbox_config_text(input);
        assert!(output.contains("[sandbox] stripped"));
    }

    #[test]
    fn strips_map_bang_suffix() {
        let input = "map! <Leader>x :!rm -rf /<CR>\n";
        let output = sandbox_config_text(input);
        assert!(output.contains("[sandbox] stripped"));
    }

    #[test]
    fn strips_case_insensitive_map_command() {
        let input = "NNOREMAP <Leader>x :!rm -rf /<CR>\n";
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
        let input = "cnoremap <Leader>x :!rm -rf /<CR>\n";
        let output = sandbox_config_text(input);
        assert!(output.contains("[sandbox] stripped"));
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
        assert!(output.contains("[sandbox] stripped"), "mapleader_hack should be stripped");
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
            assert!(output.contains("[sandbox] stripped"), "failed for :{}", abbrev);
        }
    }

    #[test]
    fn strips_read_intermediate_abbreviations() {
        for abbrev in &["r", "re", "rea", "read"] {
            let input = format!("nnoremap <Leader>r :{} !cat /etc/passwd<CR>\n", abbrev);
            let output = sandbox_config_text(&input);
            assert!(output.contains("[sandbox] stripped"), "failed for :{}", abbrev);
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
            assert!(output.contains("[sandbox] stripped"), "failed for: {}", case.trim());
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
            assert!(output.contains("[sandbox] stripped"), "failed for: {}", case.trim());
        }
    }

    #[test]
    fn no_panic_on_multibyte_after_colon() {
        // Should not panic on multi-byte UTF-8 after `:` — just pass through.
        let input = "nnoremap <Leader>x :ñfoo<CR>\n";
        let output = sandbox_config_text(input);
        assert!(output.contains("nnoremap")); // safe mapping, not stripped
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
        assert!(output.lines().any(|l| l.contains("stripped recursive map") && l.contains("nmap k gk")));
        assert!(output.lines().any(|l| l.contains("stripped recursive map") && l.contains("imap jj <Esc>")));
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
}
