//! Config file system for GodotVim.
//!
//! Manages reading, writing, and parsing of `.godot-vimrc` config files
//! that persist mappings and settings across editor sessions.
//!
//! The config file uses standard Vim syntax (`:set`, `:map`, `:noremap`, etc.)
//! and is processed by the engine's existing `:source` infrastructure.
//! A preset system allows users to toggle recommended mappings on/off
//! via the Mapping Dialog.
//!
//! # Lifecycle
//!
//! On `enter_tree`, the plugin resolves the config path (via [`path`]), reads the
//! file, applies security policy (via [`sandbox`]), and calls
//! `controller.reload_config()`. On `:mappings` dialog save, the dialog emits
//! `config_saved` which triggers re-sourcing. On `:source`, the same pipeline
//! runs with the sandboxing policy from [`SecurityPolicy`].

pub(crate) mod mapping_service;
pub(crate) mod parser;
pub(crate) mod path;
pub(crate) mod presets;
pub(crate) mod sandbox;
pub(crate) mod types;
pub(crate) mod writer;
