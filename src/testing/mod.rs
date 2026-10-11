//! Test infrastructure for the bridge layer. Provides `MockTextEdit` (a pure-Rust
//! stand-in for Godot's CodeEdit), `MockHost` (vim-core's engine on top of it,
//! through the production dispatch), and integration tests that verify effect
//! handlers produce correct text/cursor/selection state without Godot runtime.

#[cfg(test)]
mod mock_text_edit;

#[cfg(test)]
mod mock_host;

#[cfg(test)]
mod bridge_tests;

#[cfg(test)]
mod history_tests;

#[cfg(test)]
pub(crate) use mock_text_edit::MockTextEdit;
