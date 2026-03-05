//! Text editing operations: Backspace, ReplaceChar, Delete, Yank, Substitute.

use crate::bridge::godot::names::regex;
use crate::bridge::vim_adapter::core::column_codec;
use crate::bridge::vim_adapter::core::snapshot::GodotSnapshot;
use crate::bridge::vim_adapter::core::transaction;
use crate::bridge::vim_adapter::handlers::mode::ModeHandler;
use crate::bridge::vim_wrapper::VimController;
use godot::classes::CodeEdit;
use godot::prelude::*;
use vim_core::runtime::pure::{execute_open_line, execute_replace_char};
use vim_core::state::mode::{Mode, InsertMode};

pub fn handle_replace_char(editor: &mut Gd<CodeEdit>, c: char) {
    let cursor = column_codec::caret_to_core_position(editor);
    let snapshot = GodotSnapshot::from_editor(editor);
    let tx = execute_replace_char(&snapshot, cursor, c, 1);
    transaction::apply_transaction(editor, &tx);
}

pub fn execute_substitute_range(
    editor: &mut Gd<CodeEdit>,
    start: usize,
    end: usize,
    pattern: &str,
    replacement: &str,
    flags: &str,
) {
    let mut regex = godot::classes::RegEx::new_gd();
    if regex.compile(&GString::from(pattern)) != godot::global::Error::OK {
        log::error!("Invalid regex for substitute pattern={}", pattern);
        return;
    }

    let global = flags.contains('g');

    editor.begin_complex_operation();
    let safe_end = end.min((editor.get_line_count() as usize).saturating_sub(1));

    for i in start..=safe_end {
        let line = editor.get_line(i as i32).to_string();
        let result = if global {
            regex
                .call(
                    regex::methods::SUB,
                    &[
                        GString::from(&line).to_variant(),
                        GString::from(replacement).to_variant(),
                        true.to_variant(),
                    ],
                )
                .to_string()
        } else {
            regex
                .call(
                    regex::methods::SUB,
                    &[
                        GString::from(&line).to_variant(),
                        GString::from(replacement).to_variant(),
                        false.to_variant(),
                    ],
                )
                .to_string()
        };

        let result_str = result.to_string();
        if result_str != line {
            editor.set_line(i as i32, &result);
        }
    }
    editor.end_complex_operation();
}

impl VimController {
    pub(super) fn handle_open_line(
        &mut self,
        editor: &mut Gd<CodeEdit>,
        below: bool,
        count: usize,
    ) {
        let cursor = column_codec::caret_to_core_position(editor);
        let snapshot = GodotSnapshot::from_editor(editor);
        let tx = execute_open_line(&snapshot, cursor, below, count, &self.engine.config);

        // Seed the newline for repetition
        self.engine.record_insert_newline();

        transaction::apply_transaction(editor, &tx);

        // Calculate and apply smart indentation.
        let current_line = editor.get_caret_line();
        let source_line_idx = if below {
            current_line - 1
        } else {
            current_line + 1
        };

        if source_line_idx >= 0 && source_line_idx < editor.get_line_count() {
            let indent = calculate_smart_indent(editor, source_line_idx);
            if !indent.is_empty() {
                editor.insert_text_at_caret(&indent);
            }
        }

        self.engine.set_mode(Mode::Insert(InsertMode::Standard { count }));
        self.handle_mode_change(Mode::Insert(InsertMode::Standard { count }), None);
        log::debug!(
            "OpenLine{}: applied tx + smart indent",
            if below { "Below" } else { "Above" }
        );
    }
}

pub fn calculate_smart_indent(editor: &Gd<CodeEdit>, source_line_idx: i32) -> String {
    let line_text = editor.get_line(source_line_idx).to_string();

    // Extract the existing indent (leading whitespace) from the source line.
    let indent: String = line_text
        .chars()
        .take_while(|c| c.is_whitespace())
        .collect();

    // Check whether the line ends with a block-opening character.
    let trimmed = line_text.trim_end();

    let should_indent = trimmed.ends_with(':')
        || trimmed.ends_with('{')
        || trimmed.ends_with('(')
        || trimmed.ends_with('[');

    if should_indent {
        let use_spaces = editor.is_indent_using_spaces();
        let indent_size = editor.get_indent_size();

        let extra_indent = if use_spaces {
            " ".repeat(indent_size as usize)
        } else {
            "\t".to_string()
        };
        format!("{indent}{extra_indent}")
    } else {
        indent
    }
}
