use crate::bridge::vim_adapter::core::cast::i32_to_usize;
use crate::bridge::vim_adapter::core::snapshot::GodotSnapshot;
use crate::bridge::vim_adapter::handlers::visual;
use crate::bridge::vim_wrapper::VimController;
use godot::classes::CodeEdit;
use godot::prelude::*;
use vim_core::domain::position::Position;
use vim_core::domain::selection::Selection;
use vim_core::inputs::commands::motions::Motion;
use vim_core::runtime::pure::{self as pure_motion, MotionContext};
use vim_core::state::config::Config;
use vim_core::state::mode::{Mode, VisualKind};
use vim_core::state::VimState;

use super::fold_vertical::execute_vertical_motion_fold_aware_public;
use crate::bridge::vim_adapter::core::cursor::{move_cursor_with_tracking, CursorMoveType};

/// Apply a pure motion (no viewport effects) and update cursor.
///
/// The `config` parameter is passed in from the cached VimController config,
/// avoiding FFI calls per motion.
pub fn apply_pure_motion(
    editor: &mut Gd<CodeEdit>,
    vim_state: &mut VimState,
    motion: Motion,
    count: usize,
    config: &Config,
) {
    if matches!(motion, Motion::Up | Motion::Down) {
        execute_vertical_motion_fold_aware_public(editor, vim_state, motion, count, config);
        return;
    }

    let current_pos = resolve_current_pos(editor, vim_state);
    let selection = build_selection_for_mode(vim_state, current_pos);
    let (new_sel, new_preferred) =
        compute_motion_result(editor, vim_state, motion, count, config, selection);
    apply_result_to_editor_and_state(editor, vim_state, motion, new_sel, new_preferred);
}

fn resolve_current_pos(editor: &Gd<CodeEdit>, vim_state: &VimState) -> Position {
    if let Mode::Visual(VisualKind::Block { start: _, cursor }) = vim_state.mode() {
        cursor
    } else {
        VimController::cursor_from_editor(editor)
    }
}

fn build_selection_for_mode(vim_state: &VimState, current_pos: Position) -> Selection {
    match vim_state.mode() {
        Mode::Visual(VisualKind::Char { start }) => Selection::new(
            Position::from_byte(start.line, start.col.as_usize()),
            current_pos,
        ),
        Mode::Visual(VisualKind::Line { start_line }) => {
            Selection::new(Position::from_byte(start_line, 0), current_pos)
        }
        Mode::Visual(VisualKind::Block { start, cursor: _ }) => Selection::new(
            Position::from_byte(start.line, start.col.as_usize()),
            current_pos,
        ),
        _ => Selection::at(current_pos),
    }
}

fn compute_motion_result(
    editor: &Gd<CodeEdit>,
    vim_state: &VimState,
    motion: Motion,
    count: usize,
    config: &Config,
    selection: Selection,
) -> (Selection, Option<vim_core::domain::column::ByteCol>) {
    let snapshot = GodotSnapshot::from_editor_with_selection(editor, selection);
    let extend = vim_state.mode().is_visual();
    let ctx = MotionContext::new(&snapshot, vim_state, count, config, None, Some(&snapshot));
    pure_motion::apply_motion(motion, &ctx, extend, vim_state.search.last_search())
}

fn resolve_move_type(editor: &Gd<CodeEdit>, motion: Motion, target_line: usize) -> CursorMoveType {
    if !motion.is_jump() {
        return CursorMoveType::Step;
    }

    let current_line = i32_to_usize(editor.get_caret_line());
    if current_line != target_line {
        CursorMoveType::Jump
    } else {
        CursorMoveType::Step
    }
}

fn apply_result_to_editor_and_state(
    editor: &mut Gd<CodeEdit>,
    vim_state: &mut VimState,
    motion: Motion,
    new_sel: Selection,
    new_preferred: Option<vim_core::domain::column::ByteCol>,
) {
    vim_state.set_preferred_column(new_preferred);

    let target_line = new_sel.head.line;
    let move_type = resolve_move_type(editor, motion, target_line);
    move_cursor_with_tracking(
        editor,
        vim_state,
        Position::from_byte(target_line, new_sel.head.col.as_usize()),
        move_type,
    );

    if vim_state.mode().is_visual() {
        visual::render_visual_selection(editor, &vim_state.mode(), new_sel.head);
    }

    if let Mode::Visual(VisualKind::Block { start, .. }) = vim_state.mode() {
        vim_state.set_mode(Mode::Visual(VisualKind::Block {
            start,
            cursor: new_sel.head,
        }));
        log::debug!(
            "Visual block cursor updated start={:?} cursor={:?}",
            start,
            new_sel.head
        );
    }
}
