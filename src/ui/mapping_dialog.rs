//! Floating mapping management dialog.
//!
//! Opened via `:mappings` — provides a full CRUD editor for user mappings
//! and a toggleable presets section. Changes auto-save to `.godot-vimrc`
//! and hot-reload into the engine immediately.
//!
//! ## UI Layout
//!
//! ```text
//! Window "GodotVim Mappings" (700×500)
//! ├── MarginContainer
//! │   └── VBoxContainer
//! │       ├── TabContainer
//! │       │   ├── "User Mappings" tab
//! │       │   │   ├── HBox: [OptionButton(mode filter) | LineEdit(search)]
//! │       │   │   ├── Tree (8 cols: LHS, RHS, N✓, I✓, V✓, O✓, C✓, Type)
//! │       │   │   ├── HBox: [LineEdit(from) | LineEdit(to) | OptionButton(type) | Button(Add)]
//! │       │   │   └── HBox: [Button(Delete) | spacer | Button(Reload)]
//! │       │   └── "Presets" tab
//! │       │       ├── Label (description)
//! │       │       └── Tree (5 cols: Enabled✓, LHS, RHS, Modes, Category)
//! │       ├── HSeparator
//! │       └── HBox: [Label(config path) | Label("Timeout:") | SpinBox(ms) | Button(Close)]
//! ```

use godot::builtin::PackedInt64Array;
use godot::classes::tree::SelectMode;
use godot::classes::{
    Button, Control, HBoxContainer, HSeparator, IWindow, Label, LineEdit, MarginContainer,
    OptionButton, SpinBox, TabContainer, Tree, VBoxContainer, Window,
};
use godot::prelude::*;

use crate::config::mapping_service::{
    row_passes_mode_filter, MappingGroupId, MappingService, PresetId, UserMappingRow,
};
use crate::config::types::ModeSet;
use crate::config::writer;
use crate::safety::panic_guard;
use vim_core::keymap::MappingKind;

// ─── Column indices ──────────────────────────────────────────────────────

// User mappings tree columns.
const COL_LHS: i32 = 0;
const COL_RHS: i32 = 1;
const COL_N: i32 = 2;
const COL_I: i32 = 3;
const COL_V: i32 = 4;
const COL_O: i32 = 5;
const COL_C: i32 = 6;
const COL_TYPE: i32 = 7;

// Preset tree columns.
const PCOL_ENABLED: i32 = 0;
const PCOL_LHS: i32 = 1;
const PCOL_RHS: i32 = 2;
const PCOL_MODES: i32 = 3;
const PCOL_CATEGORY: i32 = 4;

// ─── GodotClass ──────────────────────────────────────────────────────────

/// Floating dialog for mapping management.
///
/// Created lazily by the plugin when `:mappings` is first invoked.
/// Subsequent invocations re-show the same instance (preserving tree state).
#[derive(GodotClass)]
#[class(tool, base=Window)]
pub(crate) struct MappingDialog {
    base: Base<Window>,

    // All widgets are `Option` because gdext requires `None` in `init()`
    // (the scene tree isn't available until `ready()`).
    user_tree: Option<Gd<Tree>>,
    preset_tree: Option<Gd<Tree>>,
    from_input: Option<Gd<LineEdit>>,
    to_input: Option<Gd<LineEdit>>,
    type_select: Option<Gd<OptionButton>>,
    mode_filter: Option<Gd<OptionButton>>,
    search_input: Option<Gd<LineEdit>>,
    timeout_spinbox: Option<Gd<SpinBox>>,
    config_path_label: Option<Gd<Label>>,

    /// In-memory representation of `.godot-vimrc`; kept in sync with disk.
    service: Option<MappingService>,
    config_path: String,
}

#[godot_api]
impl IWindow for MappingDialog {
    fn init(base: Base<Window>) -> Self {
        Self {
            base,
            user_tree: None,
            preset_tree: None,
            from_input: None,
            to_input: None,
            type_select: None,
            mode_filter: None,
            search_input: None,
            timeout_spinbox: None,
            config_path_label: None,
            service: None,
            config_path: String::new(),
        }
    }

    fn ready(&mut self) {
        panic_guard(
            "mapping_dialog::ready",
            || {
                self.base_mut()
                    .set_title(&GString::from("GodotVim Mappings"));
                self.base_mut().set_size(Vector2i::new(700, 500));
                self.base_mut().set_min_size(Vector2i::new(500, 350));

                // Hide on close rather than freeing — the dialog is reused across
                // multiple `:mappings` invocations to preserve tree state.
                let callable = self.base().callable("on_close_requested");
                self.base_mut().connect("close_requested", &callable);

                self.build_ui();
            },
            (),
        );
    }
}

// ─── Signal handlers ─────────────────────────────────────────────────────

#[godot_api]
impl MappingDialog {
    /// The plugin listens to this to hot-reload the engine's keymap tables.
    #[signal]
    fn config_saved();

    /// Deferred target for tree edits -- Godot's Tree forbids structural
    /// modifications (clear/rebuild) during an `item_edited` callback.
    #[func]
    fn deferred_save_and_reload(&mut self) {
        panic_guard(
            "mapping_dialog::deferred_save_and_reload",
            || self.save_and_reload(),
            (),
        );
    }

    #[func]
    fn on_close_requested(&mut self) {
        panic_guard(
            "mapping_dialog::on_close_requested",
            || self.base_mut().hide(),
            (),
        );
    }

    #[func]
    fn on_add_pressed(&mut self) {
        panic_guard(
            "mapping_dialog::on_add_pressed",
            || {
                let from = self
                    .from_input
                    .as_ref()
                    .map_or(String::new(), |e| e.get_text().to_string());
                let to = self
                    .to_input
                    .as_ref()
                    .map_or(String::new(), |e| e.get_text().to_string());

                if from.trim().is_empty() || to.trim().is_empty() {
                    return;
                }

                let kind = if self
                    .type_select
                    .as_ref()
                    .is_none_or(|s| s.get_selected() == 0)
                {
                    MappingKind::NonRecursive
                } else {
                    MappingKind::Recursive
                };

                if let Some(svc) = &mut self.service {
                    svc.add_mapping(&from, &to, kind);
                    self.save_and_reload();
                }

                if let Some(input) = &mut self.from_input {
                    input.set_text(&GString::new());
                }
                if let Some(input) = &mut self.to_input {
                    input.set_text(&GString::new());
                }
            },
            (),
        );
    }

    #[func]
    fn on_delete_pressed(&mut self) {
        panic_guard(
            "mapping_dialog::on_delete_pressed",
            || {
                let Some(tree) = &self.user_tree else { return };
                let Some(selected) = tree.get_selected() else {
                    return;
                };

                let doc_indices = Self::read_doc_indices(&selected);
                if doc_indices.is_empty() {
                    return;
                }

                if let Some(svc) = &mut self.service {
                    let id = MappingGroupId(doc_indices);
                    svc.remove_mapping(&id);
                    self.save_and_reload();
                }
            },
            (),
        );
    }

    #[func]
    fn on_reload_pressed(&mut self) {
        panic_guard(
            "mapping_dialog::on_reload_pressed",
            || self.reload_from_file(),
            (),
        );
    }

    #[func]
    fn on_user_tree_item_edited(&mut self) {
        panic_guard(
            "mapping_dialog::on_user_tree_item_edited",
            || {
                let Some(tree) = &self.user_tree else { return };
                let Some(item) = tree.get_edited() else {
                    return;
                };
                let col = tree.get_edited_column();

                let doc_indices = Self::read_doc_indices(&item);
                if doc_indices.is_empty() {
                    return;
                }

                let Some(svc) = &mut self.service else { return };
                let id = MappingGroupId(doc_indices);

                match col {
                    COL_LHS => {
                        let new_text = item.get_text(col).to_string();
                        svc.edit_lhs(&id, &new_text);
                    }
                    COL_RHS => {
                        let new_text = item.get_text(col).to_string();
                        svc.edit_rhs(&id, &new_text);
                    }
                    COL_N | COL_I | COL_V | COL_O | COL_C => {
                        let mut modes = ModeSet::empty();
                        if item.is_checked(COL_N) {
                            modes |= ModeSet::NORMAL;
                        }
                        if item.is_checked(COL_I) {
                            modes |= ModeSet::INSERT;
                        }
                        if item.is_checked(COL_V) {
                            modes |= ModeSet::VISUAL;
                        }
                        if item.is_checked(COL_O) {
                            modes |= ModeSet::OPERATOR;
                        }
                        if item.is_checked(COL_C) {
                            modes |= ModeSet::COMMAND;
                        }
                        svc.update_modes(&id, modes);
                    }
                    _ => return,
                }

                self.schedule_save_and_reload();
            },
            (),
        );
    }

    #[func]
    fn on_preset_tree_item_edited(&mut self) {
        panic_guard(
            "mapping_dialog::on_preset_tree_item_edited",
            || {
                let Some(tree) = &self.preset_tree else {
                    return;
                };
                let Some(item) = tree.get_edited() else {
                    return;
                };
                let col = tree.get_edited_column();

                if col != PCOL_ENABLED {
                    return;
                }

                let metadata = item.get_metadata(0);
                let Ok(doc_idx) = metadata.try_to::<i64>() else {
                    return;
                };
                if doc_idx < 0 {
                    return;
                }
                let doc_idx = doc_idx.max(0) as usize;

                if let Some(svc) = &mut self.service {
                    let id = PresetId(doc_idx);
                    svc.toggle_preset(&id, item.is_checked(PCOL_ENABLED));
                }

                self.schedule_save_and_reload();
            },
            (),
        );
    }

    #[func]
    fn on_close_button_pressed(&mut self) {
        panic_guard(
            "mapping_dialog::on_close_button_pressed",
            || self.base_mut().hide(),
            (),
        );
    }

    #[func]
    fn on_mode_filter_changed(&mut self, _index: i32) {
        panic_guard(
            "mapping_dialog::on_mode_filter_changed",
            || self.refresh_user_tree(),
            (),
        );
    }

    #[func]
    fn on_search_changed(&mut self, _text: GString) {
        panic_guard(
            "mapping_dialog::on_search_changed",
            || self.refresh_user_tree(),
            (),
        );
    }

    #[func]
    fn on_timeout_changed(&mut self, value: f64) {
        panic_guard(
            "mapping_dialog::on_timeout_changed",
            || {
                let ms = value.round().max(0.0).min(u32::MAX as f64) as u32;
                if let Some(svc) = &mut self.service {
                    svc.set_timeoutlen(ms);
                    self.save_and_reload();
                }
            },
            (),
        );
    }
}

// ─── UI construction ─────────────────────────────────────────────────────

impl MappingDialog {
    /// Build the full dialog UI tree programmatically (no .tscn file).
    /// See the module-level doc comment for the widget hierarchy.
    fn build_ui(&mut self) {
        let mut margin = MarginContainer::new_alloc();
        margin.set_anchors_preset(godot::classes::control::LayoutPreset::FULL_RECT);
        margin.add_theme_constant_override("margin_left", 8);
        margin.add_theme_constant_override("margin_right", 8);
        margin.add_theme_constant_override("margin_top", 8);
        margin.add_theme_constant_override("margin_bottom", 8);

        let mut main_vbox = VBoxContainer::new_alloc();

        let mut tabs = TabContainer::new_alloc();
        tabs.set_v_size_flags(godot::classes::control::SizeFlags::EXPAND_FILL);

        let user_tab = self.build_user_tab();
        tabs.add_child(&user_tab);

        let preset_tab = self.build_preset_tab();
        tabs.add_child(&preset_tab);

        main_vbox.add_child(&tabs);

        let separator = HSeparator::new_alloc();
        main_vbox.add_child(&separator);

        let bottom_bar = self.build_bottom_bar();
        main_vbox.add_child(&bottom_bar);

        margin.add_child(&main_vbox);
        self.base_mut().add_child(&margin);
    }

    fn build_user_tab(&mut self) -> Gd<Control> {
        let mut vbox = VBoxContainer::new_alloc();
        vbox.set_name(&StringName::from("User Mappings"));

        let mut filter_hbox = HBoxContainer::new_alloc();

        let mut mode_filter = OptionButton::new_alloc();
        mode_filter.add_item("All Modes");
        mode_filter.add_item("Normal");
        mode_filter.add_item("Insert");
        mode_filter.add_item("Visual");
        mode_filter.add_item("Operator");
        mode_filter.add_item("Command");
        let callable = self.base().callable("on_mode_filter_changed");
        mode_filter.connect("item_selected", &callable);
        filter_hbox.add_child(&mode_filter);
        self.mode_filter = Some(mode_filter);

        let mut search = LineEdit::new_alloc();
        search.set_placeholder(&GString::from("Search..."));
        search.set_h_size_flags(godot::classes::control::SizeFlags::EXPAND_FILL);
        let callable = self.base().callable("on_search_changed");
        search.connect("text_changed", &callable);
        filter_hbox.add_child(&search);
        self.search_input = Some(search);

        vbox.add_child(&filter_hbox);

        let mut tree = Tree::new_alloc();
        tree.set_columns(8);
        tree.set_column_titles_visible(true);
        tree.set_column_title(COL_LHS, &GString::from("LHS"));
        tree.set_column_title(COL_RHS, &GString::from("RHS"));
        tree.set_column_title(COL_N, &GString::from("N"));
        tree.set_column_title(COL_I, &GString::from("I"));
        tree.set_column_title(COL_V, &GString::from("V"));
        tree.set_column_title(COL_O, &GString::from("O"));
        tree.set_column_title(COL_C, &GString::from("C"));
        tree.set_column_title(COL_TYPE, &GString::from("Type"));

        // Mode checkbox columns are narrow (35px); just enough for a checkmark.
        for col in [COL_N, COL_I, COL_V, COL_O, COL_C] {
            tree.set_column_expand(col, false);
            tree.set_column_custom_minimum_width(col, 35);
        }
        tree.set_column_expand(COL_TYPE, false);
        tree.set_column_custom_minimum_width(COL_TYPE, 70);

        tree.set_hide_root(true);
        tree.set_select_mode(SelectMode::ROW);
        tree.set_v_size_flags(godot::classes::control::SizeFlags::EXPAND_FILL);

        let callable = self.base().callable("on_user_tree_item_edited");
        tree.connect("item_edited", &callable);

        vbox.add_child(&tree);
        self.user_tree = Some(tree);

        let mut add_hbox = HBoxContainer::new_alloc();

        let mut from_input = LineEdit::new_alloc();
        from_input.set_placeholder(&GString::from("LHS (e.g. jk)"));
        from_input.set_h_size_flags(godot::classes::control::SizeFlags::EXPAND_FILL);
        add_hbox.add_child(&from_input);
        self.from_input = Some(from_input);

        let mut to_input = LineEdit::new_alloc();
        to_input.set_placeholder(&GString::from("RHS (e.g. <Esc>)"));
        to_input.set_h_size_flags(godot::classes::control::SizeFlags::EXPAND_FILL);
        add_hbox.add_child(&to_input);
        self.to_input = Some(to_input);

        let mut type_select = OptionButton::new_alloc();
        type_select.add_item(&GString::from("noremap"));
        type_select.add_item(&GString::from("map"));
        add_hbox.add_child(&type_select);
        self.type_select = Some(type_select);

        let mut add_button = Button::new_alloc();
        add_button.set_text(&GString::from("Add"));
        let callable = self.base().callable("on_add_pressed");
        add_button.connect("pressed", &callable);
        add_hbox.add_child(&add_button);

        vbox.add_child(&add_hbox);

        let mut action_hbox = HBoxContainer::new_alloc();

        let mut delete_button = Button::new_alloc();
        delete_button.set_text(&GString::from("Delete Selected"));
        let callable = self.base().callable("on_delete_pressed");
        delete_button.connect("pressed", &callable);
        action_hbox.add_child(&delete_button);

        let mut spacer = Control::new_alloc();
        spacer.set_h_size_flags(godot::classes::control::SizeFlags::EXPAND_FILL);
        action_hbox.add_child(&spacer);

        let mut reload_button = Button::new_alloc();
        reload_button.set_text(&GString::from("Reload from File"));
        let callable = self.base().callable("on_reload_pressed");
        reload_button.connect("pressed", &callable);
        action_hbox.add_child(&reload_button);

        vbox.add_child(&action_hbox);

        vbox.upcast::<Control>()
    }

    fn build_preset_tab(&mut self) -> Gd<Control> {
        let mut vbox = VBoxContainer::new_alloc();
        vbox.set_name(&StringName::from("Presets"));

        let mut desc = Label::new_alloc();
        desc.set_text(&GString::from(
            "Toggle recommended mappings. Changes are saved to your config file.",
        ));
        desc.set_autowrap_mode(godot::classes::text_server::AutowrapMode::WORD_SMART);
        vbox.add_child(&desc);

        let mut tree = Tree::new_alloc();
        tree.set_columns(5);
        tree.set_column_titles_visible(true);
        tree.set_column_title(PCOL_ENABLED, &GString::from("On"));
        tree.set_column_title(PCOL_LHS, &GString::from("LHS"));
        tree.set_column_title(PCOL_RHS, &GString::from("RHS"));
        tree.set_column_title(PCOL_MODES, &GString::from("Modes"));
        tree.set_column_title(PCOL_CATEGORY, &GString::from("Category"));

        tree.set_column_expand(PCOL_ENABLED, false);
        tree.set_column_custom_minimum_width(PCOL_ENABLED, 35);
        tree.set_column_expand(PCOL_MODES, false);
        tree.set_column_custom_minimum_width(PCOL_MODES, 50);

        tree.set_hide_root(true);
        tree.set_select_mode(SelectMode::ROW);
        tree.set_v_size_flags(godot::classes::control::SizeFlags::EXPAND_FILL);

        let callable = self.base().callable("on_preset_tree_item_edited");
        tree.connect("item_edited", &callable);

        vbox.add_child(&tree);
        self.preset_tree = Some(tree);

        vbox.upcast::<Control>()
    }

    fn build_bottom_bar(&mut self) -> Gd<HBoxContainer> {
        let mut hbox = HBoxContainer::new_alloc();

        let mut path_label = Label::new_alloc();
        path_label.set_text(&GString::from("Config: (none)"));
        path_label.set_h_size_flags(godot::classes::control::SizeFlags::EXPAND_FILL);
        hbox.add_child(&path_label);
        self.config_path_label = Some(path_label);

        // Timeout controls mapping ambiguity resolution: how long (ms) to wait
        // for a longer match before accepting the current prefix.
        let mut timeout_label = Label::new_alloc();
        timeout_label.set_text(&GString::from("Timeout:"));
        hbox.add_child(&timeout_label);

        let mut timeout_spinbox = SpinBox::new_alloc();
        use crate::settings::defaults;
        timeout_spinbox.set_min(defaults::TIMEOUTLEN_MIN as f64);
        timeout_spinbox.set_max(defaults::TIMEOUTLEN_MAX as f64);
        timeout_spinbox.set_step(100.0);
        timeout_spinbox.set_value(defaults::TIMEOUTLEN as f64);
        timeout_spinbox.set_suffix(&GString::from("ms"));
        let callable = self.base().callable("on_timeout_changed");
        timeout_spinbox.connect("value_changed", &callable);
        hbox.add_child(&timeout_spinbox);
        self.timeout_spinbox = Some(timeout_spinbox);

        let mut close_button = Button::new_alloc();
        close_button.set_text(&GString::from("Close"));
        let callable = self.base().callable("on_close_button_pressed");
        close_button.connect("pressed", &callable);
        hbox.add_child(&close_button);

        hbox
    }
}

// ─── Data operations ─────────────────────────────────────────────────────

impl MappingDialog {
    /// Load (or create default) the config and show the dialog.
    ///
    /// If the config file doesn't exist, a default `.godot-vimrc` is written
    /// to disk so the user has a starting point to edit.
    pub(crate) fn open_with_config(&mut self, config_path: &str) {
        self.config_path = config_path.to_string();

        let svc = match writer::read_file(config_path) {
            Some(text) => MappingService::from_text(&text),
            None => {
                let svc = MappingService::default_config();
                let text = svc.to_text();
                if let Err(e) = writer::write_text_to_file(config_path, &text) {
                    log::warn!("Failed to create default config: {}", e);
                }
                svc
            }
        };

        self.update_timeout_spinbox(&svc);
        self.service = Some(svc);
        self.refresh_user_tree();
        self.refresh_preset_tree();
        self.update_config_path_label();

        self.base_mut().popup_centered();
    }

    /// Extract document-level indices from a tree item's metadata.
    ///
    /// User mapping rows store a `PackedInt64Array` (one group can span
    /// multiple config lines). Preset rows store a single `i64`. Both
    /// are stashed in column-0 metadata during tree population.
    fn read_doc_indices(item: &Gd<godot::classes::TreeItem>) -> Vec<usize> {
        let metadata = item.get_metadata(0);
        if let Ok(arr) = metadata.try_to::<PackedInt64Array>() {
            (0..arr.len())
                .map(|i| usize::try_from(arr[i].max(0)).unwrap_or(0))
                .collect()
        } else if let Ok(single) = metadata.try_to::<i64>() {
            if single >= 0 {
                vec![single as usize]
            } else {
                Vec::new()
            }
        } else {
            Vec::new()
        }
    }

    /// Re-read `.godot-vimrc` from disk (picks up external edits).
    fn reload_from_file(&mut self) {
        if let Some(text) = writer::read_file(&self.config_path) {
            let svc = MappingService::from_text(&text);
            self.update_timeout_spinbox(&svc);
            self.service = Some(svc);
            self.refresh_user_tree();
            self.refresh_preset_tree();
            log::debug!("MappingDialog: reloaded from '{}'", self.config_path);
        }
    }

    /// Deferred save -- required from `item_edited` callbacks because Godot's
    /// Tree forbids structural modifications during signal dispatch.
    fn schedule_save_and_reload(&mut self) {
        self.base_mut()
            .call_deferred("deferred_save_and_reload", &[]);
    }

    /// Persist to disk, refresh both trees, and emit `config_saved` so the
    /// plugin hot-reloads the engine's keymap tables.
    fn save_and_reload(&mut self) {
        let Some(svc) = &self.service else { return };

        let text = svc.to_text();
        if let Err(e) = writer::write_text_to_file(&self.config_path, &text) {
            log::error!("MappingDialog: failed to save config: {}", e);
            return;
        }

        self.refresh_user_tree();
        self.refresh_preset_tree();
        self.base_mut().emit_signal("config_saved", &[]);

        log::debug!("MappingDialog: saved config to '{}'", self.config_path);
    }

    /// Sync the SpinBox from the service's timeout value. Signals are blocked
    /// to prevent a feedback loop (set_value would trigger on_timeout_changed).
    fn update_timeout_spinbox(&mut self, svc: &MappingService) {
        let Some(spinbox) = &mut self.timeout_spinbox else {
            return;
        };
        let ms = svc
            .timeoutlen()
            .unwrap_or(u32::try_from(crate::settings::defaults::TIMEOUTLEN).unwrap_or(1000));

        spinbox.set_block_signals(true);
        spinbox.set_value(ms as f64);
        spinbox.set_block_signals(false);
    }

    fn update_config_path_label(&mut self) {
        if let Some(label) = &mut self.config_path_label {
            let text = format!("Config: {}", self.config_path);
            label.set_text(&GString::from(text.as_str()));
        }
    }

    // ── Tree population ──────────────────────────────────────────────

    /// Rebuild the user mappings tree. Rows are pre-grouped by the service
    /// (same LHS + RHS + Kind = one visual row with merged mode checkboxes);
    /// we only apply the mode/search filter here.
    fn refresh_user_tree(&mut self) {
        let Some(tree) = &mut self.user_tree else {
            return;
        };
        tree.clear();
        let Some(mut root) = tree.create_item() else {
            return;
        };
        let Some(svc) = &self.service else { return };

        let search_text = self
            .search_input
            .as_ref()
            .map_or(String::new(), |s| s.get_text().to_string());
        let mode_filter_idx = self.mode_filter.as_ref().map_or(0, |f| f.get_selected());

        let rows = svc.user_mappings(&search_text);

        for row in &rows {
            if !row_passes_mode_filter(row, mode_filter_idx) {
                continue;
            }
            Self::add_user_row(&mut root, row);
        }
    }

    fn add_user_row(root: &mut Gd<godot::classes::TreeItem>, row: &UserMappingRow) {
        let Some(mut item) = root.create_child() else {
            return;
        };

        // Stash doc_indices as metadata so edit/delete can locate all config
        // lines belonging to this visual group (one row may span multiple lines).
        let arr = PackedInt64Array::from(
            row.id
                .0
                .iter()
                .map(|&i| i as i64)
                .collect::<Vec<i64>>()
                .as_slice(),
        );
        item.set_metadata(0, &Variant::from(arr));

        item.set_text(COL_LHS, &GString::from(&row.lhs));
        item.set_editable(COL_LHS, true);
        item.set_text(COL_RHS, &GString::from(&row.rhs));
        item.set_editable(COL_RHS, true);

        for (col, checked) in [
            (COL_N, row.modes.contains(ModeSet::NORMAL)),
            (COL_I, row.modes.contains(ModeSet::INSERT)),
            (COL_V, row.modes.contains(ModeSet::VISUAL)),
            (COL_O, row.modes.contains(ModeSet::OPERATOR)),
            (COL_C, row.modes.contains(ModeSet::COMMAND)),
        ] {
            item.set_cell_mode(col, godot::classes::tree_item::TreeCellMode::CHECK);
            item.set_checked(col, checked);
            item.set_editable(col, true);
        }

        let type_str = if row.kind == MappingKind::NonRecursive {
            "noremap"
        } else {
            "map"
        };
        item.set_text(COL_TYPE, &GString::from(type_str));
    }

    fn refresh_preset_tree(&mut self) {
        let Some(tree) = &mut self.preset_tree else {
            return;
        };
        tree.clear();
        let Some(mut root) = tree.create_item() else {
            return;
        };

        let Some(svc) = &self.service else { return };

        for preset_row in &svc.preset_mappings() {
            let Some(mut item) = root.create_child() else {
                continue;
            };

            item.set_metadata(0, &Variant::from(preset_row.id.0 as i64));

            item.set_cell_mode(PCOL_ENABLED, godot::classes::tree_item::TreeCellMode::CHECK);
            item.set_checked(PCOL_ENABLED, preset_row.enabled);
            item.set_editable(PCOL_ENABLED, true);

            // Presets are read-only except for the enabled toggle.
            item.set_text(PCOL_LHS, &GString::from(&preset_row.lhs));
            item.set_text(PCOL_RHS, &GString::from(&preset_row.rhs));
            item.set_text(
                PCOL_MODES,
                &GString::from(preset_row.modes_display.as_str()),
            );
            item.set_text(PCOL_CATEGORY, &GString::from(preset_row.category));
        }
    }
}
