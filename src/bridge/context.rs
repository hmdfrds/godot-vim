//! Godot CodeEdit fold/indent provider adapters for vim-core's `FoldProvider`
//! and `IndentProvider` traits.
//!
//! The owned variants (`OwnedGodotFoldProvider`, `OwnedGodotIndentProvider`)
//! are stored as fields inside `GodotHost` and used via `VimHost::providers()`.

use godot::classes::CodeEdit;
use godot::prelude::*;
use vim_core::document::{FoldProvider, IndentProvider, IndentResult};
use vim_core::primitives::{Direction, LineNumber};

use super::code_edit_ext::CodeEditExt;
use super::codec;

/// Owned fold provider for use as a field in [`GodotHost`].
///
/// `Gd::clone()` is a cheap refcount bump, so owning the handle has
/// negligible cost. The owned variant satisfies `VimHost::providers()`
/// which returns references tied to `&self`.
pub(crate) struct OwnedGodotFoldProvider {
    editor: Gd<CodeEdit>,
}

impl OwnedGodotFoldProvider {
    #[must_use]
    pub(crate) fn new(editor: Gd<CodeEdit>) -> Self {
        Self { editor }
    }
}

impl FoldProvider for OwnedGodotFoldProvider {
    fn next_visible_line(&self, line: LineNumber, direction: Direction) -> LineNumber {
        if !self.is_folded(line) {
            return line;
        }
        let line_i32 = codec::usize_to_i32(usize::from(line));
        let result = match direction {
            Direction::Forward => self.editor.move_down_visible(line_i32),
            Direction::Backward => self.editor.move_up_visible(line_i32),
            _ => {
                log::warn!(
                    "Unknown Direction variant {:?} in next_visible_line — treating as Forward",
                    direction
                );
                self.editor.move_down_visible(line_i32)
            }
        };
        LineNumber::from(codec::i32_to_usize(result))
    }

    fn is_folded(&self, line: LineNumber) -> bool {
        let line_i32 = codec::usize_to_i32(usize::from(line));
        let line_count = self.editor.get_line_count();
        if line_i32 >= line_count {
            log::trace!("is_folded: line {} >= count {}", line_i32, line_count);
            return false;
        }
        let offset = self.editor.get_next_visible_line_offset_from(line_i32, 1);
        offset > 1
    }
}

/// Owned indent provider for use as a field in [`GodotHost`].
///
/// Implements a simple heuristic for `o`/`O` commands: if the current line
/// ends with a block-opening character (`:`, `{`, `(`, `[`), the new line
/// gets one extra indent level. The trailing character is checked against
/// Godot's syntax highlighter to avoid false positives inside string literals.
pub(crate) struct OwnedGodotIndentProvider {
    editor: Gd<CodeEdit>,
}

// SAFETY: OwnedGodotIndentProvider is only constructed and used on the main thread
// (Godot's scene tree callback path). The `Send` bound on `IndentProvider`
// exists for hosts that may transfer providers across threads; Godot's
// single-threaded model ensures no actual cross-thread transfer occurs.
unsafe impl Send for OwnedGodotIndentProvider {}

impl OwnedGodotIndentProvider {
    #[must_use]
    pub(crate) fn new(editor: Gd<CodeEdit>) -> Self {
        Self { editor }
    }
}

impl IndentProvider for OwnedGodotIndentProvider {
    fn indent_for_new_line(&self, line: LineNumber) -> IndentResult {
        use compact_str::CompactString;

        let line_i32 = codec::usize_to_i32(usize::from(line));
        let line_count = self.editor.get_line_count();
        if line_i32 >= line_count {
            return IndentResult::Simple {
                indent: CompactString::default(),
                append: None,
            };
        }

        let line_text = self.editor.get_line(line_i32).to_string();

        let indent: CompactString = line_text
            .chars()
            .take_while(|&c| c == ' ' || c == '\t')
            .collect();

        let trimmed = line_text.trim_end();
        let last_char_col = codec::usize_to_i32(trimmed.chars().count().saturating_sub(1));
        let should_indent = !trimmed.is_empty()
            && (trimmed.ends_with(':')
                || trimmed.ends_with('{')
                || trimmed.ends_with('(')
                || trimmed.ends_with('['))
            && self
                .editor
                .is_in_string_ex(line_i32)
                .column(last_char_col)
                .done()
                == -1;

        let indent = if should_indent {
            let use_spaces = self.editor.is_indent_using_spaces();
            let indent_size = self.editor.safe_indent_size();
            if use_spaces {
                compact_str::format_compact!("{indent}{}", " ".repeat(indent_size))
            } else {
                compact_str::format_compact!("{indent}\t")
            }
        } else {
            indent
        };

        IndentResult::Simple {
            indent,
            append: None,
        }
    }
}
