//! Godot mapping loader.
//!
//! This module is the I/O shell for mapping configuration and produces a pure
//! `MappingStore` for runtime lookup.

use std::fmt;

use crate::bridge::settings::presets::get_recommended_mappings;
use crate::bridge::settings::VimSettings;
use vim_core::inputs::mapping::{
    parse_key_sequence, KeyMapping, MappedAction, MappingMode, MappingOwner, MappingStore,
};

/// Mapping schema errors for hard-cut unified mapping configuration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MappingSchemaError {
    LegacyKeysDetected(Vec<String>),
    ParseError {
        from: String,
        to: String,
        mode: char,
    },
}

impl fmt::Display for MappingSchemaError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::LegacyKeysDetected(keys) => write!(
                f,
                "Legacy mapping dictionaries are not supported ({}). Use `plugins/GodotVim/mapping/all` only.",
                keys.join(", ")
            ),
            Self::ParseError { from, to, mode } => write!(
                f,
                "Invalid unified mapping entry: from='{}' to='{}' mode='{}'",
                from, to, mode
            ),
        }
    }
}

impl std::error::Error for MappingSchemaError {}

/// Shell layer for loading mappings from settings.
pub struct GodotMappingLoader;

impl GodotMappingLoader {
    /// Load canonical unified mappings and system presets into a pure store.
    pub fn load() -> Result<MappingStore, MappingSchemaError> {
        let legacy_sets = Self::detect_legacy_mapping_dicts();
        if !legacy_sets.is_empty() {
            return Err(MappingSchemaError::LegacyKeysDetected(legacy_sets));
        }

        let mut preset_mappings: Vec<KeyMapping> = Vec::new();

        for preset in get_recommended_mappings() {
            if preset.modes.is_empty() {
                continue;
            }

            let from_keys = parse_key_sequence(&preset.from);
            if from_keys.is_empty() {
                continue;
            }

            let action = if preset.to.starts_with(':') {
                MappedAction::Command(preset.to.trim_start_matches(':').to_string())
            } else {
                MappedAction::Keys(parse_key_sequence(&preset.to))
            };

            let mut modes = Vec::new();
            for c in preset.modes.chars() {
                match c {
                    'n' => modes.push(MappingMode::Normal),
                    'i' => modes.push(MappingMode::Insert),
                    'v' => modes.push(MappingMode::Visual),
                    'g' => modes.push(MappingMode::Global),
                    _ => {}
                }
            }

            preset_mappings.push(KeyMapping {
                from: from_keys,
                to: action,
                modes,
                owner: MappingOwner::System,
            });
        }

        let unified = VimSettings::all_mappings();
        let unified_keys: std::collections::HashSet<Vec<vim_core::inputs::VimKey>> = unified
            .iter()
            .map(|(from, _, _)| parse_key_sequence(from))
            .collect();

        let mut mappings: Vec<KeyMapping> = preset_mappings
            .into_iter()
            .filter(|m| !unified_keys.contains(&m.from))
            .collect();

        for (from, to, modes_str) in &unified {
            for mode_char in modes_str.chars() {
                let mode = match mode_char {
                    'n' => MappingMode::Normal,
                    'i' => MappingMode::Insert,
                    'v' => MappingMode::Visual,
                    'g' => MappingMode::Global,
                    _ => continue,
                };

                let Some(mapping) = MappingStore::parse_mapping(from, to, mode) else {
                    return Err(MappingSchemaError::ParseError {
                        from: from.clone(),
                        to: to.clone(),
                        mode: mode_char,
                    });
                };
                mappings.push(mapping);
            }
        }

        Ok(MappingStore::from_mappings(mappings))
    }

    fn detect_legacy_mapping_dicts() -> Vec<String> {
        [
            ("imap", VimSettings::imap()),
            ("nmap", VimSettings::nmap()),
            ("vmap", VimSettings::vmap()),
            ("gmap", VimSettings::gmap()),
        ]
        .into_iter()
        .filter_map(|(name, entries)| {
            if entries.is_empty() {
                None
            } else {
                Some(format!("{name}={}", entries.len()))
            }
        })
        .collect()
    }
}
