//! Curated preset mappings for GodotVim.
//!
//! Presets are recommended mappings that users can toggle on/off via the
//! Mapping Dialog. They are stored in the config file with marker comments.

use super::types::{mapping_to_vim_command, parse_mode_prefix, ParsedMapping};
use vim_core::keymap::MappingKind;

/// A preset mapping definition for the Mapping Dialog's Presets tab.
#[derive(Debug, Clone)]
pub(crate) struct PresetDefinition {
    pub(crate) lhs: &'static str,
    pub(crate) rhs: &'static str,
    /// Vim mode prefix: "n", "i", "v", "o", "c", or "" for `:map` (all).
    pub(crate) mode_prefix: &'static str,
    /// Mapping kind: non-recursive (`noremap`) or recursive (`map`).
    pub(crate) kind: MappingKind,
    /// Dialog grouping category (e.g., "File Ops", "Debugging").
    pub(crate) category: &'static str,
    pub(crate) default_enabled: bool,
}

impl PresetDefinition {
    pub(crate) fn to_vim_command(&self) -> String {
        let parsed = ParsedMapping {
            lhs: self.lhs.to_string(),
            rhs: self.rhs.to_string(),
            modes: parse_mode_prefix(self.mode_prefix)
                .unwrap_or(vim_core::grammar::MapModePrefix::Normal),
            kind: self.kind,
        };
        mapping_to_vim_command(&parsed)
    }
}

/// All shipped presets use `noremap` (non-recursive) to prevent mapping chain
/// surprises and satisfy sandbox requirements. V1's Global-mode entries became
/// Normal-mode since that's the only mode active when the editor has focus.
pub(crate) const PRESETS: &[PresetDefinition] = &[
    // ── Insert Mode Escapes ──────────────────────────────────────────────
    PresetDefinition {
        lhs: "jj",
        rhs: "<Esc>",
        mode_prefix: "i",
        kind: MappingKind::NonRecursive,
        category: "Insert Escape",
        default_enabled: false,
    },
    PresetDefinition {
        lhs: "jk",
        rhs: "<Esc>",
        mode_prefix: "i",
        kind: MappingKind::NonRecursive,
        category: "Insert Escape",
        default_enabled: false,
    },
    // ── File Operations ──────────────────────────────────────────────────
    PresetDefinition {
        lhs: "<Space>w",
        rhs: ":save<CR>",
        mode_prefix: "n",
        kind: MappingKind::NonRecursive,
        category: "File Ops",
        default_enabled: false,
    },
    PresetDefinition {
        lhs: "<Space>W",
        rhs: ":saveall<CR>",
        mode_prefix: "n",
        kind: MappingKind::NonRecursive,
        category: "File Ops",
        default_enabled: false,
    },
    // ── Buffer Navigation ────────────────────────────────────────────────
    PresetDefinition {
        lhs: "<Space>n",
        rhs: ":bn<CR>",
        mode_prefix: "n",
        kind: MappingKind::NonRecursive,
        category: "Buffer Nav",
        default_enabled: false,
    },
    PresetDefinition {
        lhs: "<Space>p",
        rhs: ":bp<CR>",
        mode_prefix: "n",
        kind: MappingKind::NonRecursive,
        category: "Buffer Nav",
        default_enabled: false,
    },
    // ── Debugging ────────────────────────────────────────────────────────
    // Disabled by default: the `<Space>d` prefix collides with the `d`
    // (delete) operator, creating a `timeoutlen` delay on every `<Space>d`
    // keystroke. Users who want debug mappings can enable them in the
    // Mapping Dialog's Presets tab.
    PresetDefinition {
        lhs: "<Space>db",
        rhs: ":GodotBreakpoint<CR>",
        mode_prefix: "n",
        kind: MappingKind::NonRecursive,
        category: "Debugging",
        default_enabled: false,
    },
    PresetDefinition {
        lhs: "<Space>dc",
        rhs: ":GodotContinue<CR>",
        mode_prefix: "n",
        kind: MappingKind::NonRecursive,
        category: "Debugging",
        default_enabled: false,
    },
    PresetDefinition {
        lhs: "<Space>dn",
        rhs: ":GodotNext<CR>",
        mode_prefix: "n",
        kind: MappingKind::NonRecursive,
        category: "Debugging",
        default_enabled: false,
    },
    PresetDefinition {
        lhs: "<Space>di",
        rhs: ":GodotStepIn<CR>",
        mode_prefix: "n",
        kind: MappingKind::NonRecursive,
        category: "Debugging",
        default_enabled: false,
    },
    PresetDefinition {
        lhs: "<Space>do",
        rhs: ":GodotStepOut<CR>",
        mode_prefix: "n",
        kind: MappingKind::NonRecursive,
        category: "Debugging",
        default_enabled: false,
    },
    // ── Scene Control ────────────────────────────────────────────────────
    PresetDefinition {
        lhs: "<Space>r",
        rhs: ":run<CR>",
        mode_prefix: "n",
        kind: MappingKind::NonRecursive,
        category: "Scene",
        default_enabled: false,
    },
    PresetDefinition {
        lhs: "<Space>R",
        rhs: ":runcurrent<CR>",
        mode_prefix: "n",
        kind: MappingKind::NonRecursive,
        category: "Scene",
        default_enabled: false,
    },
    PresetDefinition {
        lhs: "<Space>S",
        rhs: ":stop<CR>",
        mode_prefix: "n",
        kind: MappingKind::NonRecursive,
        category: "Scene",
        default_enabled: false,
    },
    // ── Navigation ───────────────────────────────────────────────────────
    PresetDefinition {
        lhs: "<Space>e",
        rhs: ":FileSystem<CR>",
        mode_prefix: "n",
        kind: MappingKind::NonRecursive,
        category: "Navigation",
        default_enabled: false,
    },
    PresetDefinition {
        lhs: "<Space>i",
        rhs: ":Inspector<CR>",
        mode_prefix: "n",
        kind: MappingKind::NonRecursive,
        category: "Navigation",
        default_enabled: false,
    },
    PresetDefinition {
        lhs: "<Space>s",
        rhs: ":Script<CR>",
        mode_prefix: "n",
        kind: MappingKind::NonRecursive,
        category: "Navigation",
        default_enabled: false,
    },
    // ── Editor State ─────────────────────────────────────────────────────
    PresetDefinition {
        lhs: "<Space>z",
        rhs: ":zen<CR>",
        mode_prefix: "n",
        kind: MappingKind::NonRecursive,
        category: "Editor",
        default_enabled: false,
    },
    PresetDefinition {
        lhs: "<Space>Z",
        rhs: ":unzen<CR>",
        mode_prefix: "n",
        kind: MappingKind::NonRecursive,
        category: "Editor",
        default_enabled: false,
    },
    // ── Search ────────────────────────────────────────────────────────────
    PresetDefinition {
        lhs: "<Esc>",
        rhs: ":noh<CR>",
        mode_prefix: "n",
        kind: MappingKind::NonRecursive,
        category: "Search",
        default_enabled: false,
    },
];
