//! Minimal expression evaluator for `=` register and `<expr>` mappings.
//!
//! Supports: string literals, integer literals, `mode()`, `nr2char(N)`,
//! register references (`@x`). Returns `E15` for unsupported expressions.

/// Process Vim double-quoted string escape sequences (`\n`, `\t`, `\\`, `\"`,
/// `\r`, `\e`, `\b`). Unknown escapes are preserved verbatim (matching Vim).
fn unescape_vim_double_quote(s: &str) -> std::borrow::Cow<'_, str> {
    // Fast path: no backslash means no escapes to process.
    if !s.contains('\\') {
        return std::borrow::Cow::Borrowed(s);
    }

    let mut result = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(ch) = chars.next() {
        if ch == '\\' {
            match chars.next() {
                Some('n') => result.push('\n'),
                Some('t') => result.push('\t'),
                Some('\\') => result.push('\\'),
                Some('"') => result.push('"'),
                Some('r') => result.push('\r'),
                Some('e') => result.push('\x1B'),
                Some('b') => result.push('\x08'),
                Some(other) => {
                    result.push('\\');
                    result.push(other);
                }
                None => result.push('\\'),
            }
        } else {
            result.push(ch);
        }
    }
    std::borrow::Cow::Owned(result)
}

/// Evaluate a subset of VimL expressions sufficient for practical `<expr>`
/// mappings and `=` register usage.
///
/// Intentionally limited: covers string/int literals, `mode()`, `nr2char(N)`,
/// and register references — not a full VimL interpreter. Returns `E15` for
/// anything outside this subset.
pub(super) fn eval_simple_expression<'a>(expr: &'a str, mode_str: &'a str) -> Result<std::borrow::Cow<'a, str>, String> {
    let expr = expr.trim();

    if let Some(inner) = expr.strip_prefix('\'').and_then(|s| s.strip_suffix('\'')) {
        return Ok(std::borrow::Cow::Borrowed(inner));
    }
    if let Some(inner) = expr.strip_prefix('"').and_then(|s| s.strip_suffix('"')) {
        return Ok(unescape_vim_double_quote(inner));
    }

    if let Ok(n) = expr.parse::<i64>() {
        return Ok(std::borrow::Cow::Owned(n.to_string()));
    }

    if expr == "mode()" || expr == "mode(1)" {
        return Ok(std::borrow::Cow::Borrowed(mode_str));
    }

    // nr2char(0) returns "" (not NUL), matching real Vim behavior.
    if let Some(inner) = expr.strip_prefix("nr2char(").and_then(|s| s.strip_suffix(')')) {
        if let Ok(n) = inner.trim().parse::<u32>() {
            if n == 0 {
                return Ok(std::borrow::Cow::Borrowed(""));
            }
            if let Some(ch) = char::from_u32(n) {
                return Ok(std::borrow::Cow::Owned(ch.to_string()));
            }
        }
        return Err(format!("E474: Invalid argument: {}", expr));
    }

    if expr.starts_with('@') && expr.len() == 2 {
        return Err("E354: Register evaluation not yet supported in host eval".to_string());
    }

    Err(format!("E15: Invalid expression: {}", expr))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dq(s: &str) -> String {
        eval_simple_expression(&format!("\"{}\"", s), "n").unwrap().into_owned()
    }

    fn sq(s: &str) -> String {
        eval_simple_expression(&format!("'{}'", s), "n").unwrap().into_owned()
    }

    #[test]
    fn double_quote_newline_escape() {
        assert_eq!(dq(r"\n"), "\n");
    }

    #[test]
    fn double_quote_tab_escape() {
        assert_eq!(dq(r"\t"), "\t");
    }

    #[test]
    fn double_quote_backslash_escape() {
        assert_eq!(dq(r"\\"), "\\");
    }

    #[test]
    fn double_quote_escaped_quote() {
        assert_eq!(dq(r#"\""#), "\"");
    }

    #[test]
    fn double_quote_carriage_return() {
        assert_eq!(dq(r"\r"), "\r");
    }

    #[test]
    fn double_quote_escape_char() {
        assert_eq!(dq(r"\e"), "\x1B");
    }

    #[test]
    fn double_quote_backspace() {
        assert_eq!(dq(r"\b"), "\x08");
    }

    #[test]
    fn double_quote_mixed_escapes() {
        assert_eq!(dq(r"hello\nworld"), "hello\nworld");
    }

    #[test]
    fn double_quote_no_escapes() {
        assert_eq!(dq("hello"), "hello");
    }

    #[test]
    fn double_quote_unknown_escape_preserved() {
        assert_eq!(dq(r"\q"), r"\q");
    }

    #[test]
    fn single_quote_literal_no_escapes() {
        // Vim single-quoted strings are literal: \n is two characters, not a newline.
        let result = sq(r"\n");
        assert_eq!(result.len(), 2);
        assert_eq!(result, r"\n");
    }
}
