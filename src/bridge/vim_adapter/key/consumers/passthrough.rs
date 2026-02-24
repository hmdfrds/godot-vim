//! Passthrough consumer - handles keys that bypass Vim.
//!
//! This consumer uses the unified `should_vim_handle` decision function
//! to determine if a key should pass through to Godot.

use crate::bridge::settings::VimSettings;
use crate::bridge::vim_adapter::key::consumer::{ConsumeResult, KeyConsumer};
use crate::bridge::vim_adapter::key::context::KeyContext;
use vim_core::inputs::whitelist::should_vim_handle;

/// Consumer that handles passthrough keys.
///
/// Uses the unified `should_vim_handle` function for routing decisions.
pub struct PassthroughConsumer;

impl KeyConsumer for PassthroughConsumer {
    fn name(&self) -> &'static str {
        "Passthrough"
    }

    fn is_applicable(&self, _ctx: &KeyContext) -> bool {
        true
    }

    fn consume(&self, ctx: &mut KeyContext) -> ConsumeResult {
        // Check for a custom mapping before passthrough; mappings take priority.
        // This allows bindings like <C-s> → :w to work.
        if ctx.could_start_mapping() || !ctx.pending_keys.is_empty() {
            return ConsumeResult::NotConsumed;
        }

        let user_passthrough = VimSettings::key_passthrough_list();

        if !should_vim_handle(&ctx.key, &ctx.vim_state.mode(), &user_passthrough) {
            return ConsumeResult::Passthrough;
        }

        ConsumeResult::NotConsumed
    }
}
