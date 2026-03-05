//! Controller submodules for `VimController`.
//!
//! These modules extend `VimController` with impl blocks for different categories
//! of handler methods, reducing the size of `vim_wrapper.rs`.

pub(crate) mod attach_session;
mod buffer;
mod cmdline;
mod cursor;
mod cursor_geometry;
mod dispatch;
mod dispatch_actions;
mod dispatch_search;
mod edit;
mod edit_repeat;
mod external_cmd_queue;
mod input_pipeline;
mod key_processing;
mod lifecycle;
mod mapping;
mod runtime_pipeline;
mod runtime_gateway;
mod signals;
mod transaction;
mod visual_selection;
mod visuals;

// Re-export traits for use in vim_wrapper.rs
pub use lifecycle::LifecycleTrait;
pub use signals::SignalHandlersTrait;
