//! Sourcing a vimrc on request (`:source`, a Mapping Dialog save): what it
//! did and what the status bar says about it.

use compact_str::format_compact;

use crate::types::StatusMessage;

/// What sourcing a vimrc did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Loaded {
    Missing,
    /// A project file under Project Vimrc `Disabled`: nothing was sourced.
    Disabled,
    /// `stripped` lines were disabled by the Sandbox policy.
    Sourced {
        stripped: usize,
    },
}

/// The status line after loading `path`. `None` keeps the engine's own
/// "Config file sourced".
pub(crate) fn status(path: &str, loaded: Loaded) -> StatusMessage {
    match loaded {
        Loaded::Missing => StatusMessage::Error(format_compact!("E484: Can't open file {path}")),
        Loaded::Disabled => StatusMessage::Error(format_compact!(
            "{path} not sourced: Security > Project Vimrc is Disabled"
        )),
        Loaded::Sourced { stripped: 0 } => StatusMessage::None,
        Loaded::Sourced { stripped } => StatusMessage::Info(format_compact!(
            "Config file sourced, {stripped} line(s) skipped by the Sandbox policy"
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_clean_load_leaves_the_engines_message() {
        assert_eq!(
            status("user://.godot-vimrc", Loaded::Sourced { stripped: 0 }),
            StatusMessage::None
        );
    }

    #[test]
    fn every_other_outcome_is_named() {
        let path = "res://.godot-vimrc";
        assert_eq!(
            status(path, Loaded::Missing),
            StatusMessage::Error("E484: Can't open file res://.godot-vimrc".into())
        );
        assert_eq!(
            status(path, Loaded::Disabled),
            StatusMessage::Error(
                "res://.godot-vimrc not sourced: Security > Project Vimrc is Disabled".into()
            )
        );
        assert_eq!(
            status(path, Loaded::Sourced { stripped: 2 }),
            StatusMessage::Info(
                "Config file sourced, 2 line(s) skipped by the Sandbox policy".into()
            )
        );
    }
}
