use crate::bridge::components::cmdline::VimCmdLine;
use crate::bridge::vim_adapter::core::cast::usize_to_i32;
use crate::bridge::vim_adapter::core::column_codec;
use godot::classes::text_edit::CaretType;
use godot::classes::CodeEdit;
use godot::prelude::*;
use vim_core::domain::position::Position;
use vim_core::state::mode::{Mode, ReplaceMode, VisualKind};

/// Updates cursor visuals (block vs line) and status bar.
pub fn update_cursor_visuals(
    mode: &Mode,
    editor: &mut Gd<CodeEdit>,
    _cmdline: &mut Option<Gd<VimCmdLine>>,
) {
    let caret_type = match mode {
        Mode::Insert(..) => CaretType::LINE,
        _ => CaretType::BLOCK,
    };
    editor.set_caret_type(caret_type);
    editor.set_caret_blink_enabled(matches!(
        mode,
        Mode::Insert(..)
            | Mode::Replace(ReplaceMode::Overwrite)
            | Mode::Replace(ReplaceMode::Virtual)
    ));
}

/// Renders visual selection (highlighting) for all visual modes.
///
/// This is the source of truth for translating logical Vim selection to
/// physical Godot `CodeEdit` selections.
///
/// Vim uses inclusive character-based selection: when the cursor is on 'f' in "func",
/// pressing 'v' selects 'f'. In Godot, `select(from, to)` is exclusive on `to`,
/// so `select(col, col+1)` is required to select the character at `col`.
///
/// Uses `set_caret_column` + `select_ex` with `use_selection_origin` to ensure
/// the selection and caret are placed correctly even with tabs/mixed-width characters.
pub fn render_visual_selection(editor: &mut Gd<CodeEdit>, mode: &Mode, head: Position) {
    match mode {
        Mode::Visual(VisualKind::Char { start }) => {
            let start_line = usize_to_i32(start.line);
            let current_line = usize_to_i32(head.line);
            let start_col = usize_to_i32(column_codec::byte_to_editor_col_in_editor(
                editor,
                start.line,
                start.col.as_usize(),
            ));
            let current_col = usize_to_i32(column_codec::byte_to_editor_col_in_editor(
                editor,
                head.line,
                head.col.as_usize(),
            ));

            // Vim visual mode is INCLUSIVE on both ends.
            // Determine low and high positions.
            let (low_line, low_col, high_line, high_col) =
                if (start_line, start_col) <= (current_line, current_col) {
                    (start_line, start_col, current_line, current_col)
                } else {
                    (current_line, current_col, start_line, start_col)
                };

            // Vim visual mode is INCLUSIVE - both start and end characters are selected.
            // Godot's select() is EXCLUSIVE on the end - select(a, b) highlights chars [a, b).
            // A BLOCK caret visually covers the character AT the caret column.
            //
            // Forward: caret at high_col, block caret covers high_col, selection [low, high) covers the rest.
            //   Visual result: [low_col, high_col] inclusive.
            // Backward: caret at low_col, block caret covers low_col, selection origin at high+1
            //   so [low, high+1) covers [low, high]. Block caret overlaps within selection.
            if (start_line, start_col) <= (current_line, current_col) {
                // Forward: block caret at head covers the inclusive end
                editor.select(low_line, low_col, high_line, high_col);
            } else {
                // Backward: origin at high+1 for inclusive start, caret at low
                editor.select(high_line, high_col + 1, low_line, low_col);
            }
        }
        Mode::Visual(VisualKind::Line { start_line: start }) => {
            let current_line = usize_to_i32(head.line);
            let start = usize_to_i32(*start);

            if current_line < start {
                // Backward: Anchor at End of Start, Head at Top of Current
                let anchor_len = editor.get_line(start).len();
                editor.select(start, usize_to_i32(anchor_len), current_line, 0);
            } else {
                // Forward: Anchor at Start of Start, Head at End of Current
                let head_len = editor.get_line(current_line).len();
                editor.select(start, 0, current_line, usize_to_i32(head_len));
            }
        }
        Mode::Visual(VisualKind::Block { .. }) => {
            update_visual_block(mode, editor, head);
        }
        _ => {}
    }
}

/// Updates visual block selection display.
pub fn update_visual_block(mode: &Mode, editor: &mut Gd<CodeEdit>, head: Position) {
    if let Mode::Visual(VisualKind::Block { start, cursor: _ }) = mode {
        let (start_line, start_col) = (start.line, start.col.as_usize());
        let (current_line, current_col) = (head.line, head.col.as_usize());

        let min_line = start_line.min(current_line);
        let max_line = start_line.max(current_line);

        let min_col = start_col.min(current_col);
        let max_col = start_col.max(current_col);

        // Clear previous secondary carets before adding new ones
        editor.remove_secondary_carets();

        // First, apply selection to the primary caret (caret 0)

        // Determine selection endpoints based on direction to ensure Inclusive visual behavior
        // Forward: Left->Right. We want Caret at Max. Selection indices [Min, Max).
        //          Visual: Selection + BlockCaret(Max) = [Min, Max].
        //          Call: select(Min, Max)
        // Backward: Right->Left. We want Caret at Min. Selection indices [Min, Max].
        //          Visual: BlockCaret(Min) + Selection = [Min, Max].
        //          Call: select(Max+1, Min)

        let current_line_i32 = usize_to_i32(current_line);
        let current_min_col =
            column_codec::byte_to_editor_col_in_editor(editor, current_line, min_col);
        let current_max_col =
            column_codec::byte_to_editor_col_in_editor(editor, current_line, max_col);
        let (render_anchor, render_head) = if current_col == min_col {
            // Backward
            (
                usize_to_i32(current_max_col + 1),
                usize_to_i32(current_min_col),
            )
        } else {
            // Forward
            // The selection includes the cursor column (max_col).
            (
                usize_to_i32(current_min_col),
                usize_to_i32(current_max_col + 1),
            )
        };

        // Primary caret
        editor.select(
            current_line_i32,
            render_anchor,
            current_line_i32,
            render_head,
        );

        // Secondary carets for other lines
        for line in min_line..=max_line {
            if line == current_line {
                continue;
            }

            let line_i32 = usize_to_i32(line);
            let line_min_col = column_codec::byte_to_editor_col_in_editor(editor, line, min_col);
            let line_max_col = column_codec::byte_to_editor_col_in_editor(editor, line, max_col);
            let line_caret_col =
                column_codec::byte_to_editor_col_in_editor(editor, line, current_col);
            let (line_render_anchor, line_render_head) = if current_col == min_col {
                (usize_to_i32(line_max_col + 1), usize_to_i32(line_min_col))
            } else {
                (usize_to_i32(line_min_col), usize_to_i32(line_max_col + 1))
            };
            let new_caret_idx = editor.add_caret(line_i32, usize_to_i32(line_caret_col));

            if new_caret_idx >= 0 {
                editor
                    .select_ex(line_i32, line_render_anchor, line_i32, line_render_head)
                    .caret_index(new_caret_idx)
                    .done();
            }
        }

        editor.set_caret_blink_enabled(false);
    }
}
