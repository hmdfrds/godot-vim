//! Buffer navigation: `:bn`, `:bp`, `:b<N>` via Godot's ScriptEditor TabContainer.

use compact_str::CompactString;
use godot::classes::{CodeEdit, EditorInterface, Node, TabContainer, TextEdit};
use godot::prelude::*;
use vim_core::execution::{HostRequestId, HostResult};

use super::{host_failure, host_success};
use crate::scene_tree::{find_descendant, MAX_DISCOVERY_DEPTH};

/// Locate the ScriptEditor's TabContainer by walking the scene tree.
///
/// Godot does not expose the ScriptEditor's tab container directly through its
/// API, so we must discover it via `find_descendant`. Returns a `HostResult`
/// error if the script editor or tab container is unavailable (e.g., editor not
/// fully initialized).
fn get_tabs(id: HostRequestId) -> Result<Gd<TabContainer>, HostResult> {
    let script_editor = EditorInterface::singleton()
        .get_script_editor()
        .ok_or_else(|| host_failure(id, "E5: No script editor available"))?;
    find_descendant::<TabContainer>(&script_editor.upcast(), MAX_DISCOVERY_DEPTH)
        .ok_or_else(|| host_failure(id, "E5: No tab container found in script editor"))
}

/// Transfer keyboard focus to the newly-selected tab's CodeEdit.
///
/// Uses `call_deferred("grab_focus")` rather than immediate `grab_focus()` because
/// Godot processes tab switches asynchronously — the new tab's control may not be
/// fully ready for focus in the current frame. Falls back to focusing the tab's
/// root control if no CodeEdit is found (e.g., visual shader editors).
fn defer_focus_to_new_tab(tabs: &Gd<TabContainer>) {
    if let Some(control) = tabs.get_current_tab_control() {
        if let Some(edit) =
            find_descendant::<CodeEdit>(&control.clone().upcast(), MAX_DISCOVERY_DEPTH)
        {
            edit.upcast::<Node>().call_deferred("grab_focus", &[]);
        } else {
            control.upcast::<Node>().call_deferred("grab_focus", &[]);
        }
    }
}

/// `:bn` / `:bp` — switch to next or previous buffer.
///
/// Vim's buffer model maps to Godot's ScriptEditor tabs: each open script
/// corresponds to one tab. Unlike Vim, there is no concept of hidden buffers,
/// so going past the first or last tab is an error rather than wrapping.
pub(super) fn handle_switch_buffer(id: HostRequestId, delta: i32) -> HostResult {
    log::debug!("buffer::switch: count={}", delta);
    let mut tabs = match get_tabs(id) {
        Ok(t) => t,
        Err(e) => return e,
    };

    let count = tabs.get_tab_count();
    if count <= 1 {
        return host_failure(id, "Only one buffer open");
    }

    let current = tabs.get_current_tab();
    let target = current as i64 + delta as i64;
    if target >= count as i64 {
        return host_failure(id, "E87: Cannot go beyond last buffer");
    }
    if target < 0 {
        return host_failure(id, "E88: Cannot go before first buffer");
    }
    let next = i32::try_from(target).expect("bounds-checked above");
    tabs.set_current_tab(next);
    defer_focus_to_new_tab(&tabs);

    host_success(id)
}

/// `:bl` / `:blast` — go to the last buffer.
pub(super) fn handle_goto_last_buffer(id: HostRequestId) -> HostResult {
    let tabs = match get_tabs(id) {
        Ok(t) => t,
        Err(e) => return e,
    };
    let count = crate::bridge::codec::i32_to_usize(tabs.get_tab_count());
    if count == 0 {
        return host_failure(id, "No buffers open");
    }
    handle_goto_buffer(id, count)
}

/// `:b<N>` — go to buffer by 1-indexed number.
pub(super) fn handle_goto_buffer(id: HostRequestId, index: usize) -> HostResult {
    log::debug!("buffer::goto: number={}", index);
    let mut tabs = match get_tabs(id) {
        Ok(t) => t,
        Err(e) => return e,
    };

    let count = crate::bridge::codec::i32_to_usize(tabs.get_tab_count());
    if count == 0 {
        return host_failure(id, "No buffers open");
    }

    if index == 0 || index > count {
        return host_failure(
            id,
            format!("E86: Buffer {} does not exist (1-{})", index, count),
        );
    }

    let target = crate::bridge::codec::usize_to_i32(index - 1);
    tabs.set_current_tab(target);
    defer_focus_to_new_tab(&tabs);

    host_success(id)
}

/// Jump to a specific editor tab identified by Godot `InstanceId`, positioning
/// the cursor at a byte offset.
///
/// Used by jump-list and tag navigation where the engine tracks buffer identity
/// via opaque `u64` IDs. The linear scan over tabs is acceptable because Godot's
/// ScriptEditor rarely has more than a few dozen tabs open.
pub(super) fn handle_jump_to_buffer(
    id: HostRequestId,
    target_id: u64,
    cursor_offset: usize,
    buffer_display: impl std::fmt::Display,
) -> HostResult {
    // vim-core stores instance IDs as u64 (unsigned), but Godot's InstanceId
    // uses i64. Bit-identical reinterpretation via ne_bytes avoids a silent
    // `as` cast that could mask sign-related bugs.
    let instance_id = InstanceId::from_i64(i64::from_ne_bytes(target_id.to_ne_bytes()));
    let mut tabs = match get_tabs(id) {
        Ok(t) => t,
        Err(e) => return e,
    };
    let count = tabs.get_tab_count();
    for i in 0..count {
        if let Some(control) = tabs.get_tab_control(i) {
            if let Some(edit) = find_descendant::<CodeEdit>(&control.upcast(), MAX_DISCOVERY_DEPTH)
            {
                if edit.instance_id() == instance_id {
                    tabs.set_current_tab(i);
                    // Convert the engine's byte offset to Godot's (line, column) pair.
                    let text = edit.get_text().to_string();
                    let line_index = crate::bridge::codec::LineIndex::new(&text);
                    let pos = line_index.byte_to_line_col(&text, cursor_offset);
                    let mut ed = edit.clone();
                    ed.set_caret_line(pos.line);
                    ed.set_caret_column(pos.col);
                    edit.upcast::<Node>().call_deferred("grab_focus", &[]);
                    return host_success(id);
                }
            }
        }
    }
    host_failure(id, format!("E94: Buffer {} not found", buffer_display))
}

/// Check if an arbitrary tab has unsaved changes without switching the active tab.
///
/// Walks into the tab's control subtree to find its CodeEdit and compares
/// Godot's `get_version()` vs `get_saved_version()` counters. If no CodeEdit
/// exists (e.g., a VisualShader tab), conservatively returns `true` so `:ls`
/// shows `+` rather than silently hiding unsaved state.
fn is_tab_unsaved(tabs: &Gd<TabContainer>, tab_index: i32) -> bool {
    let Some(control) = tabs.get_tab_control(tab_index) else {
        return false;
    };
    let Some(edit) = find_descendant::<CodeEdit>(&control.upcast(), MAX_DISCOVERY_DEPTH) else {
        return true;
    };
    // get_version()/get_saved_version() live on TextEdit in Godot's class
    // hierarchy, so we upcast from CodeEdit.
    let text_edit: Gd<TextEdit> = edit.upcast();
    text_edit.get_version() != text_edit.get_saved_version()
}

/// `:ls` / `:buffers` — list all open script tabs.
///
/// Builds a Vim-style buffer listing where each line shows:
///   `{1-indexed number}{flags}{modified} "{tab title}"`
///
/// Flag columns (matching Vim's `:ls` layout):
/// - Column 1 (`%` or space): `%` = current buffer.
/// - Column 2 (`a` or space): `a` = active (loaded and visible).
/// - Column 3 (`+` or space): `+` = buffer has unsaved modifications.
pub(super) fn handle_buffer_list(id: HostRequestId) -> HostResult {
    log::debug!("buffer::list");
    let tabs = match get_tabs(id) {
        Ok(t) => t,
        Err(e) => return e,
    };

    let count = tabs.get_tab_count();
    if count == 0 {
        return HostResult::Success {
            id,
            message: Some(CompactString::from("No buffers open")),
        };
    }

    let current = tabs.get_current_tab();
    let mut lines = Vec::with_capacity(crate::bridge::codec::i32_to_usize(count));

    for i in 0..count {
        let title = tabs.get_tab_title(i).to_string();
        let activity = if i == current { "%a" } else { "  " };
        let modified = if is_tab_unsaved(&tabs, i) { "+" } else { " " };
        lines.push(format!(
            "  {: >2}{}{} \"{}\"",
            i + 1,
            activity,
            modified,
            title
        ));
    }

    HostResult::Success {
        id,
        message: Some(CompactString::from(lines.join("\n"))),
    }
}
