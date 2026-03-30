//! Typed error enum for `EditorHost` trait methods.
//!
//! Replaces `Result<T, String>` with `Result<T, HostError>` across the trait
//! and its helper functions. Handlers do NOT match on variants — they convert
//! to `String` at the `host_failure()` boundary via `Display`.

use compact_str::CompactString;
use std::fmt;

/// Typed errors for `EditorHost` operations.
///
/// Each variant corresponds to a canonical Vim error code. The `Display` impl
/// produces the `E{N}: {message}` format that `host_failure()` forwards to the
/// engine as a `CompactString`.
#[derive(Debug, Clone)]
pub(crate) enum HostError {
    /// E32: No file name — buffer has no associated path.
    NoFileName,

    /// E37: No write since last change (add ! to override).
    #[allow(dead_code)]
    UnsavedChanges,

    /// E166: Can't open linked file for writing — symlink write blocked without `!`.
    SymlinkWrite { path: CompactString },

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

            HostError::CantOpenFile { path, detail } => match detail {
                Some(d) => write!(f, "E484: Can't open file \"{}\": {}", path, d),
                None => write!(f, "E484: Can't open file \"{}\"", path),
            },

            HostError::WriteFailed { path, detail } => match detail {
                Some(d) => write!(f, "E514: Failed to write \"{}\": {}", path, d),
                None => write!(f, "E514: Failed to write \"{}\"", path),
            },
        }
    }
}

impl std::error::Error for HostError {}
