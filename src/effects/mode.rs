//! Applies mode-transition effects: set mode, command-line edits, insert
//! entry bookkeeping, and autocomplete popup dismissal.

use vim_core::primitives::{CommandLineEdit, InsertEntryType, Mode, Offset};

use crate::bridge::port::IdeCapable;

/// Imperative side effects of mode transitions that cannot be expressed
/// through the pull-model `UiSnapshot`. Currently: dismiss Godot's
/// autocomplete popup and parameter hint tooltip on Normal mode entry,
/// preventing them from lingering after Escape.
pub(super) fn handle_set_mode(editor: &mut impl IdeCapable, mode: Mode) {
    log::debug!("Mode -> {}", mode);

    if mode.is_normal() {
        editor.cancel_code_completion();
        editor.dismiss_code_hint();
    }
}

/// Log-only: command-line state is rendered via pull-model `UiSnapshot`.
pub(super) fn handle_command_line_edit(edit: CommandLineEdit) {
    log::trace!("CommandLineEdit: {:?}", edit);
}

/// Log-only: insert-mode entry metadata is engine-internal.
pub(super) fn handle_begin_insert(
    entry_type: InsertEntryType,
    count: u32,
    auto_indent_len: usize,
    entry_offset: Offset,
) {
    log::trace!(
        "BeginInsert: entry_type={:?}, count={}, auto_indent_len={}, entry_offset={}",
        entry_type,
        count,
        auto_indent_len,
        entry_offset.get()
    );
}

/// Log-only: block-insert metadata is engine-internal.
pub(super) fn handle_set_block_insert(
    lines_below: usize,
    grapheme_col: usize,
    cursor_return_offset: Offset,
) {
    log::trace!(
        "SetBlockInsert: lines_below={}, grapheme_col={}, cursor_return_offset={}",
        lines_below,
        grapheme_col,
        cursor_return_offset.get()
    );
}
