//! Completion consumer - handles code completion popup navigation.
//!
//! When the code completion popup is active, certain keys are passed
//! through to Godot for navigation/selection.

use crate::bridge::vim_adapter::key::consumer::{ConsumeResult, KeyConsumer};
use crate::bridge::vim_adapter::key::context::KeyContext;
use vim_core::inputs::KeyCode;

/// Consumer for code completion popup handling.
///
/// When completion popup is active:
/// - Up/Down/Tab/Enter: Pass through for navigation
/// - ESC: Cancel completion (consumed)
/// - Other keys: Continue normal processing
pub struct CompletionConsumer;

impl KeyConsumer for CompletionConsumer {
    fn name(&self) -> &'static str {
        "Completion"
    }

    fn is_applicable(&self, ctx: &KeyContext) -> bool {
        ctx.completion_active
    }

    fn consume(&self, ctx: &mut KeyContext) -> ConsumeResult {
        match ctx.key.code {
            KeyCode::Up | KeyCode::Down | KeyCode::Tab => ConsumeResult::Passthrough,
            KeyCode::Enter => ConsumeResult::NotConsumed,
            KeyCode::Esc => {
                ctx.mark_handled();
                ConsumeResult::Passthrough
            }
            _ => ConsumeResult::NotConsumed,
        }
    }
}
