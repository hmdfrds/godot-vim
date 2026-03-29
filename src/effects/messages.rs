//! Applies message and error effects to the shell's global state for
//! display in the status bar.

use vim_core::errors::VimError;

use crate::state::GlobalState;

pub(crate) fn handle_show_message(globals: &mut GlobalState, text: &str) {
    log::debug!("show_message: {}", text);
    globals.set_message(text);
}

/// Vim errors (E486, E20, etc.) are expected user-facing errors — surfaced
/// via the status bar, not the log panel.
pub(crate) fn handle_show_error(globals: &mut GlobalState, error: &VimError) {
    let msg = error.to_string();
    log::debug!("show_error: {}", msg);
    globals.set_error(&msg);
}

pub(super) fn handle_clear_message(globals: &mut GlobalState) {
    globals.clear_message();
}
