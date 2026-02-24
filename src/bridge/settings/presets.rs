//! Curated list of popular Vim mappings as presets.
//!
//! All mappings are disabled by default (modes = "").
//! Users can enable them via the Mappings panel in Project Settings.

use typed_builder::TypedBuilder;

/// A simple representation of a recommended mapping.
#[derive(TypedBuilder)]
pub struct PresetMapping {
    pub from: String,
    pub to: String,
    /// Mode flags: i=insert, n=normal, v=visual, c=command
    /// Empty string means disabled by default
    #[builder(default)]
    pub modes: String,
}

/// Returns the full list of recommended mappings.
///
/// Organized into sections:
/// 1. Insert Mode Escapes - Popular ways to exit insert mode
/// 2. Leader Mappings - Space-prefixed shortcuts
/// 3. Window Navigation - Ctrl+HJKL for pane movement
/// 4. Buffer Navigation - Quick buffer switching
/// 5. Godot-Specific - Editor integration commands
/// 6. Popular Vim Mappings - Common convenience remaps
#[allow(clippy::vec_init_then_push)]
pub fn get_recommended_mappings() -> Vec<PresetMapping> {
    let mut mappings = Vec::new();

    // ═══════════════════════════════════════════════════════════════════════════════
    // Insert Mode Escapes
    // ═══════════════════════════════════════════════════════════════════════════════
    mappings.push(PresetMapping {
        from: "jj".into(),
        to: "<Esc>".into(),
        modes: "".into(), // Disabled by default
    });
    mappings.push(PresetMapping {
        from: "jk".into(),
        to: "<Esc>".into(),
        modes: "".into(),
    });
    mappings.push(PresetMapping {
        from: "kj".into(),
        to: "<Esc>".into(),
        modes: "".into(),
    });

    // ═══════════════════════════════════════════════════════════════════════════════
    // Leader Mappings (Space prefix)
    // ═══════════════════════════════════════════════════════════════════════════════

    // Buffer navigation
    mappings.push(PresetMapping {
        from: "<Space>n".into(),
        to: ":bn".into(),
        modes: "n".into(),
    });
    mappings.push(PresetMapping {
        from: "<Space>p".into(),
        to: ":bp".into(),
        modes: "n".into(),
    });

    // Buffer switching 1-9
    for i in 1..=9 {
        mappings.push(PresetMapping {
            from: format!("<Space>{i}"),
            to: format!(":b{i}"),
            modes: "n".into(),
        });
    }

    // ═══════════════════════════════════════════════════════════════════════════════
    // Godot-Specific Mappings
    // ═══════════════════════════════════════════════════════════════════════════════
    // Debugging
    mappings.push(PresetMapping {
        from: "<Space>db".into(),
        to: ":GodotBreakpoint".into(),
        modes: "n".into(),
    });
    mappings.push(PresetMapping {
        from: "<Space>dc".into(),
        to: ":GodotContinue".into(),
        modes: "n".into(),
    });
    mappings.push(PresetMapping {
        from: "<Space>dn".into(),
        to: ":GodotNext".into(),
        modes: "n".into(),
    });
    mappings.push(PresetMapping {
        from: "<Space>di".into(),
        to: ":GodotStepIn".into(),
        modes: "n".into(),
    });
    mappings.push(PresetMapping {
        from: "<Space>do".into(),
        to: ":GodotStepOut".into(),
        modes: "n".into(),
    });
    mappings.push(PresetMapping {
        from: "<Space>dp".into(),
        to: ":GodotPause".into(),
        modes: "n".into(),
    });

    // ═══════════════════════════════════════════════════════════════════════════════
    // Global Mode Mappings (Editor-wide, active by default)
    // ═══════════════════════════════════════════════════════════════════════════════
    // Dock Navigation
    mappings.push(PresetMapping {
        from: "<Space>e".into(),
        to: ":FileSystem".into(),
        modes: "g".into(),
    });
    mappings.push(PresetMapping {
        from: "<Space>o".into(),
        to: ":Scene".into(),
        modes: "g".into(),
    });
    mappings.push(PresetMapping {
        from: "<Space>i".into(),
        to: ":Inspector".into(),
        modes: "g".into(),
    });
    mappings.push(PresetMapping {
        from: "<Space>s".into(),
        to: ":Script".into(),
        modes: "g".into(),
    });
    mappings.push(PresetMapping {
        from: "<Space>`".into(),
        to: ":FocusDock output".into(),
        modes: "g".into(),
    });
    mappings.push(PresetMapping {
        from: "<Space>f2".into(),
        to: ":FocusDock 2d".into(),
        modes: "g".into(),
    });
    mappings.push(PresetMapping {
        from: "<Space>f3".into(),
        to: ":FocusDock 3d".into(),
        modes: "g".into(),
    });

    // ═══════════════════════════════════════════════════════════════════════════════
    // Scene Control (Global - work from any focus)
    // ═══════════════════════════════════════════════════════════════════════════════
    mappings.push(PresetMapping {
        from: "<Space>r".into(),
        to: ":run".into(),
        modes: "g".into(),
    });
    mappings.push(PresetMapping {
        from: "<Space>R".into(),
        to: ":runcurrent".into(),
        modes: "g".into(),
    });
    mappings.push(PresetMapping {
        from: "<Space>S".into(),
        to: ":stop".into(),
        modes: "g".into(),
    });

    // ═══════════════════════════════════════════════════════════════════════════════
    // File Operations (Normal mode)
    // ═══════════════════════════════════════════════════════════════════════════════
    mappings.push(PresetMapping {
        from: "<Space>w".into(),
        to: ":save".into(),
        modes: "n".into(),
    });
    mappings.push(PresetMapping {
        from: "<Space>W".into(),
        to: ":saveall".into(),
        modes: "n".into(),
    });

    // ═══════════════════════════════════════════════════════════════════════════════
    // Editor State (Global)
    // ═══════════════════════════════════════════════════════════════════════════════
    mappings.push(PresetMapping {
        from: "<Space>z".into(),
        to: ":zen".into(),
        modes: "g".into(),
    });
    mappings.push(PresetMapping {
        from: "<Space>Z".into(),
        to: ":unzen".into(),
        modes: "g".into(),
    });

    mappings
}
