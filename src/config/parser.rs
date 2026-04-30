//! Config file parser.
//!
//! Reads a `.godot-vimrc` file into a [`ConfigDocument`], preserving
//! structure for faithful roundtrip serialization.

use vim_core::keymap::MappingKind;

use super::types::{parse_mode_prefix, ConfigDocument, ConfigLine, MappingPayload, ParsedMapping};

#[cfg(test)]
use vim_core::grammar::MapModePrefix;

/// Parse config file text into a structured [`ConfigDocument`].
///
/// Every line type (comments, blanks, mappings, `:set`, `:let mapleader`,
/// preset markers, unknown) is preserved for faithful roundtrip serialization.
pub(crate) fn parse_config(text: &str) -> ConfigDocument {
    let mut lines = Vec::new();
    let mut pending_preset: Option<(String, bool)> = None;

    for raw_line in text.lines() {
        let trimmed = raw_line.trim();

        // Preset markers are NOT stored as Comment lines. The Mapping variant
        // carries preset metadata, and serialize() reconstructs the marker.
        // Storing both would double the marker on every save-parse-serialize cycle.
        if let Some(marker) = parse_preset_marker(trimmed) {
            pending_preset = Some(marker);
            continue;
        }

        if trimmed.is_empty() {
            pending_preset = None;
            lines.push(ConfigLine::BlankLine);
            continue;
        }

        if trimmed.starts_with('"') {
            // `" disabled: nnoremap jk <Esc>` — self-contained marker for user mappings.
            if let Some(cmd_str) = trimmed.strip_prefix("\" disabled:") {
                let cmd_str = cmd_str.trim_start();
                if let Some(parsed) = try_parse_mapping_command(cmd_str) {
                    pending_preset = None;
                    lines.push(ConfigLine::Mapping(Box::new(MappingPayload {
                        preset_id: None,
                        enabled: false,
                        parsed,
                    })));
                    continue;
                }
            }

            // Disabled presets are commented-out mapping lines following a marker.
            if let Some((ref preset_id, false)) = pending_preset {
                let uncommented = trimmed.trim_start_matches('"').trim();
                if let Some(parsed) = try_parse_mapping_command(uncommented) {
                    let preset_id = preset_id.clone();
                    pending_preset = None;
                    lines.push(ConfigLine::Mapping(Box::new(MappingPayload {
                        preset_id: Some(preset_id),
                        enabled: false,
                        parsed,
                    })));
                    continue;
                }
            }
            pending_preset = None;
            lines.push(ConfigLine::Comment(raw_line.to_string()));
            continue;
        }

        if let Some(parsed) = try_parse_mapping_command(trimmed) {
            let (preset_id, enabled) = if let Some((id, is_enabled)) = pending_preset.take() {
                (Some(id), is_enabled)
            } else {
                (None, true)
            };
            lines.push(ConfigLine::Mapping(Box::new(MappingPayload {
                preset_id,
                enabled,
                parsed,
            })));
            continue;
        }

        // Discard stale preset marker so it doesn't attach to a non-mapping line.
        pending_preset = None;

        if trimmed.starts_with("set ") || trimmed.starts_with("se ") {
            lines.push(ConfigLine::Setting(raw_line.to_string()));
            continue;
        }

        if trimmed.starts_with("let ") && trimmed.contains("mapleader") {
            lines.push(ConfigLine::Leader(raw_line.to_string()));
            continue;
        }

        lines.push(ConfigLine::Other(raw_line.to_string()));
    }

    ConfigDocument { lines }
}

/// Parse a `" preset:enabled [id]` / `" preset:disabled [id]` marker.
/// The inline ID is optional; when absent, identity comes from the next
/// mapping line's LHS.
fn parse_preset_marker(line: &str) -> Option<(String, bool)> {
    let content = line.strip_prefix('"')?.trim();
    if let Some(rest) = content.strip_prefix("preset:enabled") {
        let id = rest.trim().to_string();
        Some((id, true))
    } else if let Some(rest) = content.strip_prefix("preset:disabled") {
        let id = rest.trim().to_string();
        Some((id, false))
    } else {
        None
    }
}

/// Try to parse a line as a mapping command. Unmap variants are not yet
/// recognized and fall through to `ConfigLine::Other`.
fn try_parse_mapping_command(line: &str) -> Option<ParsedMapping> {
    let (prefix, noremap, rest) = parse_map_command_prefix(line)?;

    let rest = rest.trim();
    if rest.is_empty() {
        return None;
    }

    let (lhs, rhs) = split_at_first_whitespace(rest)?;

    let modes = parse_mode_prefix(prefix)?;

    Some(ParsedMapping {
        lhs: lhs.to_string(),
        rhs: rhs.to_string(),
        modes,
        kind: if noremap {
            MappingKind::NonRecursive
        } else {
            MappingKind::Recursive
        },
    })
}

/// Returns `(mode_prefix, is_noremap, rest_of_line)`.
fn parse_map_command_prefix(line: &str) -> Option<(&'static str, bool, &str)> {
    // Longer prefixes first so "nnoremap" matches before "nmap".
    const COMMANDS: &[(&str, &str, bool)] = &[
        ("nnoremap ", "n", true),
        ("inoremap ", "i", true),
        ("vnoremap ", "v", true),
        ("onoremap ", "o", true),
        ("cnoremap ", "c", true),
        ("noremap ", "", true),
        ("nmap ", "n", false),
        ("imap ", "i", false),
        ("vmap ", "v", false),
        ("omap ", "o", false),
        ("cmap ", "c", false),
        ("map ", "", false),
    ];

    for &(cmd, prefix, noremap) in COMMANDS {
        if let Some(rest) = line.strip_prefix(cmd) {
            return Some((prefix, noremap, rest));
        }
    }
    None
}

fn split_at_first_whitespace(s: &str) -> Option<(&str, &str)> {
    let idx = s.find(char::is_whitespace)?;
    let lhs = &s[..idx];
    let rhs = s[idx..].trim_start();
    if rhs.is_empty() {
        None
    } else {
        Some((lhs, rhs))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_empty_config() {
        let doc = parse_config("");
        assert!(doc.lines.is_empty());
    }

    #[test]
    fn parse_comments_and_blanks() {
        let text = "\" This is a comment\n\n\" Another comment\n";
        let doc = parse_config(text);
        assert_eq!(doc.lines.len(), 3);
        assert!(matches!(doc.lines[0], ConfigLine::Comment(_)));
        assert!(matches!(doc.lines[1], ConfigLine::BlankLine));
        assert!(matches!(doc.lines[2], ConfigLine::Comment(_)));
    }

    #[test]
    fn parse_simple_mapping() {
        let text = "nnoremap jk <Esc>\n";
        let doc = parse_config(text);
        assert_eq!(doc.lines.len(), 1);
        if let ConfigLine::Mapping(payload) = &doc.lines[0] {
            assert!(payload.preset_id.is_none());
            assert!(payload.enabled);
            assert_eq!(payload.parsed.lhs, "jk");
            assert_eq!(payload.parsed.rhs, "<Esc>");
            assert_eq!(payload.parsed.modes, MapModePrefix::Normal);
            assert_eq!(payload.parsed.kind, MappingKind::NonRecursive);
        } else {
            panic!("Expected Mapping line");
        }
    }

    #[test]
    fn parse_recursive_mapping() {
        let text = "nmap j gj\n";
        let doc = parse_config(text);
        if let ConfigLine::Mapping(payload) = &doc.lines[0] {
            assert_eq!(payload.parsed.lhs, "j");
            assert_eq!(payload.parsed.rhs, "gj");
            assert_eq!(payload.parsed.kind, MappingKind::Recursive);
        } else {
            panic!("Expected Mapping line");
        }
    }

    #[test]
    fn parse_insert_mode_mapping() {
        let text = "inoremap jk <Esc>\n";
        let doc = parse_config(text);
        if let ConfigLine::Mapping(payload) = &doc.lines[0] {
            assert_eq!(payload.parsed.modes, MapModePrefix::Insert);
        } else {
            panic!("Expected Mapping line");
        }
    }

    #[test]
    fn parse_generic_map() {
        let text = "noremap x dd\n";
        let doc = parse_config(text);
        if let ConfigLine::Mapping(payload) = &doc.lines[0] {
            assert_eq!(payload.parsed.modes, MapModePrefix::All);
            assert_eq!(payload.parsed.kind, MappingKind::NonRecursive);
        } else {
            panic!("Expected Mapping line");
        }
    }

    #[test]
    fn parse_set_command() {
        let text = "set timeoutlen=500\n";
        let doc = parse_config(text);
        assert!(matches!(doc.lines[0], ConfigLine::Setting(_)));
    }

    #[test]
    fn parse_let_mapleader() {
        let text = "let mapleader = \" \"\n";
        let doc = parse_config(text);
        assert!(matches!(doc.lines[0], ConfigLine::Leader(_)));
    }

    #[test]
    fn parse_preset_enabled() {
        let text = "\" preset:enabled\nnnoremap <Space>w :save<CR>\n";
        let doc = parse_config(text);
        // Marker is consumed by parser (not stored as Comment); only the Mapping remains.
        assert_eq!(doc.lines.len(), 1);
        if let ConfigLine::Mapping(payload) = &doc.lines[0] {
            assert!(payload.preset_id.is_some());
            assert!(payload.enabled);
            assert_eq!(payload.parsed.lhs, "<Space>w");
            assert_eq!(payload.parsed.rhs, ":save<CR>");
        } else {
            panic!("Expected Mapping line");
        }
    }

    #[test]
    fn parse_preset_disabled() {
        let text = "\" preset:disabled\n\" nnoremap jj <Esc>\n";
        let doc = parse_config(text);
        // Marker is consumed; only the disabled Mapping remains.
        assert_eq!(doc.lines.len(), 1);
        if let ConfigLine::Mapping(payload) = &doc.lines[0] {
            assert!(payload.preset_id.is_some());
            assert!(!payload.enabled);
            assert_eq!(payload.parsed.lhs, "jj");
            assert_eq!(payload.parsed.rhs, "<Esc>");
        } else {
            panic!("Expected disabled Mapping line, got {:?}", doc.lines[0]);
        }
    }

    #[test]
    fn parse_unknown_line_preserved() {
        let text = "some_custom_thing\n";
        let doc = parse_config(text);
        assert!(matches!(doc.lines[0], ConfigLine::Other(_)));
    }

    #[test]
    fn parse_full_config() {
        let text = "\
\" GodotVim Configuration
let mapleader = \" \"
set timeoutlen=500

\" --- User Mappings ---
nnoremap <Leader>w :save<CR>
inoremap jk <Esc>

\" --- Presets ---
\" preset:enabled
nnoremap <Space>r :run<CR>
\" preset:disabled
\" inoremap jj <Esc>
";
        let doc = parse_config(text);

        // Count user mappings (no preset_id) and preset mappings (has preset_id).
        let user_mappings: Vec<_> = doc
            .lines
            .iter()
            .filter(|l| {
                matches!(l,
                    ConfigLine::Mapping(p) if p.preset_id.is_none()
                )
            })
            .collect();
        let preset_mappings: Vec<_> = doc
            .lines
            .iter()
            .filter_map(|l| match l {
                ConfigLine::Mapping(p) if p.preset_id.is_some() => Some(p.enabled),
                _ => None,
            })
            .collect();

        assert_eq!(user_mappings.len(), 2);
        assert_eq!(preset_mappings.len(), 2);
        assert!(preset_mappings[0]); // first preset is enabled
        assert!(!preset_mappings[1]); // second is disabled
    }

    #[test]
    fn parse_disabled_user_mapping() {
        let text = "\" disabled: nnoremap jk <Esc>\n";
        let doc = parse_config(text);
        assert_eq!(doc.lines.len(), 1);
        if let ConfigLine::Mapping(payload) = &doc.lines[0] {
            assert!(payload.preset_id.is_none());
            assert!(!payload.enabled);
            assert_eq!(payload.parsed.lhs, "jk");
            assert_eq!(payload.parsed.rhs, "<Esc>");
            assert_eq!(payload.parsed.modes, MapModePrefix::Normal);
            assert_eq!(payload.parsed.kind, MappingKind::NonRecursive);
        } else {
            panic!("Expected disabled Mapping line, got {:?}", doc.lines[0]);
        }
    }

    #[test]
    fn disabled_user_mapping_roundtrip() {
        use super::super::types::ConfigDocument;
        use super::super::writer;

        // Build a doc with a disabled user mapping.
        let doc = ConfigDocument {
            lines: vec![ConfigLine::Mapping(Box::new(MappingPayload {
                preset_id: None,
                enabled: false,
                parsed: ParsedMapping {
                    lhs: "jk".to_string(),
                    rhs: "<Esc>".to_string(),
                    modes: MapModePrefix::Normal,
                    kind: MappingKind::NonRecursive,
                },
            }))],
        };

        let serialized = writer::serialize(&doc);
        assert_eq!(serialized, "\" disabled: nnoremap jk <Esc>\n");

        // Parse back and verify roundtrip fidelity.
        let reparsed = parse_config(&serialized);
        assert_eq!(reparsed.lines.len(), 1);
        if let ConfigLine::Mapping(payload) = &reparsed.lines[0] {
            assert!(payload.preset_id.is_none());
            assert!(!payload.enabled);
            assert_eq!(payload.parsed.lhs, "jk");
            assert_eq!(payload.parsed.rhs, "<Esc>");
        } else {
            panic!("Roundtrip failed");
        }
    }
}
