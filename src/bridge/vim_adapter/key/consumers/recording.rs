//! Recording consumer - handles macro recording.
//!
//! Records keys to the macro buffer and handles 'q' to stop recording.

use crate::bridge::vim_adapter::key::consumer::{ConsumeResult, KeyConsumer};
use crate::bridge::vim_adapter::key::context::KeyContext;
use vim_core::inputs::KeyCode;

/// Consumer for macro recording.
///
/// When recording:
/// - 'q': Stop recording (consumed, but not recorded to the macro buffer)
/// - Other keys: Record to macro buffer, continue processing
///
/// The actual recording to the macro buffer is done in `VimWrapper` because
/// consumers do not have mutable access to `VimState`.
pub struct RecordingConsumer;

impl KeyConsumer for RecordingConsumer {
    fn name(&self) -> &'static str {
        "Recording"
    }

    fn is_applicable(&self, ctx: &KeyContext) -> bool {
        ctx.is_recording()
    }

    fn consume(&self, ctx: &mut KeyContext) -> ConsumeResult {
        if ctx.key.code == KeyCode::Char('q') {
            ctx.mark_handled();
            ConsumeResult::Consumed
        } else {
            ConsumeResult::NotConsumed
        }
    }
}
