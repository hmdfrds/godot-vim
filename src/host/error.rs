//! Typed error enum for `EditorHost` trait methods.
//!
//! Replaces `Result<T, String>` with `Result<T, HostError>` across the trait
//! and its helper functions. Handlers do NOT match on variants — they convert
//! to `String` at the `host_failure()` boundary via `Display`.

use compact_str::CompactString;
use std::fmt;

/// Typed errors for `EditorHost` operations.
///
/// A variant with a Vim counterpart displays as `E{N}: {message}`; the
/// Godot-only refusals carry no number. `host_failure()` forwards the text to
/// the engine as a `CompactString`.
#[derive(Debug, Clone)]
pub(crate) enum HostError {
    /// E32: No file name — buffer has no associated path.
    NoFileName,

    /// E37: No write since last change (add ! to override).
    #[allow(dead_code)]
    UnsavedChanges,

    /// E166: Can't open linked file for writing — symlink write blocked without `!`.
    SymlinkWrite { path: CompactString },

    /// E382: Cannot write. The attached CodeEdit (Shader Editor, addon) is
    /// not the script editor's current script, so saving would put its text
    /// into that script. Vim's E382 refuses a buffer that is not a file.
    NotCurrentScript,

    /// E484: Can't open file — file not found, access denied, too large, etc.
    CantOpenFile {
        path: CompactString,
        detail: Option<CompactString>,
    },

    /// E514: Write failed — ResourceSaver or `std::fs::write` error.
    WriteFailed {
        path: CompactString,
        detail: Option<CompactString>,
    },

    /// No File menu of Godot's would close the attached editor's tab.
    NotClosed,
}

impl fmt::Display for HostError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            HostError::NoFileName => write!(f, "E32: No file name"),

            HostError::UnsavedChanges => {
                write!(f, "E37: No write since last change (add ! to override)")
            }

            HostError::SymlinkWrite { path } => {
                write!(f, "E166: Can't open linked file for writing: \"{}\"", path)
            }

            HostError::NotCurrentScript => write!(
                f,
                "E382: Cannot write, this editor is not the script editor's current script; \
                 use its own Save"
            ),

            HostError::CantOpenFile { path, detail } => match detail {
                Some(d) => write!(f, "E484: Can't open file \"{}\": {}", path, d),
                None => write!(f, "E484: Can't open file \"{}\"", path),
            },

            HostError::WriteFailed { path, detail } => match detail {
                Some(d) => write!(f, "E514: Failed to write \"{}\": {}", path, d),
                None => write!(f, "E514: Failed to write \"{}\"", path),
            },

            HostError::NotClosed => write!(
                f,
                "Cannot close this editor: no File menu of Godot's closes it from here"
            ),
        }
    }
}

impl std::error::Error for HostError {}
