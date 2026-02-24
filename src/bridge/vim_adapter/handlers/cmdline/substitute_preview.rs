use crate::bridge::vim_wrapper::VimController;

impl VimController {
    /// Clear substitute preview state without reverting (for when command is executed).
    ///
    /// Called after a substitute command is executed, since the preview changes
    /// should now become permanent.
    pub(crate) fn clear_substitute_preview_state(&mut self) {
        self.visuals.substitute_preview.clear();
    }
}
