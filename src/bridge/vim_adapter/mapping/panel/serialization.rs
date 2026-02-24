use super::{MappingPanel, COL_FROM, COL_GMAP, COL_IMAP, COL_NMAP, COL_TO, COL_VMAP};
use crate::bridge::settings::keys;
use godot::builtin::{Array, VarDictionary};
use godot::classes::EditorInterface;
use godot::prelude::*;

impl MappingPanel {
    /// Add a mapping row with checkbox columns.
    pub(super) fn add_mapping_row(
        &mut self,
        parent: &Gd<godot::classes::TreeItem>,
        from: &str,
        to: &str,
        modes: [bool; 4],
    ) {
        let Some(tree) = &mut self.tree else { return };
        let Some(mut item) = tree.create_item_ex().parent(parent).done() else {
            return;
        };

        item.set_text(COL_FROM, from);
        item.set_text(COL_TO, to);
        item.set_editable(COL_FROM, true);
        item.set_editable(COL_TO, true);

        item.set_cell_mode(COL_IMAP, godot::classes::tree_item::TreeCellMode::CHECK);
        item.set_cell_mode(COL_NMAP, godot::classes::tree_item::TreeCellMode::CHECK);
        item.set_cell_mode(COL_VMAP, godot::classes::tree_item::TreeCellMode::CHECK);
        item.set_cell_mode(COL_GMAP, godot::classes::tree_item::TreeCellMode::CHECK);

        item.set_checked(COL_IMAP, modes[0]);
        item.set_checked(COL_NMAP, modes[1]);
        item.set_checked(COL_VMAP, modes[2]);
        item.set_checked(COL_GMAP, modes[3]);

        item.set_editable(COL_IMAP, true);
        item.set_editable(COL_NMAP, true);
        item.set_editable(COL_VMAP, true);
        item.set_editable(COL_GMAP, true);
        item.set_meta("from", &from.to_variant());
        item.set_meta("to", &to.to_variant());
    }

    /// Save tree state to `EditorSettings`.
    /// Uses unified storage that preserves all mappings (including disabled).
    pub(super) fn save_mappings_to_settings(&self) {
        let Some(tree) = &self.tree else { return };
        let Some(root) = tree.get_root() else { return };

        let Some(mut settings) = EditorInterface::singleton().get_editor_settings() else {
            log::error!("EditorSettings not available for saving mappings");
            return;
        };

        // Collect all mappings into a unified array (preserves disabled entries).
        let mut all_mappings: Array<Variant> = Array::new();

        // Iterate through all mapping rows
        let mut child = root.get_first_child();
        while let Some(item) = child {
            let from = item.get_text(COL_FROM);
            let to = item.get_text(COL_TO);

            if !from.is_empty() && !to.is_empty() {
                let imap_enabled = item.is_checked(COL_IMAP);
                let nmap_enabled = item.is_checked(COL_NMAP);
                let vmap_enabled = item.is_checked(COL_VMAP);
                let gmap_enabled = item.is_checked(COL_GMAP);

                // Build modes string: "invg" where each char is present if mode is enabled
                let mut modes = String::new();
                if imap_enabled {
                    modes.push('i');
                }
                if nmap_enabled {
                    modes.push('n');
                }
                if vmap_enabled {
                    modes.push('v');
                }
                if gmap_enabled {
                    modes.push('g');
                }

                // Add to unified array
                let mut entry = VarDictionary::new();
                entry.set("from", from);
                entry.set("to", to);
                entry.set("modes", GString::from(&modes));
                all_mappings.push(&entry.to_variant());
            }
            child = item.get_next();
        }

        // Save unified array (preserves all, including disabled)
        settings.set_setting(keys::ALL_MAPPINGS, &all_mappings.to_variant());

        // Save timeout
        if let Some(timeout) = &self.timeout_spinbox {
            #[expect(
                clippy::cast_possible_truncation,
                reason = "SpinBox value is constrained to valid i64 range"
            )]
            let value = timeout.get_value().round() as i64;
            settings.set_setting(keys::MAPPING_TIMEOUTLEN, &Variant::from(value));
        }

        log::info!("Mappings saved to EditorSettings");
    }
}
