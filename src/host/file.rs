//! File I/O host request handlers: `:w`, `:q`, `:wq`, `:e`, `:r`.
//!
//! All editor-facing handlers accept `impl EditorHost` for testability without
//! a running Godot instance. `handle_read_file` is standalone (no editor needed).
//!
//! Godot paths (`res://`, `user://`) are routed through `ResourceSaver`/`FileAccess`;
//! external filesystem paths use `std::fs` directly. Security: the `FileAccessScope`
//! policy restricts which path types are permitted, and symlink writes require `:w!`.

use compact_str::CompactString;
use vim_core::execution::{HostRequestId, HostResult};

use super::editor_host::EditorHost;
use super::error::HostError;
use super::{host_failure, host_success};
use crate::settings::FileAccessScope;
use crate::types::ForceOverride;

const MAX_READ_FILE_SIZE: usize = 10 * 1024 * 1024; // 10 MB

/// Validate that a path is a well-formed Godot virtual path (`res://` or `user://`).
///
/// Security-critical: rejects path traversal (`..`), self-references (`.`),
/// URL-encoded sequences (`%`), backslash separators, and null bytes. Any of
/// these could escape the virtual filesystem sandbox when Godot resolves the path.
pub(super) fn is_godot_path(path: &str) -> bool {
    let virtual_path = if let Some(rest) = path.strip_prefix("res://") {
        rest
    } else if let Some(rest) = path.strip_prefix("user://") {
        rest
    } else {
        return false;
    };
    if virtual_path.contains('%') || virtual_path.contains('\\') || virtual_path.contains('\0') {
        return false;
    }
    !virtual_path
        .split('/')
        .any(|segment| segment == ".." || segment == ".")
}

/// Enforce file access scope: `ProjectOnly` restricts to `res://`/`user://` paths.
pub(super) fn validate_path_scope(path: &str, scope: FileAccessScope) -> Result<(), HostError> {
    if scope == FileAccessScope::Unrestricted {
        return Ok(());
    }
    if is_godot_path(path) {
        return Ok(());
    }
    Err(HostError::CantOpenFile {
        path: CompactString::from(path),
        detail: Some(CompactString::from(
            "Access denied — path is outside the project. \
             Set security/file_access_scope to Unrestricted in Editor Settings to allow.",
        )),
    })
}

/// Pre-read size check for filesystem paths (not Godot virtual paths, which
/// are checked via `FileAccess::get_length()` in `EditorHost::read_file`).
pub(super) fn check_fs_file_size(path: &str) -> Result<(), HostError> {
    match std::fs::metadata(path) {
        Ok(meta) => {
            let size = usize::try_from(meta.len()).unwrap_or(usize::MAX);
            if size > MAX_READ_FILE_SIZE {
                Err(HostError::CantOpenFile {
                    path: CompactString::from(path),
                    detail: Some(CompactString::from(format!(
                        "File too large (>10MB): {} bytes",
                        size
                    ))),
                })
            } else {
                Ok(())
            }
        }
        // Metadata failure is fine — let the subsequent read produce the real error.
        Err(_) => Ok(()),
    }
}

/// `:w[!] [path]`
///
/// Two write paths depending on the target:
/// - **Godot paths** (`res://`, `user://`): saved via `ResourceSaver`. The `!`
///   flag has no effect (Godot's resource system has no readonly protection).
/// - **External filesystem paths**: saved via `std::fs::write`. Symlink writes
///   are blocked without `!` (symlink attack prevention). The `!` flag does NOT
///   override filesystem permissions (would need platform-specific logic).
pub(super) fn handle_write_file(
    id: HostRequestId,
    editor: &mut impl EditorHost,
    path: Option<&str>,
    force: ForceOverride,
    scope: FileAccessScope,
) -> HostResult {
    log::debug!(
        "file::write: path={} force={}",
        path.unwrap_or("<none>"),
        force.is_force()
    );
    if let Some(p) = path {
        if p.is_empty() {
            return host_failure(id, "E32: No file name");
        }
        if let Err(e) = validate_path_scope(p, scope) {
            return host_failure(id, e.to_string());
        }
        if !is_godot_path(p) {
            // Symlink attack prevention: symlink_metadata (lstat) does NOT follow
            // the link, making is_symlink() reliable. `:w!` overrides this check.
            if let Ok(meta) = std::fs::symlink_metadata(p) {
                if meta.file_type().is_symlink() && !force.is_force() {
                    let err = HostError::SymlinkWrite {
                        path: CompactString::from(p),
                    };
                    return host_failure(id, err.to_string());
                }
            }
            let text = editor.get_text();
            log::info!(
                "file::write_external: path={} force={}",
                p,
                force.is_force()
            );
            return match std::fs::write(p, &text) {
                Ok(()) => {
                    let line_count = text.lines().count();
                    let byte_count = text.len();
                    HostResult::Success {
                        id,
                        message: Some(CompactString::from(format!(
                            "\"{}\" {}L, {}B written",
                            p, line_count, byte_count
                        ))),
                    }
                }
                Err(e) => {
                    let err = HostError::WriteFailed {
                        path: CompactString::from(p),
                        detail: Some(CompactString::from(e.to_string())),
                    };
                    if force.is_force() {
                        log::warn!(
                            "file::write_external: force flag set but permission override \
                             is not supported for external paths; path={} err={e}",
                            p
                        );
                        host_failure(
                            id,
                            format!(
                                "{} \
                                 (force-write cannot override filesystem permissions for \
                                 external paths; use a Godot res:// path to benefit from :w!)",
                                err
                            ),
                        )
                    } else {
                        host_failure(id, err.to_string())
                    }
                }
            };
        }
    }

    // Capture text before saving for accurate line/byte counts in the status message.
    let text_for_counts = editor.get_text();
    match editor.save_script(path) {
        Ok(saved_path) => {
            let line_count = text_for_counts.lines().count();
            let byte_count = text_for_counts.len();
            HostResult::Success {
                id,
                message: Some(CompactString::from(format!(
                    "\"{}\" {}L, {}B written",
                    saved_path, line_count, byte_count
                ))),
            }
        }
        Err(e) => host_failure(id, e.to_string()),
    }
}

/// `:q[!]`
pub(super) fn handle_quit(
    id: HostRequestId,
    editor: &mut impl EditorHost,
    force: ForceOverride,
) -> HostResult {
    log::debug!("file::quit: force={}", force.is_force());
    if !force.is_force() && editor.is_modified() {
        return host_failure(id, "E37: No write since last change (add ! to override)");
    }

    if force.is_force() {
        // Mark clean so Godot's ScriptEditor close pipeline skips its
        // "unsaved changes" dialog.
        editor.tag_saved_version();
    }

    editor.close_tab();

    host_success(id)
}

/// `:wq[!]`
pub(super) fn handle_write_quit(
    id: HostRequestId,
    editor: &mut impl EditorHost,
    force: ForceOverride,
) -> HostResult {
    match editor.save_script(None) {
        Ok(_) => {
            editor.close_tab();
            host_success(id)
        }
        Err(e) if force.is_force() => {
            // `:wq!` = force-write, NOT force-discard. Write failure is still an error.
            host_failure(id, format!("{} (use :q! to discard changes)", e))
        }
        Err(e) => host_failure(id, e.to_string()),
    }
}

/// `:e [path]`
pub(super) fn handle_edit_file(
    id: HostRequestId,
    editor: &mut impl EditorHost,
    path: &str,
    force: ForceOverride,
    scope: FileAccessScope,
) -> HostResult {
    log::debug!("file::edit: path={} force={}", path, force.is_force());
    if let Err(e) = validate_path_scope(path, scope) {
        return host_failure(id, e.to_string());
    }
    let current_path = editor.current_script_path();
    let is_same_file = current_path.as_deref() == Some(path);

    if is_same_file {
        if force.is_force() {
            reload_from_disk(id, editor, path)
        } else {
            let text = editor.get_text();
            let line_count = text.lines().count();
            let byte_count = text.len();
            HostResult::Success {
                id,
                message: Some(CompactString::from(format!(
                    "\"{}\" {}L, {}B",
                    path, line_count, byte_count
                ))),
            }
        }
    } else if !force.is_force() && editor.is_modified() {
        host_failure(id, "E37: No write since last change (add ! to override)")
    } else {
        match editor.open_script(path) {
            Ok(()) => host_success(id),
            Err(e) => host_failure(id, e.to_string()),
        }
    }
}

/// Reload from disk, discarding editor changes. Updates both the Script
/// resource (in-memory) and the CodeEdit buffer to keep them in sync.
fn reload_from_disk(id: HostRequestId, editor: &mut impl EditorHost, path: &str) -> HostResult {
    let text = match editor.read_file(path) {
        Ok(t) => t,
        Err(e) => return host_failure(id, e.to_string()),
    };

    editor.update_script_source(&text);
    editor.set_text(&text);
    editor.tag_saved_version();
    editor.notify_name_changed();

    log::debug!("file::reload: {}", path);
    host_success(id)
}

/// `:r [path]` — read file contents into the buffer after a given line.
///
/// No `EditorHost` needed: returns file text as `HostResult::Data`, and the
/// engine handles the buffer insertion. Routes through Godot `FileAccess` for
/// virtual paths and `std::fs` for filesystem paths.
pub(super) fn handle_read_file(
    id: HostRequestId,
    path: &str,
    after_line: Option<u32>,
    scope: FileAccessScope,
) -> HostResult {
    log::debug!("file::read: path={}", path);
    if let Err(e) = validate_path_scope(path, scope) {
        return host_failure(id, e.to_string());
    }
    let result = if is_godot_path(path) {
        read_via_godot(path)
    } else {
        check_fs_file_size(path).and_then(|_| {
            std::fs::read_to_string(path).map_err(|e| HostError::CantOpenFile {
                path: CompactString::from(path),
                detail: Some(CompactString::from(e.to_string())),
            })
        })
    };

    match result {
        Ok(data) => HostResult::Data {
            id,
            data: CompactString::from(data),
            offset: after_line.map(|l| l as usize), // u32→usize: always safe
        },
        Err(e) => host_failure(id, e.to_string()),
    }
}

fn read_via_godot(path: &str) -> Result<String, HostError> {
    use godot::classes::file_access::ModeFlags;
    use godot::classes::FileAccess;
    use godot::prelude::*;

    let file = FileAccess::open(&GString::from(path), ModeFlags::READ).ok_or_else(|| {
        HostError::CantOpenFile {
            path: CompactString::from(path),
            detail: None,
        }
    })?;
    let length = usize::try_from(file.get_length()).unwrap_or(usize::MAX);
    if length > MAX_READ_FILE_SIZE {
        return Err(HostError::CantOpenFile {
            path: CompactString::from(path),
            detail: Some(CompactString::from(format!(
                "File too large (>10MB): {} bytes",
                length
            ))),
        });
    }
    let text = file.get_as_text().to_string();
    Ok(text)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::editor_host::mock::{MockBufferState, MockEditorHost};
    use crate::settings::FileAccessScope;
    use vim_core::execution::HostRequestId;

    fn test_id() -> HostRequestId {
        HostRequestId::new(1)
    }

    fn failure_msg(result: &HostResult) -> &str {
        match result {
            HostResult::Failure { error, .. } => error.as_str(),
            other => panic!("expected Failure, got {:?}", other),
        }
    }

    fn success_msg(result: &HostResult) -> Option<&str> {
        match result {
            HostResult::Success { message, .. } => message.as_deref(),
            other => panic!("expected Success, got {:?}", other),
        }
    }

    fn assert_success(result: &HostResult) {
        assert!(
            matches!(result, HostResult::Success { .. }),
            "expected Success, got {:?}",
            result
        );
    }

    fn assert_failure(result: &HostResult) {
        assert!(
            matches!(result, HostResult::Failure { .. }),
            "expected Failure, got {:?}",
            result
        );
    }

    #[test]
    fn path_scope_blocks_traversal() {
        assert!(validate_path_scope("../../etc/passwd", FileAccessScope::ProjectOnly).is_err());
    }

    #[test]
    fn path_scope_blocks_absolute_paths() {
        assert!(validate_path_scope("/etc/passwd", FileAccessScope::ProjectOnly).is_err());
    }

    #[test]
    fn path_scope_allows_godot_paths() {
        assert!(validate_path_scope("res://script.gd", FileAccessScope::ProjectOnly).is_ok());
        assert!(validate_path_scope("user://config.vim", FileAccessScope::ProjectOnly).is_ok());
    }

    #[test]
    fn path_scope_unrestricted_allows_all() {
        assert!(validate_path_scope("/etc/passwd", FileAccessScope::Unrestricted).is_ok());
        assert!(validate_path_scope("../../foo", FileAccessScope::Unrestricted).is_ok());
    }

    #[test]
    fn check_fs_file_size_accepts_small_file() {
        let dir = std::env::temp_dir();
        let path = dir.join("godot_vim_test_small.txt");
        std::fs::write(&path, "hello").unwrap();
        let result = check_fs_file_size(path.to_str().unwrap());
        assert!(result.is_ok());
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn check_fs_file_size_nonexistent_file_ok() {
        let result = check_fs_file_size("/tmp/godot_vim_definitely_nonexistent_file.txt");
        assert!(result.is_ok());
    }

    #[test]
    fn max_read_file_size_is_10mb() {
        assert_eq!(MAX_READ_FILE_SIZE, 10 * 1024 * 1024);
    }

    #[test]
    fn is_godot_path_res_basic() {
        assert!(is_godot_path("res://script.gd"));
    }

    #[test]
    fn is_godot_path_user_basic() {
        assert!(is_godot_path("user://config.cfg"));
    }

    #[test]
    fn is_godot_path_nested_directory() {
        assert!(is_godot_path("res://scenes/main/level.tscn"));
    }

    #[test]
    fn is_godot_path_user_nested() {
        assert!(is_godot_path("user://saves/slot1/data.json"));
    }

    #[test]
    fn is_godot_path_res_root() {
        assert!(is_godot_path("res://"));
    }

    #[test]
    fn is_godot_path_user_root() {
        assert!(is_godot_path("user://"));
    }

    #[test]
    fn is_godot_path_double_slash() {
        assert!(is_godot_path("res:///foo"));
    }

    #[test]
    fn is_godot_path_rejects_traversal_simple() {
        assert!(!is_godot_path("res://../etc/passwd"));
    }

    #[test]
    fn is_godot_path_rejects_traversal_nested() {
        assert!(!is_godot_path("res://foo/../../bar"));
    }

    #[test]
    fn is_godot_path_rejects_traversal_at_end() {
        assert!(!is_godot_path("res://foo/.."));
    }

    #[test]
    fn is_godot_path_rejects_user_traversal() {
        assert!(!is_godot_path("user://../secret"));
    }

    #[test]
    fn is_godot_path_rejects_dot_segment() {
        assert!(!is_godot_path("res://./script.gd"));
    }

    #[test]
    fn is_godot_path_rejects_lone_dot() {
        assert!(!is_godot_path("res://."));
    }

    #[test]
    fn is_godot_path_rejects_null_byte() {
        assert!(!is_godot_path("res://foo\0bar"));
    }

    #[test]
    fn is_godot_path_rejects_null_in_user() {
        assert!(!is_godot_path("user://config\0.cfg"));
    }

    #[test]
    fn is_godot_path_rejects_percent_encoding() {
        assert!(!is_godot_path("res://foo%20bar"));
    }

    #[test]
    fn is_godot_path_rejects_percent_encoded_dot_dot() {
        assert!(!is_godot_path("res://%2e%2e/etc/passwd"));
    }

    #[test]
    fn is_godot_path_rejects_backslash() {
        assert!(!is_godot_path("res://foo\\bar"));
    }

    #[test]
    fn is_godot_path_rejects_backslash_traversal() {
        assert!(!is_godot_path("res://foo\\..\\bar"));
    }

    #[test]
    fn is_godot_path_rejects_absolute_unix() {
        assert!(!is_godot_path("/etc/passwd"));
    }

    #[test]
    fn is_godot_path_rejects_windows_path() {
        assert!(!is_godot_path("C:\\Windows\\System32"));
    }

    #[test]
    fn is_godot_path_rejects_relative_path() {
        assert!(!is_godot_path("relative/path"));
    }

    #[test]
    fn is_godot_path_rejects_empty_string() {
        assert!(!is_godot_path(""));
    }

    #[test]
    fn is_godot_path_rejects_just_res_colon() {
        assert!(!is_godot_path("res:"));
    }

    #[test]
    fn is_godot_path_rejects_res_single_slash() {
        assert!(!is_godot_path("res:/script.gd"));
    }

    #[test]
    fn is_godot_path_allows_dotfile() {
        assert!(is_godot_path("res://.hidden"));
    }

    #[test]
    fn is_godot_path_allows_dotdot_in_filename() {
        assert!(is_godot_path("res://..secret"));
    }

    #[test]
    fn is_godot_path_allows_multiple_extensions() {
        assert!(is_godot_path("res://archive.tar.gz"));
    }

    #[test]
    fn handle_write_file_empty_path_returns_e32() {
        let mut editor = MockEditorHost::new("hello\nworld", Some("res://script.gd"));
        let result = handle_write_file(
            test_id(),
            &mut editor,
            Some(""),
            ForceOverride::Normal,
            FileAccessScope::Unrestricted,
        );
        assert_failure(&result);
        assert!(failure_msg(&result).contains("E32"));
    }

    #[test]
    fn handle_write_file_no_path_no_script_returns_e32() {
        let mut editor = MockEditorHost::new("content", None);
        let result = handle_write_file(
            test_id(),
            &mut editor,
            None,
            ForceOverride::Normal,
            FileAccessScope::Unrestricted,
        );
        assert_failure(&result);
        assert!(failure_msg(&result).contains("E32"));
    }

    #[test]
    fn handle_write_file_no_path_with_script_saves_via_script() {
        let mut editor = MockEditorHost::new("line1\nline2\nline3", Some("res://main.gd"));
        let result = handle_write_file(
            test_id(),
            &mut editor,
            None,
            ForceOverride::Normal,
            FileAccessScope::ProjectOnly,
        );
        assert_success(&result);
        let msg = success_msg(&result).unwrap();
        assert!(msg.contains("res://main.gd"), "message={msg}");
        assert!(msg.contains("3L"), "message={msg}");
        assert!(msg.contains("written"), "message={msg}");
        assert!(editor.save_called);
    }

    #[test]
    fn handle_write_file_external_path_scope_denied() {
        let mut editor = MockEditorHost::new("content", Some("res://script.gd"));
        let result = handle_write_file(
            test_id(),
            &mut editor,
            Some("/tmp/external.txt"),
            ForceOverride::Normal,
            FileAccessScope::ProjectOnly,
        );
        assert_failure(&result);
        assert!(failure_msg(&result).contains("E484"));
        assert!(failure_msg(&result).contains("Access denied"));
    }

    #[test]
    fn handle_write_file_external_path_unrestricted_writes_file() {
        let dir = std::env::temp_dir();
        let path = dir.join("godot_vim_editor_host_test_write.txt");
        let path_str = path.to_str().unwrap();

        std::fs::remove_file(&path).ok();

        let mut editor = MockEditorHost::new("test content", Some("res://script.gd"));
        let result = handle_write_file(
            test_id(),
            &mut editor,
            Some(path_str),
            ForceOverride::Normal,
            FileAccessScope::Unrestricted,
        );
        assert_success(&result);

        let written = std::fs::read_to_string(&path).unwrap();
        assert_eq!(written, "test content");
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn handle_write_file_external_symlink_no_force_returns_e166() {
        let dir = std::env::temp_dir();
        let target = dir.join("godot_vim_editor_host_test_symlink_target.txt");
        let link = dir.join("godot_vim_editor_host_test_symlink_link.txt");

        std::fs::write(&target, "target").unwrap();
        std::fs::remove_file(&link).ok();

        #[cfg(unix)]
        std::os::unix::fs::symlink(&target, &link).unwrap();
        #[cfg(not(unix))]
        {
            // Skip on non-Unix platforms.
            std::fs::remove_file(&target).ok();
            return;
        }

        let link_str = link.to_str().unwrap();
        let mut editor = MockEditorHost::new("new content", Some("res://script.gd"));
        let result = handle_write_file(
            test_id(),
            &mut editor,
            Some(link_str),
            ForceOverride::Normal,
            FileAccessScope::Unrestricted,
        );
        assert_failure(&result);
        assert!(
            failure_msg(&result).contains("E166"),
            "msg={}",
            failure_msg(&result)
        );

        std::fs::remove_file(&link).ok();
        std::fs::remove_file(&target).ok();
    }

    #[test]
    fn handle_write_file_external_symlink_with_force_writes_through() {
        let dir = std::env::temp_dir();
        let target = dir.join("godot_vim_editor_host_test_symlink_force_target.txt");
        let link = dir.join("godot_vim_editor_host_test_symlink_force_link.txt");

        std::fs::write(&target, "old target").unwrap();
        std::fs::remove_file(&link).ok();

        #[cfg(unix)]
        std::os::unix::fs::symlink(&target, &link).unwrap();
        #[cfg(not(unix))]
        {
            std::fs::remove_file(&target).ok();
            return;
        }

        let link_str = link.to_str().unwrap();
        let mut editor = MockEditorHost::new("forced write", Some("res://script.gd"));
        let result = handle_write_file(
            test_id(),
            &mut editor,
            Some(link_str),
            ForceOverride::Force,
            FileAccessScope::Unrestricted,
        );
        assert_success(&result);

        let written = std::fs::read_to_string(&target).unwrap();
        assert_eq!(written, "forced write");

        std::fs::remove_file(&link).ok();
        std::fs::remove_file(&target).ok();
    }

    #[test]
    fn handle_write_file_godot_path_saves_via_script() {
        let mut editor = MockEditorHost::new("gd content", Some("res://main.gd"));
        let result = handle_write_file(
            test_id(),
            &mut editor,
            Some("res://main.gd"),
            ForceOverride::Normal,
            FileAccessScope::ProjectOnly,
        );
        assert_success(&result);
        let msg = success_msg(&result).unwrap();
        assert!(msg.contains("res://main.gd"), "msg={msg}");
        assert!(msg.contains("written"), "msg={msg}");
    }

    #[test]
    fn handle_write_file_save_failure_returns_error() {
        let mut editor = MockEditorHost::new("content", Some("res://broken.gd"));
        editor.save_result = Some(Err(HostError::WriteFailed {
            path: CompactString::from("res://broken.gd"),
            detail: Some(CompactString::from("FAILED")),
        }));
        let result = handle_write_file(
            test_id(),
            &mut editor,
            None,
            ForceOverride::Normal,
            FileAccessScope::ProjectOnly,
        );
        assert_failure(&result);
        assert!(failure_msg(&result).contains("E514"));
    }

    #[test]
    fn handle_write_file_external_write_failure_no_force() {
        let mut editor = MockEditorHost::new("content", Some("res://script.gd"));
        let result = handle_write_file(
            test_id(),
            &mut editor,
            Some("/nonexistent_dir_godot_vim_test/file.txt"),
            ForceOverride::Normal,
            FileAccessScope::Unrestricted,
        );
        assert_failure(&result);
        assert!(
            failure_msg(&result).contains("E514"),
            "msg={}",
            failure_msg(&result)
        );
        assert!(!failure_msg(&result).contains("force-write"));
    }

    #[test]
    fn handle_write_file_external_write_failure_with_force() {
        let mut editor = MockEditorHost::new("content", Some("res://script.gd"));
        let result = handle_write_file(
            test_id(),
            &mut editor,
            Some("/nonexistent_dir_godot_vim_test/file.txt"),
            ForceOverride::Force,
            FileAccessScope::Unrestricted,
        );
        assert_failure(&result);
        assert!(
            failure_msg(&result).contains("E514"),
            "msg={}",
            failure_msg(&result)
        );
        assert!(
            failure_msg(&result).contains("force-write"),
            "msg={}",
            failure_msg(&result)
        );
    }

    #[test]
    fn handle_quit_unsaved_no_force_returns_e37() {
        let mut editor = MockEditorHost::new("unsaved", Some("res://script.gd"));
        editor.buffer_state = MockBufferState::Modified;
        let result = handle_quit(test_id(), &mut editor, ForceOverride::Normal);
        assert_failure(&result);
        assert!(failure_msg(&result).contains("E37"));
        assert!(
            !matches!(editor.buffer_state, MockBufferState::Closed),
            "tab should NOT be closed"
        );
    }

    #[test]
    fn handle_quit_unsaved_with_force_closes() {
        let mut editor = MockEditorHost::new("unsaved", Some("res://script.gd"));
        editor.buffer_state = MockBufferState::Modified;
        let result = handle_quit(test_id(), &mut editor, ForceOverride::Force);
        assert_success(&result);
        assert!(editor.save_called, "tag_saved_version should be called");
        assert!(
            matches!(editor.buffer_state, MockBufferState::Closed),
            "tab should be closed"
        );
    }

    #[test]
    fn handle_quit_saved_closes_without_tagging() {
        let mut editor = MockEditorHost::new("clean", Some("res://script.gd"));
        let result = handle_quit(test_id(), &mut editor, ForceOverride::Normal);
        assert_success(&result);
        assert!(
            matches!(editor.buffer_state, MockBufferState::Closed),
            "tab should be closed"
        );
        assert!(
            !editor.save_called,
            "tag_saved_version should NOT be called for non-force quit of clean buffer"
        );
    }

    #[test]
    fn handle_quit_force_on_clean_buffer_tags_and_closes() {
        let mut editor = MockEditorHost::new("clean", Some("res://script.gd"));
        let result = handle_quit(test_id(), &mut editor, ForceOverride::Force);
        assert_success(&result);
        assert!(
            editor.save_called,
            "tag_saved_version should be called with force"
        );
        assert!(matches!(editor.buffer_state, MockBufferState::Closed));
    }

    #[test]
    fn handle_write_quit_success_saves_and_closes() {
        let mut editor = MockEditorHost::new("content", Some("res://script.gd"));
        let result = handle_write_quit(test_id(), &mut editor, ForceOverride::Normal);
        assert_success(&result);
        assert!(editor.save_called);
        assert!(matches!(editor.buffer_state, MockBufferState::Closed));
    }

    #[test]
    fn handle_write_quit_save_failure_no_force_returns_error() {
        let mut editor = MockEditorHost::new("content", Some("res://script.gd"));
        editor.save_result = Some(Err(HostError::WriteFailed {
            path: CompactString::from("res://script.gd"),
            detail: Some(CompactString::from("save failed")),
        }));
        let result = handle_write_quit(test_id(), &mut editor, ForceOverride::Normal);
        assert_failure(&result);
        assert!(failure_msg(&result).contains("E514"));
        assert!(!failure_msg(&result).contains("use :q!"));
        assert!(
            !matches!(editor.buffer_state, MockBufferState::Closed),
            "tab should NOT be closed on save failure"
        );
    }

    #[test]
    fn handle_write_quit_save_failure_with_force_returns_error_with_hint() {
        let mut editor = MockEditorHost::new("content", Some("res://script.gd"));
        editor.save_result = Some(Err(HostError::WriteFailed {
            path: CompactString::from("res://script.gd"),
            detail: Some(CompactString::from("save failed")),
        }));
        let result = handle_write_quit(test_id(), &mut editor, ForceOverride::Force);
        assert_failure(&result);
        assert!(
            failure_msg(&result).contains("use :q! to discard changes"),
            "msg={}",
            failure_msg(&result)
        );
        assert!(
            !matches!(editor.buffer_state, MockBufferState::Closed),
            "tab should NOT be closed on save failure"
        );
    }

    #[test]
    fn handle_write_quit_no_script_returns_e32() {
        let mut editor = MockEditorHost::new("content", None);
        let result = handle_write_quit(test_id(), &mut editor, ForceOverride::Normal);
        assert_failure(&result);
        assert!(failure_msg(&result).contains("E32"));
        assert!(!matches!(editor.buffer_state, MockBufferState::Closed));
    }

    #[test]
    fn handle_edit_file_same_path_shows_file_info() {
        let mut editor = MockEditorHost::new("line1\nline2\nline3", Some("res://script.gd"));
        let result = handle_edit_file(
            test_id(),
            &mut editor,
            "res://script.gd",
            ForceOverride::Normal,
            FileAccessScope::ProjectOnly,
        );
        assert_success(&result);
        let msg = success_msg(&result).unwrap();
        assert!(msg.contains("res://script.gd"), "msg={msg}");
        assert!(msg.contains("3L"), "msg={msg}");
        assert!(!msg.contains("written"), "msg={msg}");
    }

    #[test]
    fn handle_edit_file_same_path_force_reloads() {
        let mut editor = MockEditorHost::new("old text", Some("res://script.gd"));
        editor.files.insert(
            "res://script.gd".to_string(),
            "new disk text\nline2".to_string(),
        );
        let result = handle_edit_file(
            test_id(),
            &mut editor,
            "res://script.gd",
            ForceOverride::Force,
            FileAccessScope::ProjectOnly,
        );
        assert_success(&result);
        assert_eq!(editor.text, "new disk text\nline2");
        assert!(
            editor.save_called,
            "tag_saved_version should be called after reload"
        );
        assert!(
            editor.script_source_updated.is_some(),
            "script source should be updated"
        );
    }

    #[test]
    fn handle_edit_file_different_path_unsaved_no_force_returns_e37() {
        let mut editor = MockEditorHost::new("unsaved", Some("res://current.gd"));
        editor.buffer_state = MockBufferState::Modified;
        let result = handle_edit_file(
            test_id(),
            &mut editor,
            "res://other.gd",
            ForceOverride::Normal,
            FileAccessScope::ProjectOnly,
        );
        assert_failure(&result);
        assert!(failure_msg(&result).contains("E37"));
    }

    #[test]
    fn handle_edit_file_different_path_unsaved_with_force_opens() {
        let mut editor = MockEditorHost::new("unsaved", Some("res://current.gd"));
        editor.buffer_state = MockBufferState::Modified;
        let result = handle_edit_file(
            test_id(),
            &mut editor,
            "res://other.gd",
            ForceOverride::Force,
            FileAccessScope::ProjectOnly,
        );
        assert_success(&result);
        assert_eq!(editor.opened_path.as_deref(), Some("res://other.gd"));
    }

    #[test]
    fn handle_edit_file_different_path_clean_opens() {
        let mut editor = MockEditorHost::new("clean", Some("res://current.gd"));
        let result = handle_edit_file(
            test_id(),
            &mut editor,
            "res://other.gd",
            ForceOverride::Normal,
            FileAccessScope::ProjectOnly,
        );
        assert_success(&result);
        assert_eq!(editor.opened_path.as_deref(), Some("res://other.gd"));
    }

    #[test]
    fn handle_edit_file_scope_denied_returns_error() {
        let mut editor = MockEditorHost::new("content", Some("res://script.gd"));
        let result = handle_edit_file(
            test_id(),
            &mut editor,
            "/etc/passwd",
            ForceOverride::Normal,
            FileAccessScope::ProjectOnly,
        );
        assert_failure(&result);
        assert!(failure_msg(&result).contains("Access denied"));
    }

    #[test]
    fn handle_edit_file_open_failure_returns_error() {
        let mut editor = MockEditorHost::new("clean", Some("res://current.gd"));
        editor.open_result = Some(Err(HostError::CantOpenFile {
            path: CompactString::from("res://nonexistent.gd"),
            detail: None,
        }));
        let result = handle_edit_file(
            test_id(),
            &mut editor,
            "res://nonexistent.gd",
            ForceOverride::Normal,
            FileAccessScope::ProjectOnly,
        );
        assert_failure(&result);
        assert!(failure_msg(&result).contains("E484"));
    }

    #[test]
    fn handle_edit_file_reload_failure_returns_error() {
        let mut editor = MockEditorHost::new("old", Some("res://script.gd"));
        let result = handle_edit_file(
            test_id(),
            &mut editor,
            "res://script.gd",
            ForceOverride::Force,
            FileAccessScope::ProjectOnly,
        );
        assert_failure(&result);
        assert!(failure_msg(&result).contains("E484"));
        assert_eq!(editor.text, "old");
    }

    #[test]
    fn handle_edit_file_no_current_script_opens_new() {
        let mut editor = MockEditorHost::new("", None);
        let result = handle_edit_file(
            test_id(),
            &mut editor,
            "res://new.gd",
            ForceOverride::Normal,
            FileAccessScope::ProjectOnly,
        );
        assert_success(&result);
        assert_eq!(editor.opened_path.as_deref(), Some("res://new.gd"));
    }

    #[test]
    fn handle_write_file_explicit_godot_path_saves_via_script() {
        let mut editor = MockEditorHost::new("content", Some("res://original.gd"));
        let result = handle_write_file(
            test_id(),
            &mut editor,
            Some("res://copy.gd"),
            ForceOverride::Normal,
            FileAccessScope::ProjectOnly,
        );
        assert_success(&result);
        let msg = success_msg(&result).unwrap();
        assert!(msg.contains("res://copy.gd"), "msg={msg}");
    }

    #[test]
    fn handle_write_file_reports_accurate_counts() {
        let mut editor = MockEditorHost::new("abc\ndef\nghi", Some("res://script.gd"));
        let result = handle_write_file(
            test_id(),
            &mut editor,
            None,
            ForceOverride::Normal,
            FileAccessScope::ProjectOnly,
        );
        assert_success(&result);
        let msg = success_msg(&result).unwrap();
        assert!(msg.contains("3L"), "msg={msg}");
        assert!(msg.contains("11B"), "msg={msg}");
    }

    #[test]
    fn handle_edit_file_same_path_reports_accurate_counts() {
        let mut editor = MockEditorHost::new("one\ntwo", Some("res://script.gd"));
        let result = handle_edit_file(
            test_id(),
            &mut editor,
            "res://script.gd",
            ForceOverride::Normal,
            FileAccessScope::ProjectOnly,
        );
        assert_success(&result);
        let msg = success_msg(&result).unwrap();
        assert!(msg.contains("2L"), "msg={msg}");
        assert!(msg.contains("7B"), "msg={msg}");
    }
}
