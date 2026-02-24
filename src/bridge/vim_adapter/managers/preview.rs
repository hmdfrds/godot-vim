//! SubstitutePreview - Live preview for `:s/pattern/replacement/` commands.
//!
//! This module encapsulates the state and logic for showing live substitute
//! previews in the editor, including applying and reverting changes.
//!
//! ## Separation of Concerns
//! This was extracted from `VimController` and `handlers/cmdline.rs` to:
//! - Own its private state (original lines for rollback)
//! - Provide a narrow, focused interface
//! - Enable independent testing

use crate::bridge::vim_adapter::core::cast::{i32_to_usize, usize_to_i32};
use godot::classes::CodeEdit;
use godot::prelude::*;

/// State for live substitute preview (stores original lines for rollback).
#[derive(Debug, Clone)]
pub struct SubstitutePreviewState {
    /// Original line contents before preview substitution
    pub original_lines: Vec<(usize, String)>,
    /// Last applied pattern (to detect changes)
    pub last_pattern: String,
    /// Last applied replacement
    pub last_replacement: String,
}

/// Parsed substitute command for preview purposes.
#[derive(Debug)]
pub struct SubstituteCommand<'a> {
    /// Pattern to search for
    pub pattern: &'a str,
    /// Replacement text
    pub replacement: &'a str,
    /// Line range (None = current line)
    pub range: Option<(usize, usize)>,
    /// Global flag (replace all occurrences)
    pub global: bool,
    /// Whether the second delimiter was typed (ready for preview)
    pub has_second_delimiter: bool,
}

/// Manages live substitute preview state and operations.
#[derive(Debug, Default)]
pub struct SubstitutePreview {
    /// Current preview state (if any)
    state: Option<SubstitutePreviewState>,
}

impl SubstitutePreview {
    /// Creates a new SubstitutePreview with no active state.
    #[must_use]
    pub fn new() -> Self {
        Self { state: None }
    }

    /// Apply live substitute preview - modifies text in editor.
    ///
    /// Only applies replacement when the second delimiter has been typed.
    /// Before that, only the pattern is highlighted (handled by incsearch).
    pub fn apply(&mut self, cmd: &SubstituteCommand, editor: &mut Gd<CodeEdit>) {
        // Replacement preview is shown only after the second delimiter is typed.
        // e.g., `:s/foo` highlights the pattern; `:s/foo/` starts showing the replacement.
        if !cmd.has_second_delimiter {
            // Revert any existing preview — the replacement is not yet defined.
            self.revert(editor);
            return;
        }

        if cmd.pattern.is_empty() {
            self.revert(editor);
            return;
        }

        // Determine range to search
        let line_count = i32_to_usize(editor.get_line_count());
        let (start_line, end_line) = match cmd.range {
            Some((s, e)) => (s, e.min(line_count.saturating_sub(1))),
            None => {
                let line = i32_to_usize(editor.get_caret_line());
                (line, line)
            }
        };

        // Skip the update when the pattern and replacement are unchanged.
        if let Some(ref state) = self.state {
            if state.last_pattern == cmd.pattern && state.last_replacement == cmd.replacement {
                return; // No change needed
            }
        }

        // Revert previous preview first (if any)
        self.revert(editor);

        // Store original lines before modification
        let mut original_lines = Vec::new();
        for line_idx in start_line..=end_line {
            let line_text = editor.get_line(usize_to_i32(line_idx)).to_string();
            if line_text.contains(cmd.pattern) {
                original_lines.push((line_idx, line_text));
            }
        }

        if original_lines.is_empty() {
            return; // No matches
        }

        // Apply substitutions within a complex operation (single undo step).
        // end_complex_operation is called unconditionally to prevent editor state corruption.
        editor.begin_complex_operation();
        for (line_idx, original) in &original_lines {
            let new_text = if cmd.global {
                original.replace(cmd.pattern, cmd.replacement)
            } else {
                original.replacen(cmd.pattern, cmd.replacement, 1)
            };
            editor.set_line(usize_to_i32(*line_idx), &GString::from(&new_text));
        }
        editor.end_complex_operation();

        // Store state for potential rollback
        self.state = Some(SubstitutePreviewState {
            original_lines,
            last_pattern: cmd.pattern.to_string(),
            last_replacement: cmd.replacement.to_string(),
        });

        log::trace!(
            "Substitute preview applied: '{}' → '{}'",
            cmd.pattern,
            cmd.replacement
        );
    }

    /// Revert substitute preview - restore original lines.
    ///
    /// Takes ownership of state before modifying editor, but ensures
    /// `end_complex_operation` is always called even if `set_line` panics.
    pub fn revert(&mut self, editor: &mut Gd<CodeEdit>) {
        let Some(state) = self.state.take() else {
            return;
        };
        editor.begin_complex_operation();
        for (line_idx, original) in &state.original_lines {
            editor.set_line(usize_to_i32(*line_idx), &GString::from(original));
        }
        editor.end_complex_operation();
        log::trace!(
            "Substitute preview reverted ({} lines restored)",
            state.original_lines.len()
        );
    }

    /// Clear substitute preview state without reverting.
    ///
    /// Use this when the substitute command is executed successfully,
    /// so the preview changes become permanent.
    pub fn clear(&mut self) {
        self.state = None;
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// Pattern Parsing Helpers
// ═══════════════════════════════════════════════════════════════════════════════

/// Parse substitute command for preview information.
///
/// Returns `Some(SubstituteCommand)` if this is a substitute command, `None` otherwise.
#[must_use]
pub fn parse_substitute_command(text: &str) -> Option<SubstituteCommand<'_>> {
    let after_colon = text.strip_prefix(':')?;

    // Find the 's' command and determine range
    let (range, after_range) = parse_substitute_range(after_colon)?;

    // Must start with 's' followed by a delimiter
    let after_s = after_range.strip_prefix('s')?;
    if after_s.is_empty() {
        return None;
    }

    // Get delimiter (first char after 's')
    let delim = after_s.chars().next()?;
    let rest = &after_s[delim.len_utf8()..];

    // Find pattern (up to next unescaped delimiter)
    let pattern_end = find_unescaped_delimiter(rest, delim).unwrap_or(rest.len());
    let pattern = &rest[..pattern_end];

    // Find replacement (if second delimiter was found)
    let has_second_delimiter = pattern_end < rest.len();
    let (replacement, flags) = if has_second_delimiter {
        let after_pattern = &rest[pattern_end + 1..];
        let replacement_end =
            find_unescaped_delimiter(after_pattern, delim).unwrap_or(after_pattern.len());
        let replacement = &after_pattern[..replacement_end];
        let flags = if replacement_end < after_pattern.len() {
            &after_pattern[replacement_end + 1..]
        } else {
            ""
        };
        (replacement, flags)
    } else {
        ("", "")
    };

    Some(SubstituteCommand {
        pattern,
        replacement,
        range,
        global: flags.contains('g'),
        has_second_delimiter,
    })
}

/// Parse range prefix from substitute command.
///
/// Returns `(range, remaining_text)` where range is:
/// - `None` for `:s/...` (current line)
/// - `Some((0, usize::MAX))` for `:%s/...` (whole file)
/// - `Some((start, end))` for `:1,5s/...` (specific range)
fn parse_substitute_range(text: &str) -> Option<(Option<(usize, usize)>, &str)> {
    // Check for % (whole file)
    if let Some(rest) = text.strip_prefix('%') {
        return Some((Some((0, usize::MAX)), rest));
    }

    // Check for number range like "1,5" or "'<,'>"
    if let Some(comma_idx) = text.find(',') {
        // Try to parse start number
        let start_str = &text[..comma_idx];
        let after_comma = &text[comma_idx + 1..];

        // Find where the range ends (at 's')
        let end_idx = after_comma.find('s').unwrap_or(after_comma.len());
        let end_str = &after_comma[..end_idx];

        // Parse numbers (handle visual range markers)
        let start = parse_line_specifier(start_str);
        let end = parse_line_specifier(end_str);

        if let (Some(s), Some(e)) = (start, end) {
            return Some((Some((s, e)), &after_comma[end_idx..]));
        }
    }

    // Check for single line number
    let s_idx = text.find('s')?;
    if s_idx > 0 {
        let num_str = &text[..s_idx];
        if let Some(line) = parse_line_specifier(num_str) {
            return Some((Some((line, line)), &text[s_idx..]));
        }
    }

    // No range specified - current line
    Some((None, text))
}

/// Find the index of the next unescaped delimiter character.
///
/// This is public because it's also used by the incsearch pattern parser.
pub fn find_unescaped_delimiter(text: &str, delim: char) -> Option<usize> {
    let mut escaped = false;
    for (i, c) in text.char_indices() {
        if escaped {
            escaped = false;
        } else if c == '\\' {
            escaped = true;
        } else if c == delim {
            return Some(i);
        }
    }
    None
}

/// Parse a line specifier from a range specification.
///
/// Returns a placeholder value for symbolic references (`.`, `$`, `'<`, `'>`):
/// - `.` (current line) -> `Some(0)` (placeholder, resolved by caller)
/// - `$` (last line) -> `Some(usize::MAX)` (placeholder, resolved by caller)
/// - `'<`/`'>` (visual markers) -> `Some(0)` (placeholder, resolved by caller)
/// - Numeric -> `Some(n - 1)` (convert 1-indexed to 0-indexed)
fn parse_line_specifier(s: &str) -> Option<usize> {
    let trimmed = s.trim();
    if trimmed == "." {
        return Some(0);
    }
    if trimmed == "$" {
        return Some(usize::MAX);
    }
    if trimmed == "'<" || trimmed == "'>" {
        return Some(0);
    }
    trimmed.parse().ok().map(|n: usize| n.saturating_sub(1))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_simple_substitute() {
        let cmd = parse_substitute_command(":s/foo/bar/").unwrap();
        assert_eq!(cmd.pattern, "foo");
        assert_eq!(cmd.replacement, "bar");
        assert!(cmd.has_second_delimiter);
        assert!(!cmd.global);
    }

    #[test]
    fn test_parse_global_substitute() {
        let cmd = parse_substitute_command(":s/foo/bar/g").unwrap();
        assert!(cmd.global);
    }

    #[test]
    fn test_parse_whole_file_substitute() {
        let cmd = parse_substitute_command(":%s/foo/bar/").unwrap();
        assert_eq!(cmd.range, Some((0, usize::MAX)));
    }

    #[test]
    fn test_parse_incomplete_substitute() {
        let cmd = parse_substitute_command(":s/foo").unwrap();
        assert_eq!(cmd.pattern, "foo");
        assert!(!cmd.has_second_delimiter);
    }
}
