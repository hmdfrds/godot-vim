//! Individual key consumers.
//!
//! Each consumer handles a specific concern in the key processing pipeline.

pub(crate) mod cmdline;
pub(crate) mod completion;
pub(crate) mod mapping;
pub(crate) mod passthrough;
pub(crate) mod recording;
pub(crate) mod vim_command;

pub use cmdline::CmdLineConsumer;
pub use completion::CompletionConsumer;
pub use mapping::MappingConsumer;
pub use passthrough::PassthroughConsumer;
pub use recording::RecordingConsumer;
pub use vim_command::VimCommandConsumer;
