//! Safety utilities and input handling.
//!
//! Contains safety guards and input conversion utilities.

pub mod guards;
pub mod input;

// Convenience re-exports
pub use guards::{guard, install_panic_hook};
