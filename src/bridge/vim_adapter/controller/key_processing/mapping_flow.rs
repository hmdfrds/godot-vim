use crate::bridge::settings;
use crate::bridge::vim_wrapper::VimController;
use vim_core::inputs::mapping::MappingMode;
use vim_core::inputs::VimKey;

impl VimController {
    /// Attempts to process a key through the mapping system.
    ///
    /// Returns `true` if the key was consumed by mapping logic (caller should return).
    /// Returns `false` if normal Vim processing should continue.
    pub(crate) fn try_process_mapping(&mut self, vim_key: &VimKey, from_user_input: bool) -> bool {
        if !settings::VimSettings::mapping_enabled() {
            return false;
        }

        let mapping_mode = match self.get_mapping_mode() {
            Some(mode) => mode,
            None => return false,
        };

        // Global mappings are available as a fallback in Normal/Visual mode,
        // but never in Insert mode to avoid typing delays.
        let check_global =
            !matches!(mapping_mode, MappingMode::Insert) && mapping_mode != MappingMode::Global;

        // Proceed only if there are pending keys or this key could start a new mapping.
        let has_pending = self.input.mapping_state.has_pending();
        if !(has_pending
            || self
                .input
                .mapping_store
                .could_start_mapping(vim_key, mapping_mode)
            || (check_global
                && self
                    .input
                    .mapping_store
                    .could_start_mapping(vim_key, MappingMode::Global)))
        {
            return false;
        }

        self.input.mapping_state.add_key(*vim_key);
        let pending = self.input.mapping_state.pending_keys();

        let exact_match = self
            .input
            .mapping_store
            .find_mapping(pending, mapping_mode)
            .or_else(|| {
                if check_global {
                    self.input
                        .mapping_store
                        .find_mapping(pending, MappingMode::Global)
                } else {
                    None
                }
            });

        if let Some(mapping) = exact_match {
            let action = mapping.to.clone();
            self.input.mapping_state.reset();
            if let Some(timer) = &mut self.input.mapping_timer {
                timer.stop();
            }
            self.process_mapped_key(action);

            if from_user_input {
                self.set_input_handled();
            }
            return true;
        }

        let has_prefix = self
            .input
            .mapping_store
            .has_prefix_match(self.input.mapping_state.pending_keys(), mapping_mode)
            || (check_global
                && self.input.mapping_store.has_prefix_match(
                    self.input.mapping_state.pending_keys(),
                    MappingMode::Global,
                ));

        if has_prefix {
            let timeoutlen = settings::VimSettings::timeoutlen();
            if let Some(timer) = &mut self.input.mapping_timer {
                #[expect(
                    clippy::cast_precision_loss,
                    reason = "milliseconds do not need 64-bit precision"
                )]
                timer.set_wait_time(timeoutlen as f64 / 1000.0);
                timer.start();
            }

            if from_user_input {
                self.set_input_handled();
            }
            return true;
        }

        // No match: flush pending keys and replay through the canonical internal path.
        let to_process = self.input.mapping_state.flush();
        for (i, key) in to_process.iter().enumerate() {
            self.process_vim_key_internal(key, i != 0, false);
        }

        if from_user_input {
            self.set_input_handled();
        }
        true
    }
}
