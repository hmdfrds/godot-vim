//! Mapping handler methods for VimController.
//!
//! Implements key mapping logic separated from vim_wrapper.rs for clarity.

use crate::bridge::vim_adapter::mapping::{MappedAction, MappingMode};
use crate::bridge::vim_wrapper::VimController;

use vim_core::state::mode::Mode;

impl VimController {
    /// Gets the mapping mode for the current Vim mode.
    /// Returns None if mappings should not apply (e.g. CmdLine).
    pub(crate) fn get_mapping_mode(&self) -> Option<MappingMode> {
        match self.engine.mode() {
            Mode::Normal => Some(MappingMode::Normal),
            Mode::Insert(..) | Mode::Replace(_) => Some(MappingMode::Insert),
            Mode::Visual(_) => Some(MappingMode::Visual),
            Mode::CmdLine(_) => None,
            // All pending modes act like Normal for mapping purposes
            _ => Some(MappingMode::Normal),
        }
    }

    /// Processes a mapped action by converting it to VimKeys and executing.
    pub(crate) fn process_mapped_key(&mut self, action: MappedAction) {
        match action {
            MappedAction::Key(key) => {
                self.process_vim_key_internal(&key, false, false);
            }
            MappedAction::Keys(keys) => {
                for key in keys {
                    self.process_vim_key_internal(&key, false, false);
                }
            }
            MappedAction::Command(cmd) => {
                let prev_mode = self.engine.mode();
                self.execute_ex_command_with_visuals(&cmd, prev_mode);
            }
        }
    }
}
