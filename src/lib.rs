//! GodotVim: Vim motions for the Godot editor.
#![deny(clippy::unwrap_used)]
#![deny(clippy::expect_used)]
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

use godot::prelude::*;

mod bridge;
pub mod logging;

struct GodotVim;

#[gdextension]
unsafe impl ExtensionLibrary for GodotVim {
    fn on_level_init(level: InitLevel) {
        if level == InitLevel::Scene {
            logging::init_logging();
        }
    }
}
