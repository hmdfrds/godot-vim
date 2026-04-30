//! Godot-specific ex-command handlers for `:CustomExCommand` dispatch.
//!
//! These commands have no Vim equivalent — they bridge Vim's `:` command line to
//! Godot editor operations (scene lifecycle, debugger, dock focus, distraction-free
//! mode). Each handler chain returns `Option<HostResult>`: `None` means "not my
//! command, try the next chain". The top-level dispatcher tries each chain in
//! registration order and falls back to `E492`.

use compact_str::CompactString;
use godot::classes::{CodeEdit, Control, EditorInterface, Node};
use godot::prelude::*;
use vim_core::execution::{HostRequestId, HostResult};

use super::{host_failure, host_success};
use crate::bridge::godot_calls;
use crate::scene_tree::find_descendant_by;
use crate::types::ForceOverride;

// ---------------------------------------------------------------------------
// Debugger helpers
// ---------------------------------------------------------------------------

/// Locate the debugger node by wildcard search from the editor's scene root.
///
/// The debugger is not exposed through `EditorInterface`, so we must discover it
/// via `find_child_ex("*EditorDebugger*")`. This is stable across Godot 4.x
/// minor versions where the debugger's class name has remained consistent.
fn get_debugger_node() -> Option<Gd<Node>> {
    let editor_iface = EditorInterface::singleton();
    let main_screen = editor_iface.get_editor_main_screen()?;
    let root = main_screen.upcast::<Node>();
    let tree_root = root.get_tree()?.get_root()?;
    // COMPAT: Wildcard match on internal EditorDebugger node name.
    tree_root
        .find_child_ex(&GString::from("*EditorDebugger*"))
        .recursive(true)
        .done()
}

fn call_debugger_method(method: &str) -> bool {
    if let Some(mut node) = get_debugger_node() {
        node.call(&StringName::from(method), &[]);
        true
    } else {
        false
    }
}

fn debugger_action(
    id: HostRequestId,
    method: &str,
    success_message: Option<&str>,
) -> Option<HostResult> {
    if call_debugger_method(method) {
        Some(HostResult::Success {
            id,
            message: success_message.map(CompactString::from),
        })
    } else {
        Some(host_failure(id, "E5: Debugger panel not found"))
    }
}

/// Find the first focusable child (Tree, ItemList, or RichTextLabel) inside a
/// dock container and `grab_focus()` on it. Dock containers themselves are
/// typically `FOCUS_NONE` (MarginContainer), so focusing the container directly
/// is a no-op — we must find an actual focusable widget.
fn grab_focus_on_dock(root: &Gd<Node>, id: HostRequestId) -> Option<HostResult> {
    const MAX_DEPTH: u32 = 14;

    let focusable = find_descendant_by(root, MAX_DEPTH, &|node| {
        let control = node.clone().try_cast::<Control>().ok()?;
        (control.is_visible_in_tree()
            && (node.is_class("Tree")
                || node.is_class("ItemList")
                || node.is_class("RichTextLabel")))
        .then_some(control)
    });

    if let Some(child) = focusable {
        child
            .clone()
            .upcast::<Node>()
            .call_deferred("grab_focus", &[]);
        Some(host_success(id))
    } else {
        Some(host_failure(id, "E5: No focusable control found in dock"))
    }
}

// ── Scene commands ──────────────────────────────────────────────────────

/// Maps Vim ex-commands to Godot scene lifecycle: `:run`/`:play` triggers the
/// main scene, `:runcurrent` plays the active scene, `:stop` halts playback.
fn handle_scene_command(id: HostRequestId, cmd: &str) -> Option<HostResult> {
    let mut editor_interface = EditorInterface::singleton();
    match cmd {
        "run" | "play" => {
            editor_interface.play_main_scene();
            Some(host_success(id))
        }
        "runcurrent" | "playcurrent" => {
            editor_interface.play_current_scene();
            Some(host_success(id))
        }
        "stop" => {
            if editor_interface.is_playing_scene() {
                editor_interface.stop_playing_scene();
            }
            Some(host_success(id))
        }
        _ => None,
    }
}

// ── Editor chrome ───────────────────────────────────────────────────────

/// `:zen` / `:unzen` — toggle Godot's distraction-free (zen) mode.
fn handle_editor_state_command(id: HostRequestId, cmd: &str) -> Option<HostResult> {
    let mut editor_interface = EditorInterface::singleton();
    match cmd {
        "zen" => {
            editor_interface.set_distraction_free_mode(true);
            Some(host_success(id))
        }
        "unzen" => {
            editor_interface.set_distraction_free_mode(false);
            Some(host_success(id))
        }
        _ => None,
    }
}

// ── Debugger ────────────────────────────────────────────────────────────

/// Maps Vim ex-commands to Godot debugger actions. Breakpoint toggling operates
/// on the CodeEdit directly; all other actions call methods on the EditorDebugger
/// node discovered at runtime.
fn handle_debug_command(
    id: HostRequestId,
    cmd: &str,
    editor: &mut Gd<CodeEdit>,
) -> Option<HostResult> {
    match cmd {
        "GodotBreakpoint" => {
            let line = editor.get_caret_line();
            let is_set = editor.is_line_breakpointed(line);
            editor.set_line_as_breakpoint(line, !is_set);
            Some(host_success(id))
        }
        "GodotContinue" | "cont" => debugger_action(id, "debug_continue", None),
        "GodotNext" | "next" => debugger_action(id, "debug_next", None),
        "GodotStepIn" | "stepin" => debugger_action(id, "debug_step", None),
        // Godot's debugger has no step-out action; fall back to continue.
        "GodotStepOut" | "stepout" => debugger_action(
            id,
            "debug_continue",
            Some("step-out not available, used continue"),
        ),
        "GodotPause" | "pause" => debugger_action(id, "debug_break", None),
        _ => None,
    }
}

// ── Dock focus ──────────────────────────────────────────────────────────

/// Focus a Godot editor dock panel from a Vim ex-command.
///
/// Most docks are not directly exposed via `EditorInterface` and must be
/// discovered by walking the scene tree. The approach varies per dock:
/// - `Scene`: class-based search for `SceneTreeDock` (no API accessor)
/// - `Output`: name-based search for the bottom panel
/// - `FileSystem`, `Inspector`: have dedicated API accessors
/// - `Script`: uses `set_main_screen_editor` to switch the main view
fn handle_dock_command(id: HostRequestId, cmd: &str) -> Option<HostResult> {
    match cmd {
        "Scene" => {
            let editor_interface = EditorInterface::singleton();
            if let Some(base) = editor_interface.get_base_control() {
                // COMPAT: Internal editor class, not public Godot API.
                if let Some(dock) =
                    find_node_by_class(&base.clone().upcast(), godot_calls::CLASS_SCENE_TREE_DOCK)
                {
                    make_dock_tab_visible(&dock);
                    grab_focus_on_dock(&dock, id)
                } else {
                    Some(host_failure(id, "E5: Scene dock not found"))
                }
            } else {
                Some(host_failure(id, "E5: Editor base control not available"))
            }
        }
        "Output" => {
            let editor_interface = EditorInterface::singleton();
            if let Some(base) = editor_interface.get_base_control() {
                if let Some(panel) = find_bottom_panel_by_name(&base.clone().upcast(), "Output") {
                    make_dock_tab_visible(&panel.clone().upcast());
                    grab_focus_on_dock(&panel.upcast(), id)
                } else {
                    Some(host_failure(id, "E5: Output panel not found"))
                }
            } else {
                Some(host_failure(id, "E5: Editor base control not available"))
            }
        }
        "FileSystem" => {
            let editor_interface = EditorInterface::singleton();
            if let Some(panel) = editor_interface.get_file_system_dock() {
                let root = panel.upcast::<Node>();
                grab_focus_on_dock(&root, id)
            } else {
                Some(host_failure(id, "E5: FileSystem dock not found"))
            }
        }
        "Inspector" => {
            let editor_interface = EditorInterface::singleton();
            if let Some(panel) = editor_interface.get_inspector() {
                let mut control = panel.upcast::<Control>();
                control.grab_focus();
                Some(host_success(id))
            } else {
                Some(host_failure(id, "E5: Inspector dock not found"))
            }
        }
        "Script" => {
            let mut editor_interface = EditorInterface::singleton();
            editor_interface.set_main_screen_editor(&GString::from("Script"));
            Some(host_success(id))
        }
        _ => None,
    }
}

fn find_node_by_class(root: &Gd<Node>, class_name: &str) -> Option<Gd<Node>> {
    const MAX_DEPTH: u32 = 14;
    find_descendant_by(root, MAX_DEPTH, &|node| {
        node.is_class(class_name).then(|| node.clone())
    })
}

fn find_bottom_panel_by_name(root: &Gd<Node>, name: &str) -> Option<Gd<Control>> {
    const MAX_DEPTH: u32 = 14;
    find_descendant_by(root, MAX_DEPTH, &|node| {
        if node.get_name().to_string().contains(name) {
            node.clone().try_cast::<Control>().ok()
        } else {
            None
        }
    })
}

/// Ensure a dock's tab is selected in its parent TabContainer.
///
/// Godot's dock layout groups multiple docks into shared TabContainers (e.g.,
/// Scene + Import share a slot). When focusing a dock that is behind another tab,
/// we must first select the correct tab or `grab_focus` will silently fail on
/// an invisible control.
fn make_dock_tab_visible(node: &Gd<Node>) {
    use godot::classes::TabContainer;

    let mut current = node.get_parent();
    while let Some(parent) = current {
        if parent.is_class("TabContainer") {
            if let Ok(mut tab_container) = parent.clone().try_cast::<TabContainer>() {
                for i in 0..tab_container.get_tab_count() {
                    if let Some(child) = tab_container.get_tab_control(i) {
                        if is_ancestor_of(&child.clone().upcast(), node) {
                            tab_container.set_current_tab(i);
                            return;
                        }
                    }
                }
            }
            return;
        }
        current = parent.get_parent();
    }
}

fn is_ancestor_of(ancestor: &Gd<Node>, node: &Gd<Node>) -> bool {
    if ancestor.instance_id() == node.instance_id() {
        return true;
    }
    let mut current = node.get_parent();
    while let Some(parent) = current {
        if parent.instance_id() == ancestor.instance_id() {
            return true;
        }
        current = parent.get_parent();
    }
    false
}

// ── Save commands ───────────────────────────────────────────────────────

/// Godot-specific save variants that go beyond Vim's `:w`.
///
/// `:save` writes the current script (equivalent to `:w` without a path).
/// `:saveall` and `:savescene` use Godot's bulk-save APIs that have no Vim
/// equivalent — they save all open scenes/scripts in a single operation.
fn handle_save_command(
    id: HostRequestId,
    cmd: &str,
    editor: &mut Gd<CodeEdit>,
) -> Option<HostResult> {
    match cmd {
        // Scope restriction doesn't apply — `:save` always writes to the script's
        // own res:// path, never to a user-supplied external path.
        "save" => {
            let mut host = super::GodotEditorHost(editor);
            Some(super::file::handle_write_file(
                id,
                &mut host,
                None,
                ForceOverride::Normal,
                crate::settings::FileAccessScope::Unrestricted,
            ))
        }
        "saveall" => {
            let mut editor_interface = EditorInterface::singleton();
            editor_interface.save_all_scenes();
            Some(host_success(id))
        }
        "savescene" => {
            let mut editor_interface = EditorInterface::singleton();
            let err = editor_interface.save_scene();
            if err == godot::global::Error::OK {
                Some(host_success(id))
            } else {
                Some(host_failure(
                    id,
                    format!("E514: Failed to save scene: {err:?}"),
                ))
            }
        }
        _ => None,
    }
}

/// Returns the complete list of custom Godot commands available for
/// `:ListActions` filtering and tab-completion.
pub(super) const fn list_all_commands() -> &'static [&'static str] {
    &[
        "run",
        "play",
        "runcurrent",
        "playcurrent",
        "stop",
        "zen",
        "unzen",
        "save",
        "saveall",
        "savescene",
        "GodotBreakpoint",
        "GodotContinue",
        "cont",
        "GodotNext",
        "next",
        "GodotStepIn",
        "stepin",
        "GodotStepOut",
        "stepout",
        "GodotPause",
        "pause",
        "Scene",
        "FileSystem",
        "Inspector",
        "Output",
        "Script",
    ]
}

// ── Dispatch ────────────────────────────────────────────────────────────

/// Dispatch a custom ex-command through all Godot-specific handler chains.
///
/// Chain order matters: scene commands are checked before debug commands, etc.
/// If no chain recognizes the command, returns Vim-standard `E492`.
pub(super) fn handle_custom_ex_command(
    id: HostRequestId,
    command: &str,
    editor: &mut Gd<CodeEdit>,
) -> HostResult {
    let cmd = command.trim();
    log::debug!("custom_ex_command: {}", cmd);

    if let Some(result) = handle_scene_command(id, cmd) {
        return result;
    }
    if let Some(result) = handle_editor_state_command(id, cmd) {
        return result;
    }
    if let Some(result) = handle_debug_command(id, cmd, editor) {
        return result;
    }
    if let Some(result) = handle_dock_command(id, cmd) {
        return result;
    }
    if let Some(result) = handle_save_command(id, cmd, editor) {
        return result;
    }
    log::debug!("custom_ex_command: unrecognized {}", cmd);
    host_failure(id, format!("E492: Not an editor command: {cmd}"))
}
