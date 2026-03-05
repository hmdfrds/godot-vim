use crate::bridge::types::mode::{CmdLineKind, EditorMode};
use vim_core::state::mode::{CmdType, InsertMode, Mode, PendingMode, ReplaceMode, VisualKind};

/// Convert vim-core `Mode` to shell `EditorMode`.
///
/// Collapses internal states into user-visible modes.
#[inline]
#[must_use]
pub fn mode_to_editor_mode(mode: &Mode) -> EditorMode {
    match mode {
        Mode::Normal => EditorMode::Normal,
        Mode::Insert(InsertMode::InsertNormal) => EditorMode::InsertNormal,
        Mode::Insert(..) => EditorMode::Insert,
        Mode::Visual(VisualKind::Char { .. }) => EditorMode::Visual,
        Mode::Visual(VisualKind::Line { .. }) => EditorMode::VisualLine,
        Mode::Visual(VisualKind::Block { .. }) => EditorMode::VisualBlock,
        // VisualCharPending is a Replace sub-mode; matched before the Replace(..) wildcard arm.
        Mode::Replace(ReplaceMode::VisualCharPending { .. }) => EditorMode::Visual,
        Mode::Replace(..) => EditorMode::Replace,
        Mode::CmdLine(CmdType::Ex) | Mode::CmdLine(CmdType::ExVisualRange) => {
            EditorMode::CmdLine(CmdLineKind::Ex)
        }
        Mode::CmdLine(CmdType::SearchForward) => EditorMode::CmdLine(CmdLineKind::SearchForward),
        Mode::CmdLine(CmdType::SearchBackward) => EditorMode::CmdLine(CmdLineKind::SearchBackward),
        Mode::CmdLine(CmdType::Filter) => EditorMode::CmdLine(CmdLineKind::Ex),
        Mode::Recording { register } => EditorMode::Recording {
            register: *register,
        },
        // All pending modes collapse to their parent visible mode
        Mode::Pending(PendingMode::VisualTextObject { .. })
        | Mode::Pending(PendingMode::Register { visual: Some(_) }) => EditorMode::Visual,
        Mode::Pending(..) => EditorMode::OperatorPending,
    }
}
