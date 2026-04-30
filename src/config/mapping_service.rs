//! High-level CRUD service for config-file mappings.
//!
//! Owns a [`ConfigDocument`] and exposes query/mutation methods that the
//! Mapping Dialog calls instead of manipulating config internals directly.
//! This decouples the UI layer from the config-file representation so that
//! the dialog imports only this module's public types.

use std::collections::HashMap;

use vim_core::grammar::MapModePrefix;
use vim_core::keymap::MappingKind;

use super::parser;
use super::presets::{PresetDefinition, PRESETS};
use super::types::{
    mode_prefix_display, mode_prefixes_from_set, mode_set_from_prefix, ConfigDocument, ConfigLine,
    MappingPayload, ModeSet, ParsedMapping,
};
use super::writer;

// ─── View types ──────────────────────────────────────────────────────────

/// Indices into `ConfigDocument.lines` that form one visual row in the
/// user-mappings tree. Multiple config lines share a row when they have
/// the same LHS + RHS + Kind but different mode prefixes.
///
/// `pub(crate)` inner so the dialog can bridge to Godot's PackedInt64Array.
#[derive(Debug, Clone)]
pub(crate) struct MappingGroupId(pub(crate) Vec<usize>);

/// Index into `ConfigDocument.lines` for a single preset mapping.
///
/// `pub(crate)` inner so the dialog can store/read it as tree-item metadata.
#[derive(Debug, Clone)]
pub(crate) struct PresetId(pub(crate) usize);

/// A visual row in the user-mappings tree, merging config lines that share
/// the same LHS + RHS + Kind into combined mode checkboxes.
#[derive(Debug, Clone)]
pub(crate) struct UserMappingRow {
    pub id: MappingGroupId,
    pub lhs: String,
    pub rhs: String,
    pub kind: MappingKind,
    pub modes: ModeSet,
}

/// A visual row in the presets tree.
#[derive(Debug, Clone)]
pub(crate) struct PresetRow {
    pub id: PresetId,
    pub lhs: String,
    pub rhs: String,
    pub modes_display: String,
    pub category: &'static str,
    pub enabled: bool,
}

// ─── Service ─────────────────────────────────────────────────────────────

/// High-level CRUD facade over a [`ConfigDocument`].
///
/// The Mapping Dialog holds `Option<MappingService>` and calls service
/// methods for every query and mutation. This keeps grouping logic, mode
/// reconciliation, and index management out of the UI layer.
pub(crate) struct MappingService {
    doc: ConfigDocument,
}

impl MappingService {
    // ── Constructors ─────────────────────────────────────────────────

    pub fn from_text(text: &str) -> Self {
        Self {
            doc: parser::parse_config(text),
        }
    }

    pub fn default_config() -> Self {
        let text = writer::generate_default_config(PRESETS);
        Self::from_text(&text)
    }

    pub fn to_text(&self) -> String {
        writer::serialize(&self.doc)
    }

    // ── Queries ──────────────────────────────────────────────────────

    /// Return grouped user mappings, optionally filtered by a case-insensitive
    /// search on LHS/RHS. Pass empty string for all.
    ///
    /// Lines sharing the same LHS + RHS + Kind merge into one row with
    /// combined mode booleans, preserving insertion order.
    pub fn user_mappings(&self, search: &str) -> Vec<UserMappingRow> {
        let search_lower = search.to_lowercase();
        let mut groups: Vec<UserMappingRow> = Vec::new();
        let mut index: HashMap<(String, String, MappingKind), usize> = HashMap::new();

        for (doc_idx, line) in self.doc.lines.iter().enumerate() {
            let ConfigLine::Mapping(payload) = line else {
                continue;
            };
            if payload.preset_id.is_some() {
                continue;
            }
            let MappingPayload {
                enabled, parsed, ..
            } = payload.as_ref();

            // Mode filtering is the caller's job (via `row_passes_mode_filter`).
            if !search_lower.is_empty()
                && !parsed.lhs.to_lowercase().contains(&search_lower)
                && !parsed.rhs.to_lowercase().contains(&search_lower)
            {
                continue;
            }

            let key = (parsed.lhs.clone(), parsed.rhs.clone(), parsed.kind);
            if let Some(&group_idx) = index.get(&key) {
                let group = &mut groups[group_idx];
                if *enabled {
                    group.modes |= mode_set_from_prefix(parsed.modes);
                }
                group.id.0.push(doc_idx);
            } else {
                let modes = if *enabled {
                    mode_set_from_prefix(parsed.modes)
                } else {
                    ModeSet::empty()
                };
                let group_idx = groups.len();
                index.insert(key, group_idx);
                groups.push(UserMappingRow {
                    id: MappingGroupId(vec![doc_idx]),
                    lhs: parsed.lhs.clone(),
                    rhs: parsed.rhs.clone(),
                    kind: parsed.kind,
                    modes,
                });
            }
        }

        groups
    }

    pub fn preset_mappings(&self) -> Vec<PresetRow> {
        let mut rows = Vec::new();

        for (doc_idx, line) in self.doc.lines.iter().enumerate() {
            let ConfigLine::Mapping(payload) = line else {
                continue;
            };
            if payload.preset_id.is_none() {
                continue;
            }
            let MappingPayload {
                enabled, parsed, ..
            } = payload.as_ref();

            let modes_display = mode_prefix_display(parsed.modes).to_string();

            let category = find_preset_category(parsed);

            rows.push(PresetRow {
                id: PresetId(doc_idx),
                lhs: parsed.lhs.clone(),
                rhs: parsed.rhs.clone(),
                modes_display,
                category,
                enabled: *enabled,
            });
        }

        rows
    }

    pub fn timeoutlen(&self) -> Option<u32> {
        self.doc.timeoutlen()
    }

    // ── Mutations ────────────────────────────────────────────────────

    /// Add a new user mapping, defaulting to Normal mode. Call [`update_modes`]
    /// afterward to adjust mode checkboxes.
    pub fn add_mapping(&mut self, lhs: &str, rhs: &str, kind: MappingKind) {
        let parsed = ParsedMapping {
            lhs: lhs.to_string(),
            rhs: rhs.to_string(),
            modes: MapModePrefix::Normal,
            kind,
        };
        self.doc.add_user_mapping(parsed);
    }

    pub fn remove_mapping(&mut self, id: &MappingGroupId) {
        // Remove in reverse index order so earlier removals don't shift later indices.
        let mut sorted = id.0.clone();
        sorted.sort_unstable_by(|a, b| b.cmp(a));
        for idx in sorted {
            if idx < self.doc.lines.len() {
                self.doc.lines.remove(idx);
            }
        }
    }

    pub fn edit_lhs(&mut self, id: &MappingGroupId, new_lhs: &str) {
        for &idx in &id.0 {
            if let Some(ConfigLine::Mapping(payload)) = self.doc.lines.get_mut(idx) {
                payload.parsed.lhs = new_lhs.to_string();
            }
        }
    }

    pub fn edit_rhs(&mut self, id: &MappingGroupId, new_rhs: &str) {
        for &idx in &id.0 {
            if let Some(ConfigLine::Mapping(payload)) = self.doc.lines.get_mut(idx) {
                payload.parsed.rhs = new_rhs.to_string();
            }
        }
    }

    /// Reconcile mode checkboxes: replaces the group's config lines with one
    /// line per required Vim mode prefix. All-unchecked disables rather than
    /// deletes, preserving the mapping for re-enabling later.
    pub fn update_modes(&mut self, id: &MappingGroupId, modes: ModeSet) {
        let prefixes = mode_prefixes_from_set(modes);

        if prefixes.is_empty() {
            for &idx in &id.0 {
                if let Some(ConfigLine::Mapping(payload)) = self.doc.lines.get_mut(idx) {
                    payload.enabled = false;
                }
            }
            return;
        }

        let first_idx = id.0[0];
        let (lhs, rhs, kind) = match &self.doc.lines[first_idx] {
            ConfigLine::Mapping(payload) => (
                payload.parsed.lhs.clone(),
                payload.parsed.rhs.clone(),
                payload.parsed.kind,
            ),
            _ => return,
        };

        // Reverse order so earlier removals don't shift later indices.
        let mut sorted = id.0.clone();
        sorted.sort_unstable_by(|a, b| b.cmp(a));
        for &idx in &sorted {
            if idx < self.doc.lines.len() {
                self.doc.lines.remove(idx);
            }
        }

        // Clamp to len() in case removals shortened the vec past the original position.
        let insert_at = first_idx.min(self.doc.lines.len());
        for (i, &mode) in prefixes.iter().enumerate() {
            self.doc.lines.insert(
                insert_at + i,
                ConfigLine::Mapping(Box::new(MappingPayload {
                    preset_id: None,
                    enabled: true,
                    parsed: ParsedMapping {
                        lhs: lhs.clone(),
                        rhs: rhs.clone(),
                        modes: mode,
                        kind,
                    },
                })),
            );
        }
    }

    pub fn toggle_preset(&mut self, id: &PresetId, enabled: bool) {
        if id.0 < self.doc.lines.len() {
            if let ConfigLine::Mapping(payload) = &mut self.doc.lines[id.0] {
                payload.enabled = enabled;
            }
        }
    }

    pub fn set_timeoutlen(&mut self, ms: u32) {
        self.doc.set_timeoutlen(ms);
    }
}

// ─── Helpers ─────────────────────────────────────────────────────────────

/// Falls back to "Custom" if no static preset matches (e.g., user manually
/// edited the config file to add a preset-tagged mapping we don't ship).
fn find_preset_category(parsed: &ParsedMapping) -> &'static str {
    PRESETS
        .iter()
        .find(|p| p.lhs == parsed.lhs && p.rhs == parsed.rhs)
        .map_or("Custom", |p: &PresetDefinition| p.category)
}

/// Check whether a [`UserMappingRow`] passes a mode-filter dropdown index.
///
/// Filter indices: 0 = All Modes, 1 = Normal, 2 = Insert, 3 = Visual,
/// 4 = Operator, 5 = Command.
pub(crate) fn row_passes_mode_filter(row: &UserMappingRow, filter_idx: i32) -> bool {
    match filter_idx {
        0 => true,
        1 => row.modes.contains(ModeSet::NORMAL),
        2 => row.modes.contains(ModeSet::INSERT),
        3 => row.modes.contains(ModeSet::VISUAL),
        4 => row.modes.contains(ModeSet::OPERATOR),
        5 => row.modes.contains(ModeSet::COMMAND),
        _ => true,
    }
}

// ─── Tests ───────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_config() -> &'static str {
        "\
\" GodotVim Configuration
let mapleader = \" \"
set timeoutlen=500

\" --- User Mappings ---
nnoremap jk <Esc>
inoremap jk <Esc>
nnoremap <Leader>w :save<CR>

\" --- Presets ---
\" preset:enabled
nnoremap <Space>w :save<CR>
\" preset:disabled
\" inoremap jj <Esc>
"
    }

    #[test]
    fn from_text_parses_correctly() {
        let svc = MappingService::from_text(sample_config());
        let users = svc.user_mappings("");
        // jk <Esc> noremap groups into one row (nnoremap + inoremap).
        // <Leader>w :save<CR> is a second row.
        assert_eq!(users.len(), 2);
    }

    #[test]
    fn user_mappings_groups_by_lhs_rhs_kind() {
        let svc = MappingService::from_text(sample_config());
        let users = svc.user_mappings("");
        let jk_row = &users[0];
        assert_eq!(jk_row.lhs, "jk");
        assert_eq!(jk_row.rhs, "<Esc>");
        assert!(jk_row.modes.contains(ModeSet::NORMAL));
        assert!(jk_row.modes.contains(ModeSet::INSERT));
        assert!(!jk_row.modes.contains(ModeSet::VISUAL));
        assert!(!jk_row.modes.contains(ModeSet::OPERATOR));
        assert!(!jk_row.modes.contains(ModeSet::COMMAND));
        // Two doc lines contribute to this group.
        assert_eq!(jk_row.id.0.len(), 2);
    }

    #[test]
    fn user_mappings_search_filter() {
        let svc = MappingService::from_text(sample_config());
        let filtered = svc.user_mappings("leader");
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].lhs, "<Leader>w");
    }

    #[test]
    fn preset_mappings_returns_presets() {
        let svc = MappingService::from_text(sample_config());
        let presets = svc.preset_mappings();
        assert_eq!(presets.len(), 2);
        assert!(presets[0].enabled);
        assert!(!presets[1].enabled);
    }

    #[test]
    fn add_mapping_appends() {
        let mut svc = MappingService::from_text(sample_config());
        let before = svc.user_mappings("").len();
        svc.add_mapping("gj", "j", MappingKind::Recursive);
        let after = svc.user_mappings("").len();
        assert_eq!(after, before + 1);

        let last = svc.user_mappings("").pop().unwrap();
        assert_eq!(last.lhs, "gj");
        assert_eq!(last.rhs, "j");
        assert_eq!(last.kind, MappingKind::Recursive);
    }

    #[test]
    fn remove_mapping_removes_all_group_lines() {
        let mut svc = MappingService::from_text(sample_config());
        let users = svc.user_mappings("");
        let jk_id = users[0].id.clone();
        assert_eq!(jk_id.0.len(), 2); // Two doc lines for jk.

        svc.remove_mapping(&jk_id);
        let after = svc.user_mappings("");
        assert_eq!(after.len(), 1);
        assert_eq!(after[0].lhs, "<Leader>w");
    }

    #[test]
    fn edit_lhs_updates_all_group_lines() {
        let mut svc = MappingService::from_text(sample_config());
        let users = svc.user_mappings("");
        let jk_id = users[0].id.clone();
        svc.edit_lhs(&jk_id, "kj");

        let after = svc.user_mappings("");
        assert_eq!(after[0].lhs, "kj");
    }

    #[test]
    fn edit_rhs_updates_all_group_lines() {
        let mut svc = MappingService::from_text(sample_config());
        let users = svc.user_mappings("");
        let jk_id = users[0].id.clone();
        svc.edit_rhs(&jk_id, "<C-c>");

        let after = svc.user_mappings("");
        assert_eq!(after[0].rhs, "<C-c>");
    }

    #[test]
    fn update_modes_all_unchecked_disables() {
        let mut svc = MappingService::from_text(sample_config());
        let users = svc.user_mappings("");
        let jk_id = users[0].id.clone();

        svc.update_modes(&jk_id, ModeSet::empty());

        // All modes unchecked: row still exists but modes is empty.
        let after = svc.user_mappings("");
        let jk = &after[0];
        assert!(jk.modes.is_empty());
    }

    #[test]
    fn update_modes_changes_prefixes() {
        let mut svc = MappingService::from_text(sample_config());
        let users = svc.user_mappings("");
        let jk_id = users[0].id.clone();

        // Change from N+I to N+V+O (which collapses to All).
        svc.update_modes(&jk_id, ModeSet::NVO);

        let after = svc.user_mappings("");
        let jk = &after[0];
        assert!(jk.modes.contains(ModeSet::NORMAL));
        assert!(!jk.modes.contains(ModeSet::INSERT));
        assert!(jk.modes.contains(ModeSet::VISUAL));
        assert!(jk.modes.contains(ModeSet::OPERATOR));
        assert!(!jk.modes.contains(ModeSet::COMMAND));
    }

    #[test]
    fn toggle_preset() {
        let mut svc = MappingService::from_text(sample_config());
        let presets = svc.preset_mappings();
        assert!(presets[0].enabled);

        svc.toggle_preset(&presets[0].id, false);

        let after = svc.preset_mappings();
        assert!(!after[0].enabled);
    }

    #[test]
    fn timeoutlen_read_and_write() {
        let mut svc = MappingService::from_text(sample_config());
        assert_eq!(svc.timeoutlen(), Some(500));

        svc.set_timeoutlen(2000);
        assert_eq!(svc.timeoutlen(), Some(2000));
    }

    #[test]
    fn default_config_has_presets() {
        let svc = MappingService::default_config();
        let presets = svc.preset_mappings();
        assert!(!presets.is_empty());
    }

    #[test]
    fn to_text_roundtrips() {
        let svc = MappingService::from_text(sample_config());
        let text = svc.to_text();
        let svc2 = MappingService::from_text(&text);
        let users1 = svc.user_mappings("");
        let users2 = svc2.user_mappings("");
        assert_eq!(users1.len(), users2.len());
        for (a, b) in users1.iter().zip(users2.iter()) {
            assert_eq!(a.lhs, b.lhs);
            assert_eq!(a.rhs, b.rhs);
            assert_eq!(a.kind, b.kind);
            assert_eq!(a.modes, b.modes);
        }
    }

    #[test]
    fn row_passes_mode_filter_all() {
        let row = UserMappingRow {
            id: MappingGroupId(vec![0]),
            lhs: "x".into(),
            rhs: "y".into(),
            kind: MappingKind::NonRecursive,
            modes: ModeSet::NORMAL,
        };
        assert!(row_passes_mode_filter(&row, 0)); // All modes
        assert!(row_passes_mode_filter(&row, 1)); // Normal
        assert!(!row_passes_mode_filter(&row, 2)); // Insert
    }
}
