//! Shell execution and reindentation for `:!command`, `:{range}!command`, and
//! `={motion}`.
//!
//! Shell commands are executed via Godot's `Os::execute()` rather than Rust's
//! `std::process::Command` to ensure correct behavior in the Godot editor
//! process context (environment, working directory, platform detection).

use std::sync::atomic::{AtomicU32, Ordering};

use compact_str::CompactString;
use godot::classes::{CodeEdit, Os};
use godot::prelude::*;
use vim_core::execution::{HostRequestId, HostResult};
use vim_core::primitives::Range;

use crate::bridge::code_edit_ext::CodeEditExt;

use super::host_failure;

static FILTER_COUNTER: AtomicU32 = AtomicU32::new(0);

struct TempFileGuard {
    path: std::path::PathBuf,
}

impl Drop for TempFileGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

/// Runtime OS detection via Godot rather than `cfg!(target_os)`, because Godot
/// plugins may be cross-compiled for a different target than the build host.
fn is_windows() -> bool {
    Os::singleton().get_name().to_string() == "Windows"
}

fn platform_shell() -> (&'static str, &'static str) {
    if is_windows() {
        ("cmd.exe", "/c")
    } else {
        ("/bin/sh", "-c")
    }
}

/// Execute a shell command and return (exit_code, captured_output).
///
/// `capture_stderr`: `true` merges stderr into stdout (for `:!cmd` display);
/// `false` discards stderr (for `:{range}!cmd` filters where stderr would
/// corrupt the replacement text).
fn run_shell_command(cmd: &str, capture_stderr: bool) -> (i32, String) {
    let (shell, flag) = platform_shell();
    log::debug!("shell: executing via {} {}: {}", shell, flag, cmd);
    let args = PackedStringArray::from(&[
        GString::from(flag),
        GString::from(cmd),
    ]);
    let output_array = Array::<Variant>::new();

    let exit_code = Os::singleton()
        .execute_ex(&GString::from(shell), &args)
        .output(&output_array)
        .read_stderr(capture_stderr)
        .done();

    let output_text = output_array
        .get(0)
        .and_then(|v: Variant| v.try_to::<GString>().ok())
        .map(|s: GString| s.to_string())
        .unwrap_or_default();

    log::debug!("shell: exit_code={} output_len={}", exit_code, output_text.len());
    (exit_code, output_text)
}

/// Write filter input to a temp file for piping to the shell command.
///
/// Security: `create_new(true)` (O_EXCL) prevents symlink attacks — fails if
/// the path already exists. On Unix, permissions are restricted to 0o600.
/// The PID + atomic counter in the filename ensures uniqueness without races.
fn write_temp_input(text: &str) -> Option<String> {
    use std::io::Write;

    let dir = std::env::temp_dir();
    let path = dir.join(format!(
        "godot_vim_filter_{}_{}.tmp",
        std::process::id(),
        FILTER_COUNTER.fetch_add(1, Ordering::Relaxed),
    ));

    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create_new(true);

    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }

    let mut file = opts.open(&path).map_err(|e| {
        log::warn!("temp file creation failed: {e}");
        e
    }).ok()?;
    file.write_all(text.as_bytes()).map_err(|e| {
        log::warn!("Failed to write temp file {}: {}", path.display(), e);
        // Clean up the empty file we already created
        let _ = std::fs::remove_file(&path);
        e
    }).ok()?;

    Some(path.to_string_lossy().into_owned())
}

fn shell_escape_single_quotes(s: &str) -> String {
    s.replace('\'', "'\\''")
}

/// `:!command` — run a shell command, capturing both stdout and stderr.
pub(super) fn handle_external_command(
    id: HostRequestId,
    command: &str,
) -> HostResult {
    let (exit_code, output) = run_shell_command(command, true);

    if exit_code != 0 {
        log::warn!(
            "External command exited with code {exit_code}: {command}"
        );
    }

    HostResult::Data {
        id,
        data: CompactString::from(output),
        offset: None,
    }
}

/// `:{range}!command` — pipe document text through an external filter.
///
/// Input is written to a temp file (avoiding shell escaping issues with
/// multi-line text) and piped via `cat`/`type` to the filter command. Stderr
/// is discarded so it cannot corrupt the replacement text.
pub(super) fn handle_filter(
    id: HostRequestId,
    input_text: &str,
    command: &str,
) -> HostResult {
    let windows = is_windows();
    let temp_result = write_temp_input(input_text);
    let (exit_code, output) = match temp_result {
        Some(temp_path) => {
            let _guard = TempFileGuard { path: std::path::PathBuf::from(&temp_path) };
            let full_cmd = if windows {
                format!("type \"{temp_path}\" | {command}")
            } else {
                format!("cat '{}' | {command}", shell_escape_single_quotes(&temp_path))
            };
            run_shell_command(&full_cmd, false)
        }
        None => {
            return host_failure(id, "E484: Cannot create temporary file for filter command");
        }
    };

    let stderr = if exit_code != 0 {
        log::warn!(
            "Filter command exited with code {exit_code}: {command}"
        );
        Some(CompactString::from(format!(
            "shell returned {exit_code}"
        )))
    } else {
        None
    };

    HostResult::FilteredRange {
        id,
        replacement: CompactString::from(output),
        cursor_offset: None,
        stderr,
        mark_dot_offset: None,
    }
}

/// `={motion}` — auto-reindent using Godot's indent settings.
///
/// Adapter between Godot and the pure `reindent_lines` function: fetches indent
/// config (spaces vs tabs, indent size) and the reference line preceding the
/// range from the live CodeEdit, then delegates all logic to the pure function.
pub(super) fn handle_reindent(
    id: HostRequestId,
    editor: &mut Gd<CodeEdit>,
    input_text: &str,
    range: &Range,
) -> HostResult {
    let lines_count = input_text.split('\n').count();
    log::debug!(
        "reindent: range={}..{} lines={}",
        range.start().get(), range.end().get(), lines_count
    );

    // Convert the engine's byte range to a Godot line number.
    let full_text = editor.get_text().to_string();
    let mut start_byte = range.start().get().min(full_text.len());
    // Snap to UTF-8 char boundary to avoid panic on multi-byte characters.
    while start_byte > 0 && !full_text.is_char_boundary(start_byte) {
        start_byte -= 1;
    }
    let start_line = crate::bridge::codec::usize_to_i32(full_text[..start_byte].matches('\n').count());

    let use_spaces = editor.is_indent_using_spaces();
    let indent_size = editor.safe_indent_size();
    let one_indent = if use_spaces {
        " ".repeat(indent_size)
    } else {
        "\t".to_string()
    };

    let ref_line_before_range = if start_line > 0 {
        Some(editor.get_line(start_line - 1).to_string())
    } else {
        None
    };

    let lines: Vec<&str> = input_text.split('\n').map(|l| l.trim_end_matches('\r')).collect();
    let result_lines = reindent_lines(&lines, &one_indent, ref_line_before_range.as_deref());
    let replacement = result_lines.join("\n");

    HostResult::FilteredRange {
        id,
        replacement: CompactString::from(replacement),
        cursor_offset: None,
        stderr: None,
        mark_dot_offset: None,
    }
}

/// GDScript-aware reindentation (pure function, no Godot dependency).
///
/// Heuristics: block-opening characters (`:`, `{`, `(`, `[`) increase indent;
/// closing characters (`}`, `)`, `]`) and outdent keywords (`elif`, `else`,
/// `except`, `finally`) decrease indent. `ref_line_before_range` provides
/// context for the first line's indent level (`None` = document start = level 0).
/// Empty input lines produce empty output lines (no trailing whitespace).
fn reindent_lines(
    lines: &[&str],
    one_indent: &str,
    ref_line_before_range: Option<&str>,
) -> Vec<String> {
    let mut result_lines: Vec<String> = Vec::with_capacity(lines.len());

    for (i, line) in lines.iter().enumerate() {
        let trimmed_content = line.trim_start();
        if trimmed_content.is_empty() {
            result_lines.push(String::new());
            continue;
        }

        let base_indent: std::borrow::Cow<'_, str> = if i == 0 {
            match ref_line_before_range {
                Some(ref_text) => compute_indent_from_ref(ref_text, one_indent),
                None => std::borrow::Cow::Borrowed(""),
            }
        } else {
            // Use the last non-empty reindented line as reference.
            let ref_text = result_lines[..i]
                .iter()
                .rev()
                .find(|l| !l.trim().is_empty());
            match ref_text {
                Some(ref_text) => compute_indent_from_ref(ref_text, one_indent),
                // All preceding lines are empty — fall back to pre-range reference.
                None => match ref_line_before_range {
                    Some(ref_text) => compute_indent_from_ref(ref_text, one_indent),
                    None => std::borrow::Cow::Borrowed(""),
                },
            }
        };

        let first_non_ws = trimmed_content.chars().next();
        let indent = if matches!(first_non_ws, Some('}' | ')' | ']'))
            || starts_with_outdent_keyword(trimmed_content)
        {
            strip_one_indent(&base_indent, one_indent)
        } else {
            base_indent
        };

        result_lines.push(format!("{indent}{trimmed_content}"));
    }

    result_lines
}

/// Derive indent for a new line from the reference (previous) line: inherits
/// the reference's leading whitespace, adding one level if it ends with a
/// block opener (`:`, `{`, `(`, `[`).
fn compute_indent_from_ref<'a>(ref_line: &'a str, one_indent: &str) -> std::borrow::Cow<'a, str> {
    // Indent chars are always ASCII, so byte iteration is safe here.
    let indent_len = ref_line.bytes().take_while(|&b| b == b' ' || b == b'\t').count();
    let base = &ref_line[..indent_len];
    let trimmed = ref_line.trim_end();
    let opens_block = trimmed.ends_with(':')
        || trimmed.ends_with('{')
        || trimmed.ends_with('(')
        || trimmed.ends_with('[');

    if opens_block {
        std::borrow::Cow::Owned(format!("{base}{one_indent}"))
    } else {
        std::borrow::Cow::Borrowed(base)
    }
}

fn strip_one_indent<'a>(indent: &'a str, one_indent: &str) -> std::borrow::Cow<'a, str> {
    if let Some(stripped) = indent.strip_suffix('\t') {
        std::borrow::Cow::Borrowed(stripped)
    } else if let Some(stripped) = indent.strip_suffix(one_indent) {
        std::borrow::Cow::Borrowed(stripped)
    } else {
        std::borrow::Cow::Borrowed(indent)
    }
}

/// GDScript outdent keywords (`elif`, `else`, `except`, `finally`) sit at the
/// same indent as their opening statement, not at body level. Word boundary
/// enforcement prevents false positives on identifiers like `elsewhere`.
fn starts_with_outdent_keyword(trimmed: &str) -> bool {
    const KEYWORDS: &[&str] = &["elif", "else", "except", "finally"];
    for kw in KEYWORDS {
        if let Some(rest) = trimmed.strip_prefix(kw) {
            // Keyword must be at word boundary: followed by `:`, whitespace,
            // `(`, or end of line.
            if rest.is_empty() || rest.starts_with(|c: char| !c.is_alphanumeric() && c != '_') {
                return true;
            }
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_single_quotes_unchanged() {
        assert_eq!(shell_escape_single_quotes("hello world"), "hello world");
    }

    #[test]
    fn empty_string_unchanged() {
        assert_eq!(shell_escape_single_quotes(""), "");
    }

    #[test]
    fn only_ascii_letters() {
        assert_eq!(shell_escape_single_quotes("abcXYZ"), "abcXYZ");
    }

    #[test]
    fn double_quotes_not_escaped() {
        assert_eq!(
            shell_escape_single_quotes(r#"hello "world""#),
            r#"hello "world""#
        );
    }

    #[test]
    fn backslashes_not_escaped() {
        assert_eq!(
            shell_escape_single_quotes(r"path\to\file"),
            r"path\to\file"
        );
    }

    #[test]
    fn single_quote_at_start() {
        assert_eq!(shell_escape_single_quotes("'hello"), "'\\''" .to_owned() + "hello");
        assert_eq!(shell_escape_single_quotes("'hello"), "'\\''hello");
    }

    #[test]
    fn single_quote_at_end() {
        assert_eq!(shell_escape_single_quotes("hello'"), "hello'\\''");
    }

    #[test]
    fn single_quote_in_middle() {
        assert_eq!(shell_escape_single_quotes("it's"), "it'\\''s");
    }

    #[test]
    fn multiple_single_quotes() {
        assert_eq!(
            shell_escape_single_quotes("it's a 'test'"),
            "it'\\''s a '\\''test'\\''"
        );
    }

    #[test]
    fn consecutive_single_quotes() {
        assert_eq!(shell_escape_single_quotes("''"), "'\\'''\\''" );
    }

    #[test]
    fn only_single_quote() {
        assert_eq!(shell_escape_single_quotes("'"), "'\\''");
    }

    #[test]
    fn multiline_no_quotes() {
        let input = "line one\nline two\nline three";
        assert_eq!(shell_escape_single_quotes(input), input);
    }

    #[test]
    fn multiline_with_quotes() {
        let input = "it's\na 'test'";
        let expected = "it'\\''s\na '\\''test'\\''";
        assert_eq!(shell_escape_single_quotes(input), expected);
    }

    #[test]
    fn unicode_without_quotes() {
        assert_eq!(shell_escape_single_quotes("日本語"), "日本語");
    }

    #[test]
    fn unicode_with_single_quote() {
        assert_eq!(shell_escape_single_quotes("日'語"), "日'\\''語");
    }

    #[test]
    fn dollar_sign_unchanged() {
        assert_eq!(shell_escape_single_quotes("$HOME"), "$HOME");
    }

    #[test]
    fn backticks_unchanged() {
        assert_eq!(shell_escape_single_quotes("`cmd`"), "`cmd`");
    }

    #[test]
    fn exclamation_mark_unchanged() {
        assert_eq!(shell_escape_single_quotes("hello!"), "hello!");
    }

    #[test]
    fn tabs_and_spaces_unchanged() {
        assert_eq!(shell_escape_single_quotes("a\tb  c"), "a\tb  c");
    }

    // ---------------------------------------------------------------
    // compute_indent_from_ref
    // ---------------------------------------------------------------

    #[test]
    fn compute_indent_no_opener_no_indent() {
        assert_eq!(compute_indent_from_ref("let x = 5", "    "), "");
    }

    #[test]
    fn compute_indent_no_opener_with_indent() {
        assert_eq!(compute_indent_from_ref("    let x = 5", "    "), "    ");
    }

    #[test]
    fn compute_indent_no_opener_tab_indent() {
        assert_eq!(compute_indent_from_ref("\tlet x = 5", "\t"), "\t");
    }

    // --- Colon opener (GDScript if/for/while/func) ---

    #[test]
    fn compute_indent_colon_opener() {
        assert_eq!(compute_indent_from_ref("if x:", "    "), "    ");
    }

    #[test]
    fn compute_indent_colon_opener_already_indented() {
        assert_eq!(compute_indent_from_ref("    if x:", "    "), "        ");
    }

    #[test]
    fn compute_indent_colon_opener_tab() {
        assert_eq!(compute_indent_from_ref("\tif x:", "\t"), "\t\t");
    }

    // --- Brace opener ---

    #[test]
    fn compute_indent_brace_opener() {
        assert_eq!(compute_indent_from_ref("fn main() {", "    "), "    ");
    }

    #[test]
    fn compute_indent_brace_opener_indented() {
        assert_eq!(compute_indent_from_ref("    fn main() {", "    "), "        ");
    }

    // --- Paren opener ---

    #[test]
    fn compute_indent_paren_opener() {
        assert_eq!(compute_indent_from_ref("call(", "    "), "    ");
    }

    // --- Bracket opener ---

    #[test]
    fn compute_indent_bracket_opener() {
        assert_eq!(compute_indent_from_ref("arr = [", "    "), "    ");
    }

    // --- Mixed indent ---

    #[test]
    fn compute_indent_mixed_tab_space_indent() {
        assert_eq!(compute_indent_from_ref("\t  let x = 5", "    "), "\t  ");
    }

    // --- Empty reference line ---

    #[test]
    fn compute_indent_empty_ref() {
        assert_eq!(compute_indent_from_ref("", "    "), "");
    }

    // --- Whitespace-only reference line ---

    #[test]
    fn compute_indent_whitespace_only_ref() {
        assert_eq!(compute_indent_from_ref("    ", "    "), "    ");
    }

    // --- Trailing whitespace after opener ---

    #[test]
    fn compute_indent_opener_with_trailing_space() {
        assert_eq!(compute_indent_from_ref("if x:  ", "    "), "    ");
    }

    // ---------------------------------------------------------------
    // strip_one_indent
    // ---------------------------------------------------------------

    #[test]
    fn strip_one_indent_tab() {
        let result = strip_one_indent("\t\t", "\t");
        assert_eq!(result.as_ref(), "\t");
    }

    #[test]
    fn strip_one_indent_single_tab() {
        let result = strip_one_indent("\t", "\t");
        assert_eq!(result.as_ref(), "");
    }

    #[test]
    fn strip_one_indent_four_spaces() {
        let result = strip_one_indent("        ", "    ");
        assert_eq!(result.as_ref(), "    ");
    }

    #[test]
    fn strip_one_indent_exactly_one_level() {
        let result = strip_one_indent("    ", "    ");
        assert_eq!(result.as_ref(), "");
    }

    #[test]
    fn strip_one_indent_shorter_than_one_indent() {
        let result = strip_one_indent("  ", "    ");
        assert_eq!(result.as_ref(), "  ");
    }

    #[test]
    fn strip_one_indent_no_indent() {
        let result = strip_one_indent("", "    ");
        assert_eq!(result.as_ref(), "");
    }

    #[test]
    fn strip_one_indent_empty_line() {
        let result = strip_one_indent("", "\t");
        assert_eq!(result.as_ref(), "");
    }

    #[test]
    fn strip_one_indent_tab_takes_priority_over_spaces() {
        let result = strip_one_indent("    \t", "    ");
        assert_eq!(result.as_ref(), "    ");
    }

    // ---------------------------------------------------------------
    // starts_with_outdent_keyword
    // ---------------------------------------------------------------

    // --- Each keyword bare ---

    #[test]
    fn outdent_keyword_elif_bare() {
        assert!(starts_with_outdent_keyword("elif"));
    }

    #[test]
    fn outdent_keyword_else_bare() {
        assert!(starts_with_outdent_keyword("else"));
    }

    #[test]
    fn outdent_keyword_except_bare() {
        assert!(starts_with_outdent_keyword("except"));
    }

    #[test]
    fn outdent_keyword_finally_bare() {
        assert!(starts_with_outdent_keyword("finally"));
    }

    // --- Keywords with colon ---

    #[test]
    fn outdent_keyword_else_colon() {
        assert!(starts_with_outdent_keyword("else:"));
    }

    #[test]
    fn outdent_keyword_elif_condition() {
        assert!(starts_with_outdent_keyword("elif x > 5:"));
    }

    #[test]
    fn outdent_keyword_except_type() {
        assert!(starts_with_outdent_keyword("except ValueError:"));
    }

    #[test]
    fn outdent_keyword_finally_colon() {
        assert!(starts_with_outdent_keyword("finally:"));
    }

    // --- With parenthesis (e.g. "except(") ---

    #[test]
    fn outdent_keyword_except_paren() {
        assert!(starts_with_outdent_keyword("except(ValueError):"));
    }

    // --- False positives: word boundary enforcement ---

    #[test]
    fn outdent_keyword_rejects_elsewhere() {
        assert!(!starts_with_outdent_keyword("elsewhere"));
    }

    #[test]
    fn outdent_keyword_rejects_else_if() {
        // Underscore is treated as part of the identifier — word boundary check.
        assert!(!starts_with_outdent_keyword("else_if"));
    }

    #[test]
    fn outdent_keyword_rejects_elseif() {
        assert!(!starts_with_outdent_keyword("elseif"));
    }

    #[test]
    fn outdent_keyword_rejects_finally_method() {
        assert!(!starts_with_outdent_keyword("finalize"));
    }

    #[test]
    fn outdent_keyword_rejects_exceptional() {
        assert!(!starts_with_outdent_keyword("exceptional"));
    }

    // --- Empty line ---

    #[test]
    fn outdent_keyword_empty() {
        assert!(!starts_with_outdent_keyword(""));
    }

    #[test]
    fn outdent_keyword_else_with_space() {
        assert!(starts_with_outdent_keyword("else "));
    }

    #[test]
    fn outdent_keyword_elif_with_space() {
        assert!(starts_with_outdent_keyword("elif condition:"));
    }

    // ---------------------------------------------------------------
    // reindent_lines — core reindentation algorithm
    // ---------------------------------------------------------------

    // --- Simple indentation (no block openers) ---

    #[test]
    fn reindent_simple_no_openers() {
        let result = reindent_lines(
            &["let x = 1", "let y = 2", "let z = 3"],
            "    ",
            Some("    func ready():"),
        );
        assert_eq!(result[0], "        let x = 1");
        assert_eq!(result[1], "        let y = 2");
        assert_eq!(result[2], "        let z = 3");
    }

    #[test]
    fn reindent_simple_no_openers_flat() {
        let result = reindent_lines(
            &["a = 1", "b = 2"],
            "    ",
            Some("var x = 0"),
        );
        assert_eq!(result[0], "a = 1");
        assert_eq!(result[1], "b = 2");
    }

    // --- Block opener increases indent ---

    #[test]
    fn reindent_block_opener_colon() {
        let result = reindent_lines(
            &["if x:", "    do_something()"],
            "    ",
            Some("func ready():"),
        );
        assert_eq!(result[0], "    if x:");
        assert_eq!(result[1], "        do_something()");
    }

    #[test]
    fn reindent_block_opener_brace() {
        let result = reindent_lines(
            &["fn main() {", "    let x = 1;", "}"],
            "    ",
            None, // first line of document
        );
        assert_eq!(result[0], "fn main() {");
        assert_eq!(result[1], "    let x = 1;");
        assert_eq!(result[2], "}");
    }

    #[test]
    fn reindent_block_opener_paren() {
        let result = reindent_lines(
            &["call(", "arg1,", "arg2", ")"],
            "    ",
            None,
        );
        assert_eq!(result[0], "call(");
        assert_eq!(result[1], "    arg1,");
        assert_eq!(result[2], "    arg2");
        assert_eq!(result[3], ")");
    }

    #[test]
    fn reindent_block_opener_bracket() {
        let result = reindent_lines(
            &["arr = [", "1,", "2", "]"],
            "    ",
            None,
        );
        assert_eq!(result[0], "arr = [");
        assert_eq!(result[1], "    1,");
        assert_eq!(result[2], "    2");
        assert_eq!(result[3], "]");
    }

    // --- Outdent keyword decreases indent ---

    #[test]
    fn reindent_outdent_else() {
        let result = reindent_lines(
            &["    pass", "else:", "    other()"],
            "    ",
            Some("    if x:"),
        );
        assert_eq!(result[0], "        pass");
        assert_eq!(result[1], "    else:");
        assert_eq!(result[2], "        other()");
    }

    #[test]
    fn reindent_outdent_elif() {
        let result = reindent_lines(
            &["    pass", "elif y:"],
            "    ",
            Some("    if x:"),
        );
        assert_eq!(result[0], "        pass");
        assert_eq!(result[1], "    elif y:");
    }

    #[test]
    fn reindent_outdent_except() {
        let result = reindent_lines(
            &["    risky()", "except:"],
            "    ",
            Some("try:"),
        );
        assert_eq!(result[0], "    risky()");
        assert_eq!(result[1], "except:");
    }

    #[test]
    fn reindent_outdent_finally() {
        let result = reindent_lines(
            &["    handle()", "finally:", "    cleanup()"],
            "    ",
            Some("except:"),
        );
        assert_eq!(result[0], "    handle()");
        assert_eq!(result[1], "finally:");
        assert_eq!(result[2], "    cleanup()");
    }

    #[test]
    fn reindent_outdent_closing_brace() {
        let result = reindent_lines(
            &["    x += 1;", "}"],
            "    ",
            Some("    if (cond) {"),
        );
        assert_eq!(result[0], "        x += 1;");
        assert_eq!(result[1], "    }");
    }

    #[test]
    fn reindent_outdent_closing_paren() {
        let result = reindent_lines(
            &["    arg", ")"],
            "    ",
            Some("func("),
        );
        assert_eq!(result[0], "    arg");
        assert_eq!(result[1], ")");
    }

    #[test]
    fn reindent_outdent_closing_bracket() {
        let result = reindent_lines(
            &["    val", "]"],
            "    ",
            Some("arr = ["),
        );
        assert_eq!(result[0], "    val");
        assert_eq!(result[1], "]");
    }

    // --- Empty lines preserved ---

    #[test]
    fn reindent_empty_lines_preserved() {
        let result = reindent_lines(
            &["if x:", "", "    body()"],
            "    ",
            None,
        );
        assert_eq!(result[0], "if x:");
        assert_eq!(result[1], "");
        assert_eq!(result[2], "    body()");
    }

    #[test]
    fn reindent_multiple_empty_lines() {
        let result = reindent_lines(
            &["if x:", "", "", "body()"],
            "    ",
            None,
        );
        assert_eq!(result[0], "if x:");
        assert_eq!(result[1], "");
        assert_eq!(result[2], "");
        assert_eq!(result[3], "    body()");
    }

    // --- Mixed indent levels ---

    #[test]
    fn reindent_nested_blocks() {
        let result = reindent_lines(
            &["if a:", "    if b:", "        pass"],
            "    ",
            None,
        );
        assert_eq!(result[0], "if a:");
        assert_eq!(result[1], "    if b:");
        assert_eq!(result[2], "        pass");
    }

    #[test]
    fn reindent_nested_blocks_with_outdent() {
        let result = reindent_lines(
            &["if a:", "    pass", "else:", "    other()"],
            "    ",
            None,
        );
        assert_eq!(result[0], "if a:");
        assert_eq!(result[1], "    pass");
        assert_eq!(result[2], "else:");
        assert_eq!(result[3], "    other()");
    }

    // --- Multi-line range with shifting indentation ---

    #[test]
    fn reindent_shifting_indent_gdscript() {
        let result = reindent_lines(
            &[
                "func process(delta):",
                "    if active:",
                "        update()",
                "    else:",
                "        idle()",
            ],
            "    ",
            None,
        );
        assert_eq!(result[0], "func process(delta):");
        assert_eq!(result[1], "    if active:");
        assert_eq!(result[2], "        update()");
        assert_eq!(result[3], "    else:");
        assert_eq!(result[4], "        idle()");
    }

    #[test]
    fn reindent_c_style_braces() {
        let result = reindent_lines(
            &[
                "void foo() {",
                "    if (x) {",
                "        bar();",
                "    }",
                "}",
            ],
            "    ",
            None,
        );
        assert_eq!(result[0], "void foo() {");
        assert_eq!(result[1], "    if (x) {");
        assert_eq!(result[2], "        bar();");
        assert_eq!(result[3], "    }");
        assert_eq!(result[4], "}");
    }

    // --- Reference line from before the range ---

    #[test]
    fn reindent_ref_before_range_indented() {
        let result = reindent_lines(
            &["pass"],
            "    ",
            Some("        if nested:"),
        );
        assert_eq!(result[0], "            pass");
    }

    #[test]
    fn reindent_ref_before_range_none() {
        let result = reindent_lines(
            &["var x = 1"],
            "    ",
            None,
        );
        assert_eq!(result[0], "var x = 1");
    }

    #[test]
    fn reindent_ref_before_range_no_opener() {
        let result = reindent_lines(
            &["next_line()"],
            "    ",
            Some("    let x = 5"),
        );
        assert_eq!(result[0], "    next_line()");
    }

    // --- All lines empty ---

    #[test]
    fn reindent_all_lines_empty() {
        let result = reindent_lines(
            &["", "", ""],
            "    ",
            Some("if x:"),
        );
        assert_eq!(result.len(), 3);
        assert_eq!(result[0], "");
        assert_eq!(result[1], "");
        assert_eq!(result[2], "");
    }

    #[test]
    fn reindent_all_lines_empty_no_ref() {
        let result = reindent_lines(
            &["", ""],
            "    ",
            None,
        );
        assert_eq!(result.len(), 2);
        assert_eq!(result[0], "");
        assert_eq!(result[1], "");
    }

    // --- Tab indent mode ---

    #[test]
    fn reindent_tab_indent() {
        let result = reindent_lines(
            &["if x:", "pass", "else:", "other()"],
            "\t",
            None,
        );
        assert_eq!(result[0], "if x:");
        assert_eq!(result[1], "\tpass");
        // "\tpass" no opener → base "\t", outdent → ""
        assert_eq!(result[2], "else:");
        assert_eq!(result[3], "\tother()");
    }

    #[test]
    fn reindent_tab_nested() {
        let result = reindent_lines(
            &["func f():", "if true:", "pass"],
            "\t",
            None,
        );
        assert_eq!(result[0], "func f():");
        assert_eq!(result[1], "\tif true:");
        assert_eq!(result[2], "\t\tpass");
    }

    // --- 2-space indent ---

    #[test]
    fn reindent_two_space_indent() {
        let result = reindent_lines(
            &["if x:", "pass", "else:", "other()"],
            "  ",
            None,
        );
        assert_eq!(result[0], "if x:");
        assert_eq!(result[1], "  pass");
        assert_eq!(result[2], "else:");
        assert_eq!(result[3], "  other()");
    }

    // --- Single line ---

    #[test]
    fn reindent_single_line_with_ref() {
        let result = reindent_lines(
            &["body()"],
            "    ",
            Some("    if cond:"),
        );
        assert_eq!(result.len(), 1);
        assert_eq!(result[0], "        body()");
    }

    #[test]
    fn reindent_single_empty_line() {
        let result = reindent_lines(&[""], "    ", Some("if x:"));
        assert_eq!(result.len(), 1);
        assert_eq!(result[0], "");
    }

    // --- Empty input ---

    #[test]
    fn reindent_no_lines() {
        let result = reindent_lines(&[], "    ", Some("if x:"));
        assert!(result.is_empty());
    }

    // --- Whitespace-only lines treated as empty ---

    #[test]
    fn reindent_whitespace_only_lines_treated_as_empty() {
        let result = reindent_lines(
            &["    ", "  pass"],
            "    ",
            Some("if x:"),
        );
        assert_eq!(result[0], "");
        assert_eq!(result[1], "    pass");
    }

    // --- Fallback when all preceding results are empty ---

    #[test]
    fn reindent_fallback_to_ref_when_all_preceding_empty() {
        let result = reindent_lines(
            &["", "", "code()"],
            "    ",
            Some("    for i in range:"),
        );
        assert_eq!(result[0], "");
        assert_eq!(result[1], "");
        assert_eq!(result[2], "        code()");
    }

    #[test]
    fn reindent_fallback_to_none_when_all_preceding_empty() {
        let result = reindent_lines(
            &["", "code()"],
            "    ",
            None,
        );
        assert_eq!(result[0], "");
        assert_eq!(result[1], "code()");
    }

    // --- try/except/finally full sequence ---

    #[test]
    fn reindent_try_except_finally_sequence() {
        let result = reindent_lines(
            &[
                "try:",
                "    risky()",
                "except ValueError:",
                "    handle()",
                "finally:",
                "    cleanup()",
            ],
            "    ",
            None,
        );
        assert_eq!(result[0], "try:");
        assert_eq!(result[1], "    risky()");
        assert_eq!(result[2], "except ValueError:");
        assert_eq!(result[3], "    handle()");
        assert_eq!(result[4], "finally:");
        assert_eq!(result[5], "    cleanup()");
    }

    // --- Deeply nested ---

    #[test]
    fn reindent_deep_nesting() {
        let result = reindent_lines(
            &[
                "if a:",
                "if b:",
                "if c:",
                "pass",
            ],
            "    ",
            None,
        );
        assert_eq!(result[0], "if a:");
        assert_eq!(result[1], "    if b:");
        assert_eq!(result[2], "        if c:");
        assert_eq!(result[3], "            pass");
    }

    // --- Preserves content exactly ---

    #[test]
    fn reindent_preserves_inline_content() {
        let result = reindent_lines(
            &["  var x = 'hello world'  # comment"],
            "    ",
            Some("func ready():"),
        );
        assert_eq!(result[0], "    var x = 'hello world'  # comment");
    }

    // --- Output length matches input length ---

    #[test]
    fn reindent_output_length_matches_input() {
        let lines: Vec<&str> = vec!["a", "", "b", "", "", "c"];
        let result = reindent_lines(&lines, "    ", None);
        assert_eq!(result.len(), lines.len());
    }
}
