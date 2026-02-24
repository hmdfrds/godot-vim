//! Edit operation handlers for Vim commands.
//!
//! Provides paste functionality. Block insert/append and other edit handlers
//! are implemented in `vim_wrapper.rs`.
//!
//! Paste operations use the pure `execute_paste` function from the core.
//! Only clipboard I/O and transaction application are handled here.

use crate::bridge::vim_adapter::core::cast::i32_to_usize;
use crate::bridge::vim_adapter::core::snapshot::GodotSnapshot;
use crate::bridge::vim_adapter::core::transaction;
use vim_core::domain::position::Position;
use vim_core::domain::selection::SelectionMode;
use vim_core::runtime::pure::execute_paste;

use godot::classes::{CodeEdit, DisplayServer};
use godot::prelude::*;

// ═══════════════════════════════════════════════════════════════════════════════
// PASTE
// ═══════════════════════════════════════════════════════════════════════════════

/// Perform paste operation using pure core logic.
pub fn perform_paste(
    editor: &mut Gd<CodeEdit>,
    after: bool,
    register: Option<char>,
    count: usize,
    adjust_indent: bool,
    move_cursor_to_end: bool,
    engine: &crate::bridge::vim_adapter::engine::VimEngine,
) {
    let (text, mode) = read_register(engine, register);
    if text.is_empty() {
        return;
    }

    let cursor = Position::new(
        i32_to_usize(editor.get_caret_line()),
        i32_to_usize(editor.get_caret_column()),
    );

    let snapshot = GodotSnapshot::from_editor(editor);
    let tx = execute_paste(
        &snapshot,
        cursor,
        &text,
        mode,
        after,
        count,
        adjust_indent,
        move_cursor_to_end,
    );

    editor.begin_complex_operation();
    transaction::apply_transaction(editor, &tx);
    editor.end_complex_operation();
}

// ═══════════════════════════════════════════════════════════════════════════════
// REGISTER HELPERS
// ═══════════════════════════════════════════════════════════════════════════════

fn read_register(
    engine: &crate::bridge::vim_adapter::engine::VimEngine,
    register: Option<char>,
) -> (String, SelectionMode) {
    let reg = register.unwrap_or('"');
    if reg == '+' || reg == '*' {
        let ds = DisplayServer::singleton();
        // Clipboard text is always character-wise by default
        (ds.clipboard_get().to_string(), SelectionMode::CharWise)
    } else {
        engine
            .register_get(reg)
            .map(|(text, mode)| (text.to_string(), *mode))
            .unwrap_or_default()
    }
}
