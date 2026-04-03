//! Host request dispatch: routes [`HostRequest`]s from the engine to the
//! Godot host layer, intercepts controller-level commands (`:mappings`,
//! `:source`, `:vimdebug`, `:perf`), and recurses on sub-requests
//! produced by completion responses.
//!
//! Recursion is depth-limited by [`super::MAX_HOST_DEPTH`] to prevent
//! unbounded nesting from `:source` chains or adversarial config files.

use compact_str::CompactString;
use godot::classes::CodeEdit;
use godot::prelude::*;
use vim_core::execution::{HostRequest, HostResult};

use super::context::ProcessContext;
use super::vimdebug;
use super::MAX_HOST_DEPTH;

/// Map engine `Mode` to the single-char string that Vim's `mode()` function returns.
/// Passed to host requests that need to report the current mode.
fn mode_to_vim_string(mode: vim_core::primitives::Mode) -> &'static str {
    use vim_core::primitives::{Mode, VisualType};
    match mode {
        Mode::Normal => "n",
        Mode::Insert => "i",
        Mode::Visual(VisualType::Char) => "v",
        Mode::Visual(VisualType::Line) => "V",
        Mode::Visual(VisualType::Block) => "\x16", // Ctrl-V (ASCII 22) — Vim's mode() for Visual Block
        Mode::Select(VisualType::Char) => "s",
        Mode::Select(VisualType::Line) => "S",
        Mode::Select(VisualType::Block) => "\x13", // Ctrl-S (ASCII 19) — Vim's mode() for Select Block
        Mode::Replace => "R",
        Mode::VirtualReplace => "Rv",
        Mode::CommandLine => "c",
        other => {
            log::warn!("mode_to_vim_string: unknown Mode variant {} — defaulting to Normal", other);
            "n"
        }
    }
}

/// Sandbox-filter a `ReadConfigFile` result at the controller boundary.
///
/// The host layer returns raw file bytes; sandboxing is the controller's
/// responsibility (consistent with `source_config_from_disk`). Applied
/// here, before the result reaches the engine, so the engine never sees
/// unsandboxed config text.
fn sandbox_config_result(
    request: &HostRequest,
    result: HostResult,
    sandbox: bool,
) -> HostResult {
    if !sandbox {
        return result;
    }
    let HostRequest::ReadConfigFile { .. } = request else {
        return result;
    };
    let HostResult::Data { id, data, offset } = result else {
        return result;
    };
    let sandboxed = crate::config::sandbox::sandbox_config_text(data.as_str());
    HostResult::Data {
        id,
        data: CompactString::from(sandboxed),
        offset,
    }
}

fn result_message(result: &HostResult) -> Option<&str> {
    match result {
        HostResult::Success { message, .. } => message.as_deref(),
        HostResult::Failure { error, .. } => Some(error.as_str()),
        _ => None,
    }
}

impl ProcessContext<'_> {
    pub(super) fn handle_host_requests(
        &mut self,
        requests: Vec<HostRequest>,
        editor: &mut Gd<CodeEdit>,
        depth: u32,
    ) {
        if requests.is_empty() {
            return;
        }
        if depth > MAX_HOST_DEPTH {
            log::error!(
                "Host request completion depth exceeded {} — completing {} request(s) \
                 with Failure to unblock the engine pipeline (first={:?})",
                MAX_HOST_DEPTH,
                requests.len(),
                requests.first().map(HostRequest::kind),
            );
            self.state.globals_mut().set_error(
                "E223: Host request recursion limit exceeded — command aborted",
            );
            // Each dropped request must be completed with Failure so the
            // engine's pending-request map doesn't leak entries.
            for request in &requests {
                let failure = HostResult::Failure {
                    id: request.id(),
                    error: CompactString::from(
                        "host request depth limit exceeded — dropped by controller",
                    ),
                };
                let _ = self.engine.complete_host_request(&failure);
            }
            return;
        }

        for request in &requests {
            log::debug!("host_request: {:?} (depth={})", request.kind(), depth);

            // Controller-level commands (`:mappings`, `:vimdebug`, etc.) set
            // a pending UI action and return Success without hitting the host.
            if let Some(result) = self.try_intercept_controller_command(request, editor.instance_id()) {
                let mut response = self.engine.complete_host_request(&result);
                self.drain_sub_response(&mut response, editor, depth);
                continue;
            }

            let mode_str = mode_to_vim_string(self.engine.mode());
            let result = crate::host::execute(request, editor, self.security_policy, mode_str, self.clipboard);

            let result = sandbox_config_result(request, result, self.security_policy.project_vimrc == crate::settings::ProjectVimrc::Sandbox);

            if let Some(msg) = result_message(&result) {
                log::debug!("Host result: {}", msg);
            }

            let mut response = self.engine.complete_host_request(&result);
            self.drain_sub_response(&mut response, editor, depth);
        }
    }

    /// Drain sub-effects and sub-requests that a host request completion
    /// may have triggered (e.g., `:source` producing `ShowMessage` effects).
    fn drain_sub_response(
        &mut self,
        response: &mut vim_core::execution::Response,
        editor: &mut Gd<CodeEdit>,
        depth: u32,
    ) {
        let sub_effects = response.take_effects();
        if !sub_effects.is_empty() {
            let has_text_mutation = sub_effects.iter().any(|e| e.is_text_mutation());
            // Text mutations invalidate the cache, so we must fetch fresh.
            // Otherwise, reuse the cache to avoid an FFI round-trip.
            let text = if has_text_mutation {
                editor.get_text().to_string()
            } else {
                let editor_id = editor.instance_id();
                match self.persistent_text.take() {
                    Some((id, t)) if id == editor_id => t,
                    _ => editor.get_text().to_string(),
                }
            };
            self.apply_effects(sub_effects, editor, crate::effects::dispatch::AutoBraceMode::Ineligible, &text, None);
        }
        let sub_requests = response.take_host_requests();
        if !sub_requests.is_empty() {
            self.handle_host_requests(sub_requests, editor, depth + 1);
        }
    }

    /// Intercept `CustomExCommand`s that need controller-level action
    /// (UI dialogs, config reload, debug modes) rather than Godot API calls.
    fn try_intercept_controller_command(&mut self, request: &HostRequest, editor_id: InstanceId) -> Option<HostResult> {
        let HostRequest::CustomExCommand { meta: _, command } = request else {
            return None;
        };

        let success = |msg: &str| -> Option<HostResult> {
            Some(HostResult::Success {
                id: request.id(),
                message: Some(CompactString::from(msg)),
            })
        };
        let success_silent = || -> Option<HostResult> {
            Some(HostResult::Success {
                id: request.id(),
                message: None,
            })
        };
        let vimdebug_set = |vd: &mut vimdebug::VimdebugState, mode: vimdebug::VimdebugMode, msg: &str| -> Option<HostResult> {
            vd.set_mode(mode);
            Some(HostResult::Success {
                id: request.id(),
                message: Some(CompactString::from(msg)),
            })
        };

        let cmd = command.as_str().trim();
        match cmd {
            "mappings" => {
                *self.pending_ui_action = Some(super::PendingUiAction::OpenMappingDialog);
                success_silent()
            }
            "source" => {
                *self.pending_ui_action = Some(super::PendingUiAction::SourceConfigFile);
                success_silent()
            }
            "perf" => success(&self.perf.format_report()),
            "perf reset" => {
                self.perf.reset();
                success(":perf reset")
            }
            "undotree" => {
                let msg = self.state.buffer(editor_id).undo_tree()
                    .map_or_else(
                        || "No undo tree for this buffer".to_owned(),
                        |tree| tree.format_tree(),
                    );
                success(&msg)
            }
            "vimdebug" | "vimdebug on" => {
                use vimdebug::VimdebugMode;
                if self.vimdebug.mode() == VimdebugMode::Off {
                    vimdebug_set(self.vimdebug, VimdebugMode::Watch, ":vimdebug ON (watch)")
                } else {
                    vimdebug_set(self.vimdebug, VimdebugMode::Off, ":vimdebug OFF")
                }
            }
            "vimdebug off" => vimdebug_set(self.vimdebug, vimdebug::VimdebugMode::Off, ":vimdebug OFF"),
            "vimdebug watch" => vimdebug_set(self.vimdebug, vimdebug::VimdebugMode::Watch, ":vimdebug ON (watch)"),
            "vimdebug step" => vimdebug_set(self.vimdebug, vimdebug::VimdebugMode::Step, ":vimdebug ON (step)"),
            _ => None,
        }
    }
}
