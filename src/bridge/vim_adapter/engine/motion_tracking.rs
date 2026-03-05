use godot::classes::CodeEdit;
use godot::prelude::Gd;
use vim_core::domain::selection::Selection;
use vim_core::inputs::commands::motions::Motion;

use super::VimEngine;

impl VimEngine {
    pub(crate) fn apply_motion(&mut self, editor: &mut Gd<CodeEdit>, motion: Motion, count: usize) {
        crate::bridge::vim_adapter::handlers::motion::apply_pure_motion(
            editor,
            &mut self.state,
            motion,
            count,
            &self.config,
        );
    }

    #[allow(dead_code)]
    pub(crate) fn execute_vertical_motion(
        &mut self,
        editor: &mut Gd<CodeEdit>,
        motion: Motion,
        count: usize,
    ) {
        crate::bridge::vim_adapter::handlers::motion::execute_vertical_motion_fold_aware_public(
            editor,
            &mut self.state,
            motion,
            count,
            &self.config,
        );
    }

    #[allow(dead_code)]
    pub(crate) fn execute_search_motion(
        &mut self,
        editor: &mut Gd<CodeEdit>,
        motion: Motion,
        count: usize,
    ) -> Option<Selection> {
        crate::bridge::vim_adapter::handlers::motion::execute_search_motion(
            editor,
            &mut self.state,
            motion,
            count,
        )
    }
}
