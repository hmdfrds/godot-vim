//! Focus classification for global input dispatch.
//!
//! Classifies the currently focused Godot control into one of five categories
//! that determine how `input()` processes keystrokes. This is the single
//! decision point that separates "our editor" (full Vim handling) from docks
//! (simplified hjkl nav) from foreign controls (pass-through).

use godot::classes::{Control, LineEdit, Viewport};
use godot::prelude::*;

use super::dock_search::find_sibling_nav_control;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DockKind {
    Tree,
    ItemList,
    RichTextLabel,
}

#[derive(Debug)]
pub(crate) enum FocusContext {
    /// Our attached CodeEdit — handled by `gui_input` signal, not `input()`.
    Editor,
    /// A navigable dock control (Tree/ItemList/RichTextLabel). Receives
    /// simplified Vim-style j/k/h/l navigation and `/` for search.
    Dock(DockKind, Gd<Control>),
    /// A dock's filter/search LineEdit. Only ESC and Enter are intercepted;
    /// all other keys pass through for normal text input.
    SearchBox(Gd<LineEdit>),
    /// A text input (LineEdit/TextEdit/CodeEdit) that isn't ours and isn't a
    /// dock search box. All input passes through unmodified to avoid breaking
    /// Godot's native editing (e.g., project settings dialogs, addon UIs).
    Foreign,
    /// No focus owner or unrecognized control type. Only cross-panel
    /// navigation (Ctrl+hjkl) is attempted.
    Unknown,
}

/// Uses `is_class()` string checks throughout to catch Godot subclasses
/// (e.g., `FileSystemList` inherits `ItemList`, `FileSystemTree` inherits
/// `Tree`) without requiring exact type matching or a hardcoded allowlist.
pub(crate) fn classify_focus(
    viewport: &Gd<Viewport>,
    attached_editor_id: Option<InstanceId>,
) -> FocusContext {
    let Some(focus_owner) = viewport.gui_get_focus_owner() else {
        return FocusContext::Unknown;
    };

    if focus_owner.is_class("CodeEdit") {
        if let Some(attached_id) = attached_editor_id {
            if focus_owner.instance_id() == attached_id {
                return FocusContext::Editor;
            }
        }
        // A CodeEdit that isn't ours (e.g., addon editor) — must not intercept.
        return FocusContext::Foreign;
    }

    if focus_owner.is_class("Tree") {
        return FocusContext::Dock(DockKind::Tree, focus_owner.clone().upcast());
    }
    if focus_owner.is_class("ItemList") {
        return FocusContext::Dock(DockKind::ItemList, focus_owner.clone().upcast());
    }
    if focus_owner.is_class("RichTextLabel") {
        return FocusContext::Dock(DockKind::RichTextLabel, focus_owner.clone().upcast());
    }

    // A LineEdit is a search box only if it has a sibling navigable control
    // within the same dock boundary. Otherwise it's a foreign text input
    // (e.g., project settings, dialog fields).
    if focus_owner.is_class("LineEdit") {
        let Ok(line_edit) = focus_owner.clone().try_cast::<LineEdit>() else {
            return FocusContext::Foreign;
        };
        let as_control: Gd<Control> = line_edit.clone().upcast();
        if find_sibling_nav_control(&as_control).is_some() {
            return FocusContext::SearchBox(line_edit);
        }
        return FocusContext::Foreign;
    }

    // TextEdit that isn't CodeEdit (e.g., shader editor) — foreign.
    if focus_owner.is_class("TextEdit") {
        return FocusContext::Foreign;
    }

    log::trace!("classify_focus: Unknown (unrecognized control type)");
    FocusContext::Unknown
}
