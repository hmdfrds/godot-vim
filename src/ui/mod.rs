//! Visual subsystems and their coordinator.
//!
//! Each subsystem is a self-contained Godot node (or plain struct) injected
//! into the CodeEdit by [`coordinator::UiCoordinator`]:
//!
//! - [`status_bar`]: Floating mode/message/command-line display.
//! - [`cursor_shape`]: Animated cursor overlay with GLSL difference-blend shader.
//! - [`line_numbers`]: Relative/hybrid gutter with fold icons.
//! - [`search_hl`]: Search match highlighting via CodeEdit's built-in API.
//! - [`inccommand`]: Live `:s` substitute preview overlay.
//! - [`highlight_yank`]: Yank fade animation.
//! - [`operator_debugger`]: Debug range highlight for `:vimdebug`.
//! - [`virtual_text`]: Inline text annotations.
//!
//! The [`UiSnapshot`] struct defined in [`crate::types`] is the data contract
//! between the controller and the UI layer — populated once per keystroke by the
//! controller and passed to [`UiCoordinator::update`].

mod coordinator;
pub(crate) mod cursor_shape;
mod geometry;
mod highlight_yank;
mod inccommand;
mod line_numbers;
pub(crate) mod mapping_dialog;
mod operator_debugger;
mod search_hl;
mod status_bar;
// Virtual text overlay is wired into the attach/detach lifecycle but the
// engine does not yet emit virtual text effects, so update methods are
// unused. Dead-code suppression is narrowed to individual methods in the module.
mod virtual_text;

pub(crate) use coordinator::UiCoordinator;
pub(crate) use cursor_shape::CursorColorMap;
