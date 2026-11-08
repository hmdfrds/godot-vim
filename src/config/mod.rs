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
//! file once, and loads it through the [`policy`], which judges each command
//! vim-core parses from it. On `:mappings` dialog save, the dialog emits
//! `config_saved` which triggers re-sourcing. `:source` re-reads the active
//! file the same way, or sources another file on top (see [`source`]).

pub(crate) mod mapping_service;
pub(crate) mod panelmap;
pub(crate) mod parser;
pub(crate) mod path;
pub(crate) mod policy;
pub(crate) mod presets;
pub(crate) mod sandbox;
pub(crate) mod source;
pub(crate) mod types;
pub(crate) mod writer;

/// Run config `text` in the engine with a host that answers nothing: every
/// command is allowed and a `:source` comes back as a host request.
#[cfg(test)]
pub(crate) fn source_text(
    engine: &mut vim_core::execution::VimEngine,
    text: &str,
) -> vim_core::execution::Response {
    use vim_core::execution::{ConfigText, TrustAll};
    engine
        .source_script(ConfigText::new("", text), &mut TrustAll::new())
        .response
}
