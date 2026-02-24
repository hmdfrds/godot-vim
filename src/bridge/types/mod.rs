//! Isolated Domain Types for godot-vim.
//!
//! These types form the public API boundary between godot-vim modules.
//! **NO vim-core imports allowed here** — all conversions happen in `vim_adapter/`.
//!
//! # Design Principles
//!
//! - **Full isolation**: Changing vim-core internals requires only updating `vim_adapter/convert.rs`
//! - **Newtype safety**: No raw primitives in public APIs
//! - **Zero-cost**: Types are `Copy` where possible, stack-allocated

pub mod command;
pub mod cursor;
pub mod key;
pub mod mode;
