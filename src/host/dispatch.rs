//! Routes [`HostRequest`] variants to handler modules (file I/O, clipboard,
//! buffer navigation, shell commands, custom Godot commands) with security
//! policy enforcement.

use compact_str::CompactString;
use godot::classes::CodeEdit;
use godot::prelude::*;
use vim_core::execution::{HostRequest, HostRequestId, HostResult};

use super::{host_failure, host_success};
use crate::settings::{FileAccessScope, ProjectVimrc, ShellExecution};
use crate::types::ForceOverride;

/// Security policy governing dangerous host operations.
///
/// Extracted from EditorSettings at call time and passed by value, keeping the
/// host layer decoupled from Godot's settings API. Each field gates a different
/// category of side effects.
#[derive(Debug, Clone, Copy)]
pub(crate) struct SecurityPolicy {
    pub(crate) shell_execution: ShellExecution,
    pub(crate) file_access_scope: FileAccessScope,
    /// Controls how project-level vimrc files are treated when sourced.
    ///
    /// `ProjectVimrc::Sandbox` strips dangerous patterns (`:!` commands);
    /// `ProjectVimrc::Disabled` prevents sourcing entirely. Consumed by
    /// `controller/host_bridge.rs` after `ReadConfigFile` returns — dispatch
    /// returns raw file text and the controller owns sandboxing responsibility.
    pub(crate) project_vimrc: ProjectVimrc,
}

/// Gate for shell execution — enforced before `:!cmd` and `:{range}!cmd`.
///
/// Returns `Err(HostResult::Failure)` with `E145` if shell execution is disabled,
/// preventing arbitrary command execution from within the editor.
fn require_shell_enabled(id: HostRequestId, policy: &SecurityPolicy) -> Result<(), HostResult> {
    if policy.shell_execution == ShellExecution::Disabled {
        log::warn!("Shell execution blocked by security policy");
        Err(host_failure(
            id,
            "E145: Shell commands are disabled (set security/shell_execution to Enabled in Editor Settings)",
        ))
    } else {
        Ok(())
    }
}

/// Shared evaluator for `EvaluateExpression` and `EvaluateMapping` host requests.
fn eval_to_host_result(id: HostRequestId, expr: &str, mode_str: &str) -> HostResult {
    match super::eval::eval_simple_expression(expr, mode_str) {
        Ok(value) => HostResult::Data {
            id,
            data: CompactString::from(value.as_ref()),
            offset: None,
        },
        Err(e) => host_failure(id, e),
    }
}

/// Central dispatch: routes each `HostRequest` variant to its handler module.
///
/// This is the single entry point for all host request fulfillment. The engine
/// emits host requests when it needs side effects (file I/O, clipboard, shell)
/// that only the editor shell can provide. Security policy is enforced here
/// (shell execution, file scope) before delegating to handler modules.
pub(crate) fn execute(
    request: &HostRequest,
    editor: &mut Gd<CodeEdit>,
    policy: &SecurityPolicy,
    mode_str: &str,
) -> HostResult {
    log::debug!("host::execute: {:?}", request.kind());

    // GodotEditorHost is created per-branch rather than upfront because some
    // branches (e.g., ReindentRange, RequestCompletion) need the raw
    // `&mut Gd<CodeEdit>` directly — wrapping it in GodotEditorHost would
    // consume the mutable borrow.
    match request {
        HostRequest::WriteFile {
            meta: _,
            path,
            force,
        } => {
            let mut host = super::GodotEditorHost(editor);
            super::file::handle_write_file(
                request.id(),
                &mut host,
                path.as_deref(),
                ForceOverride::from(*force),
                policy.file_access_scope,
            )
        }

        HostRequest::Quit { meta: _, force } => {
            let mut host = super::GodotEditorHost(editor);
            super::file::handle_quit(request.id(), &mut host, ForceOverride::from(*force))
        }

        HostRequest::WriteQuit { meta: _, force } => {
            let mut host = super::GodotEditorHost(editor);
            super::file::handle_write_quit(request.id(), &mut host, ForceOverride::from(*force))
        }

        HostRequest::EditFile {
            meta: _,
            path,
            force,
        } => {
            let mut host = super::GodotEditorHost(editor);
            super::file::handle_edit_file(
                request.id(),
                &mut host,
                path.as_str(),
                ForceOverride::from(*force),
                policy.file_access_scope,
            )
        }

        HostRequest::ReadFile {
            meta: _,
            path,
            after_line,
        } => super::file::handle_read_file(
            request.id(),
            path.as_str(),
            *after_line,
            policy.file_access_scope,
        ),

        HostRequest::FilterDocumentRange {
            meta: _,
            range: _,
            motion_type: _,
            input_text,
            command,
        } => {
            if let Err(result) = require_shell_enabled(request.id(), policy) {
                return result;
            }
            super::external::handle_filter(
                request.id(),
                input_text.as_str(),
                command.as_str(),
            )
        }

        HostRequest::ReindentRange {
            meta: _,
            range,
            motion_type: _,
            input_text,
        } => super::external::handle_reindent(
            request.id(),
            editor,
            input_text.as_str(),
            range,
        ),

        HostRequest::ReadClipboard {
            meta: _,
            cursor_offset: _,
        } => super::clipboard::handle_read_clipboard(request.id()),

        HostRequest::ExternalCommand { meta: _, command } => {
            if let Err(result) = require_shell_enabled(request.id(), policy) {
                return result;
            }
            super::external::handle_external_command(request.id(), command.as_str())
        }

        HostRequest::CustomExCommand { meta: _, command } => {
            super::custom_commands::handle_custom_ex_command(
                request.id(),
                command.as_str(),
                editor,
            )
        }

        HostRequest::SyncCommandLine { meta: _, .. } => {
            // No-op: command-line state is pulled from the engine via ui_snapshot()
            // (state-snapshot pattern), so there is nothing to push to the host.
            log::trace!("host::execute: SyncCommandLine no-op (state-snapshot pattern)");
            host_success(request.id())
        }

        HostRequest::SwitchBuffer { meta: _, number } => {
            super::buffer::handle_goto_buffer(request.id(), *number as usize)
        }

        HostRequest::BufferNext { meta: _, count }
        | HostRequest::TabNext { meta: _, count } => {
            super::buffer::handle_switch_buffer(request.id(), crate::bridge::codec::u32_to_i32_sat(*count))
        }

        HostRequest::BufferPrev { meta: _, count }
        | HostRequest::TabPrev { meta: _, count } => {
            super::buffer::handle_switch_buffer(request.id(), -crate::bridge::codec::u32_to_i32_sat(*count))
        }

        HostRequest::BufferFirst { meta: _ } => {
            super::buffer::handle_goto_buffer(request.id(), 1)
        }

        HostRequest::BufferLast { meta: _ } => {
            super::buffer::handle_goto_last_buffer(request.id())
        }

        HostRequest::TabClose { meta: _, force } => {
            let mut host = super::GodotEditorHost(editor);
            super::file::handle_quit(request.id(), &mut host, ForceOverride::from(*force))
        }

        HostRequest::TabNew { meta: _, path } => {
            if let Some(p) = path {
                let mut host = super::GodotEditorHost(editor);
                super::file::handle_edit_file(
                    request.id(),
                    &mut host,
                    p.as_str(),
                    ForceOverride::Normal,
                    policy.file_access_scope,
                )
            } else {
                host_failure(request.id(), "E471: Argument required")
            }
        }

        HostRequest::BufferList { meta: _ } => {
            super::buffer::handle_buffer_list(request.id())
        }

        HostRequest::ReadConfigFile { meta: _, path } => {
            if let Err(e) = super::file::validate_path_scope(path.as_str(), policy.file_access_scope) {
                return host_failure(request.id(), e.to_string());
            }
            let gpath = GString::from(path.as_str());
            match godot::classes::FileAccess::open(&gpath, godot::classes::file_access::ModeFlags::READ) {
                Some(fa) => {
                    // Raw text returned here. Sandbox filtering (stripping
                    // dangerous :! commands etc.) is applied by host_bridge
                    // before the result reaches the engine.
                    let text = fa.get_as_text().to_string();
                    HostResult::Data {
                        id: request.id(),
                        data: CompactString::from(text),
                        offset: None,
                    }
                }
                None => host_failure(request.id(), format!("E484: Can't open file {}", path)),
            }
        }

        HostRequest::EvaluateExpression { meta: _, expression } => {
            eval_to_host_result(request.id(), expression.as_str(), mode_str)
        }

        HostRequest::EvaluateMapping { meta: _, expression, .. } => {
            eval_to_host_result(request.id(), expression.as_str(), mode_str)
        }

        HostRequest::RequestCompletion { .. } => {
            if editor.is_code_completion_enabled() {
                editor.request_code_completion_ex().force(false).done();
            }
            host_success(request.id())
        }

        HostRequest::ShowMessageHistory { meta: _, entries } => {
            let text = entries.iter().map(|e| e.text.as_str()).collect::<Vec<_>>().join("\n");
            HostResult::Success {
                id: request.id(),
                message: Some(CompactString::from(text)),
            }
        }

        HostRequest::JumpToBuffer { meta: _, buffer_id, offset: jump_offset } => {
            super::buffer::handle_jump_to_buffer(
                request.id(),
                buffer_id.get(),
                jump_offset.get(),
                buffer_id,
            )
        }

        HostRequest::ListActions { meta: _, filter } => {
            let all_commands = super::custom_commands::list_all_commands();
            let filtered: Vec<&&str> = match filter {
                Some(f) if !f.is_empty() => {
                    all_commands.iter().filter(|c| c.starts_with(f.as_str())).collect()
                }
                _ => all_commands.iter().collect(),
            };
            let text = filtered.iter().map(|s| **s).collect::<Vec<_>>().join("\n");
            HostResult::Success {
                id: request.id(),
                message: Some(CompactString::from(text)),
            }
        }

        _ => {
            let kind = format!("{:?}", request.kind());
            log::warn!("Unhandled host request variant: {kind}");
            host_failure(request.id(), format!("Unhandled host request: {kind}"))
        }
    }
}
