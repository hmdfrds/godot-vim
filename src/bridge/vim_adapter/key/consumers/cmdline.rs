//! `CmdLine` consumer - handles keys in command-line mode.
//!
//! In `CmdLine` mode, most keys pass through to the `LineEdit` widget.
//! Only ESC is consumed to exit the mode.

use crate::bridge::vim_adapter::key::consumer::{ConsumeResult, KeyConsumer};
use crate::bridge::vim_adapter::key::context::KeyContext;
use vim_core::inputs::KeyCode;

/// Consumer for command-line mode input.
///
/// When in `CmdLine` mode:
/// - ESC: Exit mode and return to Normal (consumed)
/// - All other keys: Pass through to `LineEdit`
pub struct CmdLineConsumer;

impl KeyConsumer for CmdLineConsumer {
    fn name(&self) -> &'static str {
        "CmdLine"
    }

    fn is_applicable(&self, ctx: &KeyContext) -> bool {
        ctx.is_cmdline_mode()
    }

    fn consume(&self, ctx: &mut KeyContext) -> ConsumeResult {
        if ctx.key.code == KeyCode::Esc {
            ctx.mark_handled();
            ConsumeResult::Consumed
        } else {
            ConsumeResult::Passthrough
        }
    }
}
