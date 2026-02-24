//! vim_adapter anti-corruption layer.
//!
//! This is the only bridge module that imports `vim_core`.
//! Runtime execution is routed through the canonical adapter contract.
//!
//! Canonical runtime flow:
//! `InputEvent -> VimKey -> runtime_gateway -> engine::process_key_with_policy -> DispatchBatch -> dispatch`.
//!
//! Canonical Ex flow:
//! `command source -> runtime_gateway::execute_ex_command_with_visuals -> engine::process_ex_command_with_context -> DispatchBatch -> dispatch`.

// Facade layer
pub mod contracts;
pub mod convert;
pub mod effect_converter;
pub mod engine;
pub mod output;

// Core logic — vim-core dependent
pub mod controller;
pub mod core;
pub mod handlers;
pub mod key;
pub mod managers;
pub mod mapping;
pub mod optimization;
pub mod subsystems;

// Test infrastructure
#[cfg(test)]
mod engine_contract_tests;
#[cfg(test)]
mod engine_integration_tests;
#[cfg(test)]
mod engine_mutation_tests;
#[cfg(test)]
mod engine_tests;
#[cfg(test)]
pub mod mock;
