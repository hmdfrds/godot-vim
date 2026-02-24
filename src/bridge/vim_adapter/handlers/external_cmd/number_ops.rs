//! Number manipulation: IncrementNumber, DecrementNumber (<C-a>/<C-x>).

use crate::bridge::vim_adapter::core::cast::{i32_to_usize, usize_to_i32};
use godot::classes::CodeEdit;
use godot::prelude::*;

/// Modify the number at or after the cursor by delta.
pub fn modify_number_at_cursor(editor: &mut Gd<CodeEdit>, delta: i64) {
    let line_idx = editor.get_caret_line();
    let col = i32_to_usize(editor.get_caret_column());
    let line_text = editor.get_line(line_idx).to_string();
    let chars: Vec<char> = line_text.chars().collect();

    let Some((num_start, num_end, radix, is_negative)) = find_number_at_or_after(&chars, col)
    else {
        return;
    };

    let num_str: String = chars[num_start..num_end].iter().collect();
    let parse_str = num_str
        .strip_prefix("0x")
        .or_else(|| num_str.strip_prefix("0X"))
        .or_else(|| num_str.strip_prefix("0o"))
        .or_else(|| num_str.strip_prefix("0O"))
        .or_else(|| num_str.strip_prefix("0b"))
        .or_else(|| num_str.strip_prefix("0B"))
        .unwrap_or(&num_str);

    let Ok(value) = i64::from_str_radix(parse_str, radix) else {
        return;
    };

    let signed_value = if is_negative { -value } else { value };
    let new_value = signed_value.saturating_add(delta);
    // Decimal numbers are formatted without zero-padding; non-decimal numbers preserve digit width.
    let new_str = format_number(
        new_value,
        radix,
        if radix == 10 { 0 } else { parse_str.len() },
    );

    let replace_start = if is_negative {
        num_start - 1
    } else {
        num_start
    };
    apply_number_replacement(editor, line_idx, replace_start, num_end, &new_str);
}

/// Find a number at or after the cursor position.
/// If cursor is inside a number, return that entire number.
/// If cursor is not on a digit, search forward.
fn find_number_at_or_after(chars: &[char], col: usize) -> Option<(usize, usize, u32, bool)> {
    let len = chars.len();
    if len == 0 {
        return None;
    }

    // First: check if cursor is on or inside a number
    // Walk backwards to find the true start of the number
    let mut start = col.min(len - 1);

    // If cursor is on a digit, find the actual start of the number
    if start < len && chars[start].is_ascii_digit() {
        // Walk backwards to find the start
        while start > 0
            && (chars[start - 1].is_ascii_digit()
                || chars[start - 1] == 'x'
                || chars[start - 1] == 'X'
                || chars[start - 1] == 'o'
                || chars[start - 1] == 'O'
                || chars[start - 1] == 'b'
                || chars[start - 1] == 'B')
        {
            start -= 1;
        }
        // Check for leading 0 for radix prefix (0x, 0o, 0b)
        if start > 0 && chars[start - 1] == '0' {
            let c = chars[start];
            if c == 'x' || c == 'X' || c == 'o' || c == 'O' || c == 'b' || c == 'B' {
                start -= 1;
            }
        }
        // Check for hex digits (a-f) if we're in a 0x prefix
        if start + 1 < len
            && chars[start] == '0'
            && (chars[start + 1] == 'x' || chars[start + 1] == 'X')
        {
            // Hex number: the cursor may be positioned on an a-f digit.
        }
    } else {
        // Cursor is not on a digit, search forward for a number
        while start < len && !chars[start].is_ascii_digit() {
            start += 1;
        }
        if start >= len {
            return None;
        }
    }

    // Now parse from the start position
    let is_negative = start > 0 && chars[start - 1] == '-';

    // Detect radix
    let radix = if start + 1 < len && chars[start] == '0' {
        match chars[start + 1] {
            'x' | 'X' => 16,
            'o' | 'O' => 8,
            'b' | 'B' => 2,
            _ => 10,
        }
    } else {
        10
    };

    // Find end
    let mut end = start;
    if radix != 10 && end + 1 < len {
        end += 2; // Skip 0x/0o/0b prefix
    }

    while end < len {
        let c = chars[end];
        let valid = match radix {
            16 => c.is_ascii_hexdigit(),
            10 => c.is_ascii_digit(),
            8 => matches!(c, '0'..='7'),
            2 => matches!(c, '0' | '1'),
            _ => false,
        };
        if valid {
            end += 1;
        } else {
            break;
        }
    }

    if end > start {
        Some((start, end, radix, is_negative))
    } else {
        None
    }
}

fn format_number(value: i64, radix: u32, min_digits: usize) -> String {
    let abs_value = value.unsigned_abs();
    let sign = if value < 0 { "-" } else { "" };

    match radix {
        16 => format!("{sign}0x{abs_value:0>width$x}", width = min_digits),
        8 => format!("{sign}0o{abs_value:0>width$o}", width = min_digits),
        2 => format!("{sign}0b{abs_value:0>width$b}", width = min_digits),
        _ => format!("{value}"),
    }
}

fn apply_number_replacement(
    editor: &mut Gd<CodeEdit>,
    line_idx: i32,
    start: usize,
    end: usize,
    new_str: &str,
) {
    editor.remove_text(line_idx, usize_to_i32(start), line_idx, usize_to_i32(end));
    editor.set_caret_column(usize_to_i32(start));
    editor.insert_text_at_caret(new_str);
    // Position cursor at end of number (on last digit)
    let new_end = start + new_str.chars().count();
    editor.set_caret_column(usize_to_i32(new_end.saturating_sub(1)));
}
