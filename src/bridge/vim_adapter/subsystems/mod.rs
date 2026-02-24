//! VimController Subsystems — Logical groupings of related state.
//!
//! These structs decompose the monolithic VimController into cohesive units.
//! The `engine` (VimEngine) stays at the top level as the cross-cutting dependency.

pub mod dock;
pub mod input;
pub mod ui;
pub mod visuals;
