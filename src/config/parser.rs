//! Config file parser.
//!
//! Reads a `.godot-vimrc` file into a [`ConfigDocument`], preserving
//! structure for faithful roundtrip serialization.

use vim_core::keymap::MappingKind;

use super::types::{
    parse_mode_prefix, ConfigDocument, ConfigLine, MappingPayload, PanelPayload, ParsedMapping,
};

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
                // Tried AFTER the mapping parser, so every line that was a
                // disabled mapping yesterday still is. Without this arm a
                // toggled-off panel rule degrades to `ConfigLine::Comment`,
                // and the dialog can never toggle it back on — the failure is
                // invisible to a text-level round-trip, which re-emits a
                // comment verbatim.
                if let Ok(Some(parsed)) = super::panelmap::parse_panel_line(cmd_str) {
                    pending_preset = None;
                    lines.push(ConfigLine::PanelMap(Box::new(PanelPayload {
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

        // Placed after the marker is discarded and before `set `/`se `: a
        // panel line is not a mapping, so it must drop a stale preset marker
        // exactly as `set`/`let`/`Other` do. `try_parse_mapping_command`
        // above cannot shadow it — no entry in its COMMANDS table is a prefix
        // of `panelmap `. A malformed panel line deliberately falls through to
        // `Other` and is preserved verbatim; the loader is what reports it,
        // because only the loader knows the surfaces and the action registry.
        if let Ok(Some(parsed)) = super::panelmap::parse_panel_line(trimmed) {
            lines.push(ConfigLine::PanelMap(Box::new(PanelPayload {
                enabled: true,
                parsed,
            })));
            continue;
        }

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
/// recognized and fall through to `ConfigLine::Other`, and so does a line
/// of more than one command, which the dialog keeps as written.
fn try_parse_mapping_command(line: &str) -> Option<ParsedMapping> {
    let (prefix, noremap, rest) = parse_map_command_prefix(line)?;
    if vim_core::grammar::split_ex_pipeline(line).len() > 1 {
        return None;
    }

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

    // ── Panel directives ─────────────────────────────────────────────

    fn panel(line: &ConfigLine) -> &super::super::types::PanelPayload {
        match line {
            ConfigLine::PanelMap(payload) => payload,
            other => panic!("expected a PanelMap line, got {other:?}"),
        }
    }

    #[test]
    fn parse_enabled_panelmap_line() {
        let doc = parse_config("panelmap <physical> dock j godotvim.item.next\n");
        assert_eq!(doc.lines.len(), 1);
        let payload = panel(&doc.lines[0]);
        assert!(payload.enabled);
        assert_eq!(
            super::super::panelmap::render(&payload.parsed),
            "panelmap <physical> dock j godotvim.item.next"
        );
    }

    #[test]
    fn parse_panelunmap_line() {
        let doc = parse_config("panelunmap dock.filesystem a\n");
        assert!(panel(&doc.lines[0]).enabled);
    }

    #[test]
    fn parse_disabled_panelmap_line() {
        // The branch the design calls out: without the `" disabled:` arm this
        // is a `ConfigLine::Comment` and the dialog can never toggle it back.
        let doc = parse_config("\" disabled: panelmap dock j godotvim.item.next\n");
        assert_eq!(doc.lines.len(), 1);
        assert!(!panel(&doc.lines[0]).enabled);
    }

    #[test]
    fn disabled_panelmap_roundtrip() {
        use super::super::types::{ConfigDocument, PanelPayload};
        use super::super::{panelmap, writer};

        let parsed = panelmap::parse_panel_line("panelmap dock j godotvim.item.next")
            .expect("parses")
            .expect("is a panel line");
        let doc = ConfigDocument {
            lines: vec![ConfigLine::PanelMap(Box::new(PanelPayload {
                enabled: false,
                parsed,
            }))],
        };

        let serialized = writer::serialize(&doc);
        assert_eq!(
            serialized,
            "\" disabled: panelmap dock j godotvim.item.next\n"
        );

        let reparsed = parse_config(&serialized);
        assert_eq!(reparsed, doc, "a disabled panel line must survive intact");
    }

    #[test]
    fn a_malformed_panel_line_is_preserved_verbatim_rather_than_claimed() {
        // Warn-and-skip belongs to the loader, which knows the surfaces and
        // the registry. The parser's job is to not lose the user's text.
        let doc = parse_config("panelmap dock j :!rm -rf /\n");
        assert!(matches!(doc.lines[0], ConfigLine::Other(_)));
        let doc = parse_config("\" disabled: panelmap dock\n");
        assert!(matches!(doc.lines[0], ConfigLine::Comment(_)));
    }

    #[test]
    fn a_near_miss_on_the_verb_is_not_a_panel_line() {
        for text in [
            "panelmapping dock j godotvim.item.next\n",
            "panelmp dock j\n",
        ] {
            let doc = parse_config(text);
            assert!(matches!(doc.lines[0], ConfigLine::Other(_)), "{text}");
        }
    }

    #[test]
    fn a_preset_marker_never_attaches_to_a_panel_line() {
        // PRESETS holds no panel entries and panel rules are never
        // preset-managed, so the disabled-preset branch must stay untouched:
        // a commented panel line after a marker is a Comment, as it is today.
        let doc = parse_config("\" preset:disabled\n\" panelmap dock j godotvim.item.next\n");
        assert_eq!(doc.lines.len(), 1);
        assert!(matches!(doc.lines[0], ConfigLine::Comment(_)));

        // …and an ENABLED marker followed by a panel line drops the marker
        // rather than smuggling it onto the rule, exactly as `set` does.
        let doc = parse_config("\" preset:enabled\npanelmap dock j godotvim.item.next\nset tm=1\n");
        assert_eq!(doc.lines.len(), 2);
        assert!(panel(&doc.lines[0]).enabled);
        assert!(matches!(doc.lines[1], ConfigLine::Setting(_)));
    }

    #[test]
    fn a_panel_line_and_a_vim_mapping_coexist_in_one_file() {
        let doc = parse_config(
            "let mapleader = \" \"\n\
             nnoremap <Leader>ff <Action>(godotvim.fs.create)\n\
             panelmap dock.filesystem n godotvim.fs.create\n\
             panelunmap dock.filesystem a\n\
             set timeoutlen=500\n",
        );
        assert!(matches!(doc.lines[0], ConfigLine::Leader(_)));
        assert!(matches!(doc.lines[1], ConfigLine::Mapping(_)));
        assert!(matches!(doc.lines[2], ConfigLine::PanelMap(_)));
        assert!(matches!(doc.lines[3], ConfigLine::PanelMap(_)));
        assert!(matches!(doc.lines[4], ConfigLine::Setting(_)));
    }

    /// vim-core splits a map command at a bare `|` (help `map_bar`), so a
    /// line it splits is more than a mapping and is kept as written.
    #[test]
    fn a_line_of_several_commands_is_kept_verbatim() {
        for line in [
            "nnoremap x :echo 1|echo 2<CR>",
            "nnoremap <leader>y :echo \"a|b\"<CR>",
            "\" disabled: nnoremap x a|b",
        ] {
            let text = format!("{line}\n");
            let doc = parse_config(&text);
            assert!(
                matches!(
                    &doc.lines[..],
                    [ConfigLine::Other(kept) | ConfigLine::Comment(kept)] if kept == line
                ),
                "{line}: {doc:?}"
            );
            assert_eq!(super::super::writer::serialize(&doc), text);
        }
        for line in ["nnoremap x :echo 1<Bar>echo 2<CR>", "nnoremap x a\\|b"] {
            assert!(
                matches!(&parse_config(line).lines[..], [ConfigLine::Mapping(_)]),
                "{line}"
            );
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

/// The round-trip properties, in the two strengths that answer two different
/// questions.
///
/// The **document-level** property (`parse_config(serialize(&doc)) == doc`) is
/// the strong one, and it is the only one that can see a panel line losing its
/// identity: `ConfigLine::Comment` and `ConfigLine::Other` both store their raw
/// text and the writer re-emits both verbatim, so a `" disabled: panelmap …`
/// that decayed into a `Comment` still round-trips *as text*. Only equality of
/// typed documents catches it.
///
/// The **text-level fixpoint** (`parse(write(parse(x))) == parse(x)`) is the
/// weaker one, retained because it runs over raw text the typed generator
/// cannot express — stale preset markers, whitespace, near-miss verbs — and so
/// catches writer/parser drift on the other six variants.
#[cfg(test)]
mod roundtrip_props {
    use proptest::prelude::*;
    use vim_core::grammar::MapModePrefix;
    use vim_core::keymap::MappingKind;

    use super::super::types::{
        ConfigDocument, ConfigLine, MappingPayload, PanelPayload, ParsedMapping,
    };
    use super::super::{panelmap, writer};
    use super::parse_config;

    /// Comments that are neither preset markers nor a `" disabled: ` carrier
    /// for a real directive. Those three shapes are *deliberately*
    /// re-classified, so generating them would assert the opposite of the
    /// intended behaviour rather than exercising the round trip.
    const COMMENTS: &[&str] = &[
        "\"",
        "\" GodotVim Configuration",
        "\" --- User Mappings ---",
        "\" disabled: not actually a command",
        "\" preset is a word; preset:enabled is a marker",
    ];
    const SETTINGS: &[&str] = &[
        "set timeoutlen=500",
        "se tm=750",
        "set scrolloff=5",
        "set number",
    ];
    const LEADERS: &[&str] = &["let mapleader = \" \"", "let g:mapleader = \",\""];
    /// Lines that match none of the six classifiers, including three near
    /// misses that must stay `Other`.
    const OTHERS: &[&str] = &[
        "some_custom_thing",
        "colorscheme habamax",
        "setlocal wrap",
        "letmapleader",
        "panelmapping dock j godotvim.item.next",
        "panelmap dock j :!rm -rf /",
    ];
    const LHS: &[&str] = &["jk", "<Leader>w", "<C-a>", "gg", "x"];
    const RHS: &[&str] = &["<Esc>", ":save<CR>", "gj", "<Action>(godotvim.fs.create)"];
    const PRESET_IDS: &[Option<&str>] = &[None, Some(""), Some("preset.id")];
    const PANEL_LINES: &[&str] = &[
        "panelmap <physical> <void> <norepeat> panel <C-h> godotvim.focus.left",
        "panelmap dock j godotvim.item.next",
        "panelmap <shift> searchbox <CR> godotvim.search.accept",
        "panelmap dock.filesystem dd godotvim.fs.delete",
        "panelmap dock <C-d> godotvim.item.next count=10",
        "panelmap panel <C-h> native",
        "panelmap dock.filesystem <C-r> <Shortcut>(filesystem_dock/rename)",
        "panelunmap dock.filesystem a",
        "panelmap <nowait> dock.filesystem d godotvim.fs.delete",
        "panelmap dock x a.b flag=1 depth=-3",
    ];
    const MODES: &[MapModePrefix] = &[
        MapModePrefix::All,
        MapModePrefix::Normal,
        MapModePrefix::Insert,
        MapModePrefix::Visual,
        MapModePrefix::Operator,
        MapModePrefix::Command,
    ];

    fn mapping_line() -> impl Strategy<Value = ConfigLine> {
        (
            proptest::sample::select(PRESET_IDS),
            any::<bool>(),
            proptest::sample::select(LHS),
            proptest::sample::select(RHS),
            proptest::sample::select(MODES),
            any::<bool>(),
        )
            .prop_map(|(preset_id, enabled, lhs, rhs, modes, noremap)| {
                ConfigLine::Mapping(Box::new(MappingPayload {
                    preset_id: preset_id.map(str::to_string),
                    enabled,
                    parsed: ParsedMapping {
                        lhs: lhs.to_string(),
                        rhs: rhs.to_string(),
                        modes,
                        kind: if noremap {
                            MappingKind::NonRecursive
                        } else {
                            MappingKind::Recursive
                        },
                    },
                }))
            })
    }

    fn panel_line() -> impl Strategy<Value = ConfigLine> {
        (proptest::sample::select(PANEL_LINES), any::<bool>()).prop_map(|(text, enabled)| {
            let parsed = panelmap::parse_panel_line(text)
                .expect("the corpus parses")
                .expect("the corpus is claimed");
            ConfigLine::PanelMap(Box::new(PanelPayload { enabled, parsed }))
        })
    }

    fn config_line() -> impl Strategy<Value = ConfigLine> {
        prop_oneof![
            Just(ConfigLine::BlankLine),
            proptest::sample::select(COMMENTS).prop_map(|s| ConfigLine::Comment(s.to_string())),
            proptest::sample::select(SETTINGS).prop_map(|s| ConfigLine::Setting(s.to_string())),
            proptest::sample::select(LEADERS).prop_map(|s| ConfigLine::Leader(s.to_string())),
            proptest::sample::select(OTHERS).prop_map(|s| ConfigLine::Other(s.to_string())),
            mapping_line(),
            panel_line(),
        ]
    }

    fn config_document() -> impl Strategy<Value = ConfigDocument> {
        proptest::collection::vec(config_line(), 0..10).prop_map(|lines| ConfigDocument { lines })
    }

    /// Raw text, including shapes the typed generator cannot produce: stale
    /// preset markers, sloppy whitespace, and near-miss verbs.
    fn config_text() -> impl Strategy<Value = String> {
        const RAW: &[&str] = &[
            "",
            "   ",
            "\" a comment",
            "\" preset:enabled",
            "\" preset:disabled",
            "\" preset:enabled preset.id",
            "\" nnoremap jj <Esc>",
            "\" panelmap dock j godotvim.item.next",
            "\" disabled: nnoremap jk <Esc>",
            "\" disabled: panelmap dock j godotvim.item.next",
            "\" disabled: panelunmap dock j",
            "\" disabled: panelmap dock",
            "nnoremap jk <Esc>",
            "  nmap   j    gj  ",
            "let mapleader = \" \"",
            "set timeoutlen=500",
            "panelmap dock j godotvim.item.next",
            "   panelmap  <void>  <physical>  dock  k  godotvim.item.prev  ",
            "panelunmap dock.filesystem a",
            "panelmap panel <C-h> native",
            "panelmap dock j :!rm -rf /",
            "panelmapping dock j godotvim.item.next",
            "some_custom_thing",
            "nnoremap x :echo 1<Bar>echo 2<CR>",
            "nnoremap x :echo 1|echo 2<CR>",
            "\" disabled: nnoremap x a|b",
        ];
        proptest::collection::vec(proptest::sample::select(RAW), 0..14).prop_map(|lines| {
            let mut text = lines.join("\n");
            text.push('\n');
            text
        })
    }

    proptest! {
        /// The strong property. Breaking either parser branch fails this.
        #[test]
        fn a_typed_document_survives_serialize_then_parse(doc in config_document()) {
            prop_assert_eq!(parse_config(&writer::serialize(&doc)), doc);
        }

        /// The weak property, over raw text: parse -> write -> parse is a
        /// fixpoint, and so is the text from the first write onwards.
        #[test]
        fn parse_write_parse_is_a_fixpoint(text in config_text()) {
            let first = parse_config(&text);
            let rewritten = writer::serialize(&first);
            let second = parse_config(&rewritten);
            prop_assert_eq!(&second, &first);
            prop_assert_eq!(writer::serialize(&second), rewritten);
        }
    }
}
