use crate::bridge::godot::code_edit_ext::CodeEditExt;
use crate::bridge::vim_adapter::core::cast::{i32_to_usize, usize_to_i32};
use crate::bridge::vim_adapter::core::cursor::{move_cursor_with_tracking, CursorMoveType};
use crate::bridge::vim_adapter::core::snapshot::GodotSnapshot;
use godot::classes::CodeEdit;
use godot::prelude::*;
use vim_core::domain::position::Position;
use vim_core::inputs::commands::motions::Motion;
use vim_core::state::config::Config;
use vim_core::state::mode::{Mode, VisualKind};
use vim_core::state::VimState;

/// Godot adapter for `FoldProvider` — wraps a `CodeEdit` reference.
struct GodotFoldProvider<'a> {
    editor: &'a Gd<CodeEdit>,
}

impl vim_core::domain::fold::FoldProvider for GodotFoldProvider<'_> {
    fn next_visible_line(
        &self,
        line: usize,
        direction: vim_core::domain::fold::VerticalDirection,
    ) -> usize {
        let line_i32 = usize_to_i32(line);
        let result = match direction {
            vim_core::domain::fold::VerticalDirection::Down => {
                self.editor.move_down_visible(line_i32)
            }
            vim_core::domain::fold::VerticalDirection::Up => self.editor.move_up_visible(line_i32),
        };
        i32_to_usize(result)
    }
}

/// Execute vertical motion (Up/Down) with fold awareness.
///
/// Thin shell adapter: delegates pure algorithm to `vim_core::fold_aware_vertical_motion`,
/// then applies Godot-specific side effects (cursor tracking, visual rendering, scroll offset).
pub fn execute_vertical_motion_fold_aware_public(
    editor: &mut Gd<CodeEdit>,
    vim_state: &mut VimState,
    motion: Motion,
    count: usize,
    _config: &Config,
) {
    let from = Position::new(
        i32_to_usize(editor.get_caret_line()),
        i32_to_usize(editor.get_caret_column()),
    );

    let direction = match motion {
        Motion::Up => vim_core::domain::fold::VerticalDirection::Up,
        Motion::Down => vim_core::domain::fold::VerticalDirection::Down,
        _ => return,
    };

    let snapshot = GodotSnapshot::from_editor(editor);

    // Pure algorithm in vim-core
    let fold_provider = GodotFoldProvider { editor: &*editor };
    let (target_line, new_col, preferred_to_set) = vim_core::runtime::pure::fold_aware_vertical_motion(
        &snapshot,
        vim_state,
        from,
        count,
        direction,
        &fold_provider,
    );

    // Update the preferred column when the motion produces one.
    if let Some(pref) = preferred_to_set {
        vim_state.set_preferred_column(Some(pref));
    }

    // Shell side effects: cursor tracking
    move_cursor_with_tracking(
        editor,
        vim_state,
        Position::new(target_line, new_col),
        CursorMoveType::Step,
    );

    // Visual mode selection
    if vim_state.mode().is_visual() {
        let new_head = Position::new(target_line, new_col);
        crate::bridge::vim_adapter::handlers::visual::render_visual_selection(
            editor,
            &vim_state.mode(),
            new_head,
        );
    } else {
        editor.set_caret_column(usize_to_i32(new_col));
    }

    // Update vim_state cursor
    vim_state.set_cursor_pos(Position::new(target_line, new_col));

    // VisualBlock cursor update
    if let Mode::Visual(VisualKind::Block { start, .. }) = vim_state.mode() {
        vim_state.set_mode(Mode::Visual(VisualKind::Block {
            start,
            cursor: Position::new(target_line, new_col),
        }));
        log::debug!(
            "Visual block cursor updated start={:?} cursor={:?}",
            start,
            Position::new(target_line, new_col)
        );
    }

    // Scroll offset
    crate::bridge::vim_wrapper::VimController::apply_scroll_offset(editor);
}
