//! Compound action execution: `:norm` across line ranges and `:wincmd` navigation.

use godot::classes::CodeEdit;
use godot::prelude::*;

use super::context::ProcessContext;
use crate::bridge;
use crate::bridge::port_impl::CodeEditPort;
use crate::effects::CompoundAction;

impl ProcessContext<'_> {
    pub(super) fn process_compound_action(&mut self, action: CompoundAction, editor: &mut Gd<CodeEdit>) {
        match action {
            CompoundAction::NormCommand {
                start_line,
                end_line,
                keys,
                remap,
            } => {
                log::debug!(
                    "compound_action: NormCommand lines={}..={} keys_len={} remap={}",
                    start_line.get(), end_line.get(), keys.len(), matches!(remap, crate::types::RemapPolicy::Remap)
                );
                self.execute_norm_command(start_line, end_line, &keys, remap, editor);
            }
            CompoundAction::WindowNav { action } => {
                log::debug!("compound_action: WindowNav {:?}", action);
                let control: Gd<godot::classes::Control> = editor.clone().upcast();
                crate::navigation::handle_window_nav_action(&control, action);
            }
        }
    }

    /// Execute a key sequence on each line in `[start_line, end_line]`.
    ///
    /// Iterates forward with a fixed end bound (matching Vim's algorithm).
    /// Uses `feed_keys` so `:norm!` can inject with `NOREMAP` flags.
    fn execute_norm_command(
        &mut self,
        start_line: crate::effects::LineNumber,
        end_line: crate::effects::LineNumber,
        keys: &str,
        remap: crate::types::RemapPolicy,
        editor: &mut Gd<CodeEdit>,
    ) {
        if keys.is_empty() {
            log::debug!("execute_norm_command: skipped (empty keys)");
            return;
        }

        // Wrap in a tracked undo group so `ensure_undo_balanced` can recover
        // from unmatched begin/end pairs if a panic is caught mid-loop.
        {
            let mut port = CodeEditPort(editor);
            crate::effects::undo::handle_begin_undo_group(&mut port, self.undo_depth);
        }

        let line_count = bridge::codec::i32_to_usize(editor.get_line_count().max(1));
        let lo = start_line.get().min(end_line.get()).min(line_count - 1);
        let hi = start_line.get().max(end_line.get()).min(line_count - 1);
        let remap_bool = matches!(remap, crate::types::RemapPolicy::Remap);
        log::debug!(
            "execute_norm_command: processing lines {}..={} (keys={}, remap={})",
            lo, hi, keys, remap_bool
        );

        // Give the `:norm` loop its own iteration budget. Without this,
        // `:%norm` on large files would hit MAX_DRAIN_ITERATIONS prematurely.
        let saved_operations_this_cycle = *self.operations_this_cycle;
        *self.operations_this_cycle = 0;

        let mut current = lo;
        let end = hi;

        while current <= end {
            let actual_line_count = bridge::codec::i32_to_usize(editor.get_line_count().max(1));
            if current >= actual_line_count {
                break;
            }

            // Without reset, `:norm Ahello` (no trailing Esc) would leave
            // insert mode active, and subsequent lines would misinterpret keys.
            if self.engine.mode() != vim_core::primitives::Mode::Normal {
                self.engine.set_mode(vim_core::primitives::Mode::Normal);
            }

            let line_i32 = bridge::codec::usize_to_i32(current);
            editor.set_caret_line(line_i32);
            editor.set_caret_column(0);

            self.engine.feed_keys(keys, remap_bool);
            self.drain_pending(editor);

            current += 1;
        }

        // Merge back so the outer runaway guard sees total operations.
        *self.operations_this_cycle = saved_operations_this_cycle.saturating_add(*self.operations_this_cycle);

        {
            let mut port = CodeEditPort(editor);
            crate::effects::undo::handle_end_undo_group(&mut port, self.undo_depth);
        }
    }
}
