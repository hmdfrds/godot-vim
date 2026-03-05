use super::{MappingPanel, COL_FROM, COL_GMAP, COL_IMAP, COL_NMAP, COL_TO, COL_VMAP};
use crate::bridge::godot::names::{button, callbacks, range, tree};
use crate::bridge::settings::{keys, VimSettings};
use godot::builtin::{Array, VarDictionary};
use godot::classes::{
    Button, Control, EditorInterface, HBoxContainer, Label, LineEdit, SpinBox, Tree, VBoxContainer,
};
use godot::prelude::*;

impl MappingPanel {
    pub(super) fn build_ui(&mut self) {
        let mut main_vbox = VBoxContainer::new_alloc();
        main_vbox.set_anchors_preset(godot::classes::control::LayoutPreset::FULL_RECT);

        self.build_add_row(&mut main_vbox);
        self.build_mapping_tree(&mut main_vbox);
        self.build_bottom_row(&mut main_vbox);

        self.base_mut().add_child(&main_vbox);
    }

    fn build_add_row(&mut self, main_vbox: &mut Gd<VBoxContainer>) {
        let mut add_hbox = HBoxContainer::new_alloc();

        let mut from_label = Label::new_alloc();
        from_label.set_text("From:");
        add_hbox.add_child(&from_label);

        let mut from_input = LineEdit::new_alloc();
        from_input.set_placeholder("jj");
        from_input.set_custom_minimum_size(Vector2::new(60.0, 0.0));
        add_hbox.add_child(&from_input);
        self.from_input = Some(from_input);

        let mut to_label = Label::new_alloc();
        to_label.set_text("To:");
        add_hbox.add_child(&to_label);

        let mut to_input = LineEdit::new_alloc();
        to_input.set_placeholder("<Esc>");
        to_input.set_custom_minimum_size(Vector2::new(80.0, 0.0));
        add_hbox.add_child(&to_input);
        self.to_input = Some(to_input);

        let mut add_button = Button::new_alloc();
        add_button.set_text("Add");
        add_button.connect(
            button::signals::PRESSED,
            &self.base().callable(callbacks::ON_ADD_PRESSED),
        );
        add_hbox.add_child(&add_button);
        self.add_button = Some(add_button);

        main_vbox.add_child(&add_hbox);
    }

    fn build_mapping_tree(&mut self, main_vbox: &mut Gd<VBoxContainer>) {
        let mut tree = Tree::new_alloc();
        tree.set_v_size_flags(godot::classes::control::SizeFlags::EXPAND_FILL);
        tree.set_columns(6);
        tree.set_column_titles_visible(true);
        tree.set_column_title(COL_FROM, "From");
        tree.set_column_title(COL_TO, "To");
        tree.set_column_title(COL_IMAP, "I");
        tree.set_column_title(COL_NMAP, "N");
        tree.set_column_title(COL_VMAP, "V");
        tree.set_column_title(COL_GMAP, "G");

        // Set checkbox columns to be narrow
        tree.set_column_expand(COL_IMAP, false);
        tree.set_column_expand(COL_NMAP, false);
        tree.set_column_expand(COL_VMAP, false);
        tree.set_column_expand(COL_GMAP, false);
        tree.set_column_custom_minimum_width(COL_IMAP, 30);
        tree.set_column_custom_minimum_width(COL_NMAP, 30);
        tree.set_column_custom_minimum_width(COL_VMAP, 30);
        tree.set_column_custom_minimum_width(COL_GMAP, 30);

        tree.set_hide_root(true);
        tree.set_select_mode(godot::classes::tree::SelectMode::ROW);

        // Auto-save when any cell is edited (including checkbox changes)
        tree.connect(
            tree::signals::ITEM_EDITED,
            &self.base().callable(callbacks::ON_TREE_ITEM_EDITED),
        );

        main_vbox.add_child(&tree);
        self.tree = Some(tree);
    }

    fn build_bottom_row(&mut self, main_vbox: &mut Gd<VBoxContainer>) {
        let mut bottom_hbox = HBoxContainer::new_alloc();

        let mut delete_button = Button::new_alloc();
        delete_button.set_text("Delete Selected");
        delete_button.connect(
            button::signals::PRESSED,
            &self.base().callable(callbacks::ON_DELETE_PRESSED),
        );
        bottom_hbox.add_child(&delete_button);
        self.delete_button = Some(delete_button);

        let mut reload_button = Button::new_alloc();
        reload_button.set_text("Reload Settings");
        reload_button.set_tooltip_text("Apply settings changed in Project Settings");
        reload_button.connect(
            button::signals::PRESSED,
            &self.base().callable(callbacks::ON_RELOAD_SETTINGS_PRESSED),
        );
        bottom_hbox.add_child(&reload_button);

        let mut spacer = Control::new_alloc();
        spacer.set_h_size_flags(godot::classes::control::SizeFlags::EXPAND_FILL);
        bottom_hbox.add_child(&spacer);

        let mut timeout_spinbox = SpinBox::new_alloc();
        timeout_spinbox.set_min(100.0);
        timeout_spinbox.set_max(5000.0);
        timeout_spinbox.set_step(100.0);
        #[expect(
            clippy::cast_precision_loss,
            reason = "milliseconds don't need 64-bit precision for UI"
        )]
        timeout_spinbox.set_value(VimSettings::timeoutlen() as f64);
        timeout_spinbox.set_suffix("ms");
        // Auto-save when timeout changes
        timeout_spinbox.connect(
            range::signals::VALUE_CHANGED,
            &self.base().callable(callbacks::ON_TIMEOUT_CHANGED),
        );
        bottom_hbox.add_child(&timeout_spinbox);
        self.timeout_spinbox = Some(timeout_spinbox);

        // No save button; all changes are saved automatically.

        main_vbox.add_child(&bottom_hbox);
    }

    /// Load mappings from `ProjectSettings` into the tree using unified storage only.
    pub(super) fn load_mappings_from_settings(&mut self) {
        let Some(tree) = &mut self.tree else { return };

        tree.clear();
        let Some(root) = tree.create_item() else {
            return;
        };

        let Some(settings) = EditorInterface::singleton().get_editor_settings() else {
            log::warn!("EditorSettings not available for mapping panel");
            return;
        };
        let all_key: GString = keys::ALL_MAPPINGS.into();

        // Unified mapping storage
        if settings.has_setting(&all_key) {
            let variant = settings.get_setting(&all_key);
            if let Ok(arr) = variant.try_to::<Array<Variant>>() {
                for entry_var in arr.iter_shared() {
                    if let Ok(entry) = entry_var.try_to::<VarDictionary>() {
                        let from = entry
                            .get("from")
                            .and_then(|v| v.try_to::<GString>().ok())
                            .map(|s| s.to_string())
                            .unwrap_or_default();
                        let to = entry
                            .get("to")
                            .and_then(|v| v.try_to::<GString>().ok())
                            .map(|s| s.to_string())
                            .unwrap_or_default();
                        let modes_str = entry
                            .get("modes")
                            .and_then(|v| v.try_to::<GString>().ok())
                            .map(|s| s.to_string())
                            .unwrap_or_default();

                        if !from.is_empty() && !to.is_empty() {
                            let modes = [
                                modes_str.contains('i'),
                                modes_str.contains('n'),
                                modes_str.contains('v'),
                                modes_str.contains('g'),
                            ];
                            self.add_mapping_row(&root, &from, &to, modes);
                        }
                    }
                }
            }
        }
    }
}
