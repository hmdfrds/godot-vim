//! Vim engine facade and runtime modules.
//!
//! The adapter executes vim-core through a single runtime spine.

use vim_core::runtime::EffectAccumulator;
use vim_core::state::config::Config as VimConfig;
use vim_core::state::VimState;

mod config;
mod motion_tracking;
mod queries;
mod runtime;
mod tracking;

/// Vim adapter facade.
///
/// Owns canonical vim-core state, effect accumulator, and config.
pub struct VimEngine {
    /// Vim runtime state. Accessed only through typed facade methods.
    state: VimState,
    /// Reused effect accumulator for each executed action.
    pub(crate) effects: EffectAccumulator,
    /// Canonical vim-core config snapshot used by runtime handlers.
    pub(crate) config: VimConfig,
}

impl Default for VimEngine {
    fn default() -> Self {
        Self::new()
    }
}
