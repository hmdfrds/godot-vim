//! Vim command consumer - handles normal Vim command parsing.
//!
//! This is the final consumer that parses Vim commands (motions, operators, etc.)
//! and delegates to the pure Core parser.

use crate::bridge::vim_adapter::key::consumer::{ConsumeResult, KeyConsumer};
use crate::bridge::vim_adapter::key::context::KeyContext;
use vim_core::inputs::KeyCode;

/// Consumer for normal Vim command processing.
///
/// Handles ESC in Normal mode (clears highlights) and parses all other
/// keys as Vim commands. Command execution is delegated to `VimWrapper`.
pub struct VimCommandConsumer;

impl KeyConsumer for VimCommandConsumer {
    fn name(&self) -> &'static str {
        "VimCommand"
    }

    fn is_applicable(&self, _ctx: &KeyContext) -> bool {
        // Always applicable as the final fallback
        true
    }

    fn consume(&self, ctx: &mut KeyContext) -> ConsumeResult {
        if ctx.is_normal_mode() && ctx.key.code == KeyCode::Esc {
            ctx.mark_handled();
            return ConsumeResult::Consumed;
        }

        ctx.mark_handled();
        ConsumeResult::Consumed
    }
}
