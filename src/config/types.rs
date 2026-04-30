//! Data types for the config file system.
//!
//! These types model the content of a `.godot-vimrc` file, preserving
//! structure for faithful roundtrip serialization.

use bitflags::bitflags;
use vim_core::grammar::MapModePrefix;
use vim_core::keymap::MappingKind;

bitflags! {
    /// Compact set of Vim modes, used for mapping mode checkboxes.
    ///
    /// Each bit represents one mode. The `NVO` convenience constant matches
    /// Vim's `:map` semantics (normal + visual + operator-pending).
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
    pub(crate) struct ModeSet: u8 {
        const NORMAL   = 1 << 0;
        const INSERT   = 1 << 1;
        const VISUAL   = 1 << 2;
        const OPERATOR = 1 << 3;
        const COMMAND  = 1 << 4;
        const NVO = Self::NORMAL.bits() | Self::VISUAL.bits() | Self::OPERATOR.bits();
    }
}

pub(crate) fn parse_mode_prefix(prefix: &str) -> Option<MapModePrefix> {
    match prefix {
        "n" => Some(MapModePrefix::Normal),
        "i" => Some(MapModePrefix::Insert),
        "v" => Some(MapModePrefix::Visual),
        "o" => Some(MapModePrefix::Operator),
        "c" => Some(MapModePrefix::Command),
        "" => Some(MapModePrefix::All),
        _ => None,
    }
}

pub(crate) fn mode_prefix_str(mode: MapModePrefix) -> &'static str {
    match mode {
        MapModePrefix::All => "",
        MapModePrefix::Normal => "n",
        MapModePrefix::Insert => "i",
        MapModePrefix::Visual => "v",
        MapModePrefix::Operator => "o",
        MapModePrefix::Command => "c",
        _ => {
            log::error!(
                "mode_prefix_str: unhandled MapModePrefix variant {:?}",
                mode
            );
            ""
        }
    }
}

/// Display string for dialog UI. `All` shows "N V O" because `:map` covers
/// normal + visual + operator-pending (not insert or command).
pub(crate) fn mode_prefix_display(mode: MapModePrefix) -> &'static str {
    match mode {
        MapModePrefix::All => "N V O",
        MapModePrefix::Normal => "N",
        MapModePrefix::Insert => "I",
        MapModePrefix::Visual => "V",
        MapModePrefix::Operator => "O",
        MapModePrefix::Command => "C",
        _ => {
            log::error!(
                "mode_prefix_display: unhandled MapModePrefix variant {:?}",
                mode
            );
            "?"
        }
    }
}

/// Convert a single `MapModePrefix` into the `ModeSet` flags it covers.
///
/// `MapModePrefix::All` maps to `NVO` (normal + visual + operator-pending),
/// matching Vim's `:map` semantics where `:map` does NOT include insert or
/// command modes.
pub(crate) fn mode_set_from_prefix(prefix: MapModePrefix) -> ModeSet {
    match prefix {
        MapModePrefix::All => ModeSet::NVO,
        MapModePrefix::Normal => ModeSet::NORMAL,
        MapModePrefix::Insert => ModeSet::INSERT,
        MapModePrefix::Visual => ModeSet::VISUAL,
        MapModePrefix::Operator => ModeSet::OPERATOR,
        MapModePrefix::Command => ModeSet::COMMAND,
        _ => {
            log::error!(
                "mode_set_from_prefix: unhandled MapModePrefix variant {:?}",
                prefix
            );
            ModeSet::empty()
        }
    }
}

/// Convert a `ModeSet` to the minimal set of `MapModePrefix` values.
///
/// Returns a `Vec` because arbitrary flag combos (e.g., normal + insert)
/// require separate mapping commands -- Vim has no single prefix for them.
///
/// `{NORMAL, VISUAL, OPERATOR}` collapses to `All` (`:map`), matching Vim
/// semantics where `:map` covers N+V+O but NOT insert or command.
///
/// Empty `Vec` means no mode selected -- caller decides whether to disable
/// or delete.
pub(crate) fn mode_prefixes_from_set(modes: ModeSet) -> Vec<MapModePrefix> {
    let has_all_nvo = modes.contains(ModeSet::NVO);

    let mut prefixes = Vec::new();

    if has_all_nvo && !modes.contains(ModeSet::INSERT) {
        prefixes.push(MapModePrefix::All);
    } else if has_all_nvo && modes.contains(ModeSet::INSERT) {
        prefixes.push(MapModePrefix::All);
        prefixes.push(MapModePrefix::Insert);
    } else {
        if modes.contains(ModeSet::NORMAL) {
            prefixes.push(MapModePrefix::Normal);
        }
        if modes.contains(ModeSet::INSERT) {
            prefixes.push(MapModePrefix::Insert);
        }
        if modes.contains(ModeSet::VISUAL) {
            prefixes.push(MapModePrefix::Visual);
        }
        if modes.contains(ModeSet::OPERATOR) {
            prefixes.push(MapModePrefix::Operator);
        }
    }

    if modes.contains(ModeSet::COMMAND) {
        prefixes.push(MapModePrefix::Command);
    }

    prefixes
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ParsedMapping {
    pub(crate) lhs: String,
    pub(crate) rhs: String,
    pub(crate) modes: MapModePrefix,
    pub(crate) kind: MappingKind,
}

/// Boxed payload for [`ConfigLine::Mapping`]. Without boxing, the Mapping
/// variant (~89 bytes) would inflate all other ConfigLine variants to the
/// same size.
#[derive(Debug, Clone)]
pub(crate) struct MappingPayload {
    /// `Some` for preset-managed mappings; `None` for user-defined.
    pub(crate) preset_id: Option<String>,
    pub(crate) enabled: bool,
    pub(crate) parsed: ParsedMapping,
}

/// A single line in the config file, preserving structure for roundtrip.
#[derive(Debug, Clone)]
pub(crate) enum ConfigLine {
    Comment(String),
    BlankLine,
    Mapping(Box<MappingPayload>),
    Setting(String),
    Leader(String),
    /// Unrecognized lines -- preserved verbatim for roundtrip fidelity.
    Other(String),
}

/// Structured representation of a `.godot-vimrc` file, preserving line
/// ordering for roundtrip serialization.
#[derive(Debug, Clone)]
pub(crate) struct ConfigDocument {
    pub(crate) lines: Vec<ConfigLine>,
}

impl ConfigDocument {
    pub(crate) fn add_user_mapping(&mut self, mapping: ParsedMapping) {
        self.lines
            .push(ConfigLine::Mapping(Box::new(MappingPayload {
                preset_id: None,
                enabled: true,
                parsed: mapping,
            })));
    }

    pub(crate) fn timeoutlen(&self) -> Option<u32> {
        for line in &self.lines {
            if let ConfigLine::Setting(text) = line {
                if let Some(val) = parse_timeoutlen_value(text) {
                    return Some(val);
                }
            }
        }
        None
    }

    /// Update existing `set timeoutlen=` line, or insert one after the leader
    /// line (or at the top if no leader exists).
    pub(crate) fn set_timeoutlen(&mut self, ms: u32) {
        for line in &mut self.lines {
            if let ConfigLine::Setting(text) = line {
                if is_timeoutlen_setting(text) {
                    *text = format!("set timeoutlen={ms}");
                    return;
                }
            }
        }

        let insert_pos = self
            .lines
            .iter()
            .position(|l| matches!(l, ConfigLine::Leader(_)))
            .map_or(0, |i| i + 1);
        self.lines.insert(
            insert_pos,
            ConfigLine::Setting(format!("set timeoutlen={ms}")),
        );
    }
}

/// All accepted spellings of `set timeoutlen=` (including Vim's `tm` abbreviation).
const TIMEOUTLEN_PREFIXES: &[&str] = &["set timeoutlen=", "se timeoutlen=", "set tm=", "se tm="];

fn is_timeoutlen_setting(text: &str) -> bool {
    let trimmed = text.trim();
    TIMEOUTLEN_PREFIXES.iter().any(|p| trimmed.starts_with(p))
}

fn parse_timeoutlen_value(text: &str) -> Option<u32> {
    let trimmed = text.trim();
    let after_eq = TIMEOUTLEN_PREFIXES
        .iter()
        .find_map(|p| trimmed.strip_prefix(p))?;
    after_eq.trim().parse::<u32>().ok()
}

pub(crate) fn mapping_to_vim_command(m: &ParsedMapping) -> String {
    let noremap_str = if m.kind == MappingKind::NonRecursive {
        "noremap"
    } else {
        "map"
    };
    let prefix = mode_prefix_str(m.modes);
    format!("{prefix}{noremap_str} {} {}", m.lhs, m.rhs)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_doc_with_setting(setting: &str) -> ConfigDocument {
        ConfigDocument {
            lines: vec![
                ConfigLine::Leader("let mapleader = \" \"".to_string()),
                ConfigLine::Setting(setting.to_string()),
            ],
        }
    }

    #[test]
    fn timeoutlen_reads_value() {
        let doc = make_doc_with_setting("set timeoutlen=500");
        assert_eq!(doc.timeoutlen(), Some(500));
    }

    #[test]
    fn timeoutlen_reads_abbreviation() {
        let doc = make_doc_with_setting("set tm=750");
        assert_eq!(doc.timeoutlen(), Some(750));
    }

    #[test]
    fn timeoutlen_returns_none_when_missing() {
        let doc = ConfigDocument {
            lines: vec![ConfigLine::Leader("let mapleader = \" \"".to_string())],
        };
        assert_eq!(doc.timeoutlen(), None);
    }

    #[test]
    fn timeoutlen_ignores_unrelated_settings() {
        let doc = make_doc_with_setting("set number");
        assert_eq!(doc.timeoutlen(), None);
    }

    #[test]
    fn set_timeoutlen_updates_existing() {
        let mut doc = make_doc_with_setting("set timeoutlen=500");
        doc.set_timeoutlen(2000);
        assert_eq!(doc.timeoutlen(), Some(2000));
        // Verify the line was updated, not duplicated.
        let setting_count = doc
            .lines
            .iter()
            .filter(|l| matches!(l, ConfigLine::Setting(_)))
            .count();
        assert_eq!(setting_count, 1);
    }

    #[test]
    fn set_timeoutlen_inserts_after_leader() {
        let mut doc = ConfigDocument {
            lines: vec![
                ConfigLine::Comment("\" header".to_string()),
                ConfigLine::Leader("let mapleader = \" \"".to_string()),
                ConfigLine::BlankLine,
            ],
        };
        doc.set_timeoutlen(800);
        assert_eq!(doc.timeoutlen(), Some(800));
        // Should be inserted at index 2 (after leader at index 1).
        assert!(matches!(doc.lines[2], ConfigLine::Setting(_)));
    }

    #[test]
    fn set_timeoutlen_inserts_at_top_when_no_leader() {
        let mut doc = ConfigDocument {
            lines: vec![ConfigLine::Comment("\" just a comment".to_string())],
        };
        doc.set_timeoutlen(300);
        assert_eq!(doc.timeoutlen(), Some(300));
        assert!(matches!(doc.lines[0], ConfigLine::Setting(_)));
    }

    #[test]
    fn mapping_to_vim_command_normal_noremap() {
        let m = ParsedMapping {
            lhs: "jk".to_string(),
            rhs: "<Esc>".to_string(),
            modes: MapModePrefix::Normal,
            kind: MappingKind::NonRecursive,
        };
        assert_eq!(mapping_to_vim_command(&m), "nnoremap jk <Esc>");
    }

    #[test]
    fn mapping_to_vim_command_all_map() {
        let m = ParsedMapping {
            lhs: "<Leader>w".to_string(),
            rhs: ":save<CR>".to_string(),
            modes: MapModePrefix::All,
            kind: MappingKind::Recursive,
        };
        assert_eq!(mapping_to_vim_command(&m), "map <Leader>w :save<CR>");
    }

    #[test]
    fn mode_prefixes_empty_returns_empty() {
        let prefixes = mode_prefixes_from_set(ModeSet::empty());
        assert!(prefixes.is_empty());
    }

    #[test]
    fn mode_prefixes_single_mode() {
        assert_eq!(
            mode_prefixes_from_set(ModeSet::NORMAL),
            vec![MapModePrefix::Normal]
        );
        assert_eq!(
            mode_prefixes_from_set(ModeSet::INSERT),
            vec![MapModePrefix::Insert]
        );
        assert_eq!(
            mode_prefixes_from_set(ModeSet::VISUAL),
            vec![MapModePrefix::Visual]
        );
        assert_eq!(
            mode_prefixes_from_set(ModeSet::OPERATOR),
            vec![MapModePrefix::Operator]
        );
        assert_eq!(
            mode_prefixes_from_set(ModeSet::COMMAND),
            vec![MapModePrefix::Command]
        );
    }

    #[test]
    fn mode_prefixes_nvo_collapses_to_all() {
        assert_eq!(
            mode_prefixes_from_set(ModeSet::NVO),
            vec![MapModePrefix::All]
        );
    }

    #[test]
    fn mode_prefixes_nvo_plus_insert() {
        assert_eq!(
            mode_prefixes_from_set(ModeSet::NVO | ModeSet::INSERT),
            vec![MapModePrefix::All, MapModePrefix::Insert]
        );
    }

    #[test]
    fn mode_set_from_prefix_roundtrips() {
        for prefix in [
            MapModePrefix::Normal,
            MapModePrefix::Insert,
            MapModePrefix::Visual,
            MapModePrefix::Operator,
            MapModePrefix::Command,
        ] {
            let set = mode_set_from_prefix(prefix);
            let back = mode_prefixes_from_set(set);
            assert_eq!(back, vec![prefix]);
        }
    }

    #[test]
    fn mode_set_from_prefix_all_gives_nvo() {
        let set = mode_set_from_prefix(MapModePrefix::All);
        assert_eq!(set, ModeSet::NVO);
    }
}
