//! Mapping Panel - Dock for managing keymappings with checkbox-based mode selection.
//!
//! UI Design:
//! ```text
//! | From | To       | I | N | V | G |
//! |------|----------|---|---|---|---|
//! | jj   | <Esc>    | ✓ |   |   |   |
//! | <Space>f | :FileSystem |   |   |   | ✓ |
//! ```
//!
//! Each mapping can be enabled for multiple modes via checkboxes.
//! G = Global mode (works anywhere in editor, not just CodeEdit)

mod serialization;
mod ui;

use godot::classes::{Button, Control, IControl, LineEdit, SpinBox, Tree};
use godot::prelude::*;
use godot::classes::Node;

use crate::bridge::godot::names::callbacks;

/// Column indices for the tree.
const COL_FROM: i32 = 0;
const COL_TO: i32 = 1;
const COL_IMAP: i32 = 2;
const COL_NMAP: i32 = 3;
const COL_VMAP: i32 = 4;
const COL_GMAP: i32 = 5;

/// Dock panel for managing key mappings with checkbox mode selection.
#[derive(GodotClass)]
#[class(tool, base=Control)]
pub struct MappingPanel {
    pub(super) base: Base<Control>,

    // UI Elements
    pub(super) tree: Option<Gd<Tree>>,
    pub(super) from_input: Option<Gd<LineEdit>>,
    pub(super) to_input: Option<Gd<LineEdit>>,
    pub(super) add_button: Option<Gd<Button>>,
    pub(super) delete_button: Option<Gd<Button>>,
    pub(super) timeout_spinbox: Option<Gd<SpinBox>>,
}

#[godot_api]
impl IControl for MappingPanel {
    fn init(base: Base<Control>) -> Self {
        Self {
            base,
            tree: None,
            from_input: None,
            to_input: None,
            add_button: None,
            delete_button: None,
            timeout_spinbox: None,
        }
    }

    fn ready(&mut self) {
        self.build_ui();
        self.load_mappings_from_settings();
    }
}

#[godot_api]
impl MappingPanel {
    // ─────────────────────────────────────────────────────────────────────────
    // Signal Handlers
    // ─────────────────────────────────────────────────────────────────────────

    #[func]
    fn on_add_pressed(&mut self) {
        let from = self
            .from_input
            .as_ref()
            .map(|i| i.get_text().to_string())
            .unwrap_or_default();
        let to = self
            .to_input
            .as_ref()
            .map(|i| i.get_text().to_string())
            .unwrap_or_default();

        if from.is_empty() || to.is_empty() {
            return;
        }

        let Some(tree) = &mut self.tree else { return };
        let Some(root) = tree.get_root() else { return };

        // Check if mapping with same 'from' already exists - override it
        let mut child = root.get_first_child();
        while let Some(mut item) = child {
            let existing_from = item.get_text(COL_FROM).to_string();
            if existing_from == from {
                // Override existing - update 'to' value
                item.set_text(COL_TO, &to);
                log::info!("Overriding existing mapping: {} -> {}", from, to);

                // Clear inputs and auto-save
                if let Some(from_input) = &mut self.from_input {
                    from_input.clear();
                }
                if let Some(to_input) = &mut self.to_input {
                    to_input.clear();
                }
                self.save_mappings_to_settings();
                self.reload_vim_controllers_after_save();
                return;
            }
            child = item.get_next();
        }

        // Add new mapping with Insert mode checked by default
        self.add_mapping_row(&root, &from, &to, [true, false, false, false]);

        // Clear inputs and auto-save
        if let Some(from_input) = &mut self.from_input {
            from_input.clear();
        }
        if let Some(to_input) = &mut self.to_input {
            to_input.clear();
        }
        self.save_mappings_to_settings();
        self.reload_vim_controllers_after_save();
    }

    #[func]
    fn on_delete_pressed(&mut self) {
        let Some(tree) = &mut self.tree else { return };
        let Some(selected) = tree.get_selected() else {
            log::warn!("No mapping selected");
            return;
        };

        let Some(mut parent) = selected.get_parent() else {
            return;
        };

        parent.remove_child(&selected);

        // Auto-save after delete
        self.save_mappings_to_settings();
        self.reload_vim_controllers_after_save();
        log::info!("Mapping deleted and saved");
    }

    /// Called when any tree cell is edited (text or checkbox)
    #[func]
    fn on_tree_item_edited(&mut self) {
        // Auto-save on any edit
        self.save_mappings_to_settings();
        self.reload_vim_controllers_after_save();
    }

    /// Reload `VimController` mappings after save
    fn reload_vim_controllers_after_save(&self) {
        if let Some(tree) = self.base().get_tree() {
            if let Some(root) = tree.get_root() {
                Self::reload_vim_controllers(&root.upcast());
            }
        }
    }

    /// Called when timeout spinbox value changes
    #[func]
    fn on_timeout_changed(&mut self, _value: f64) {
        self.save_mappings_to_settings();
    }

    #[func]
    fn on_save_pressed(&mut self) {
        self.save_mappings_to_settings();
        log::info!("Mappings saved to EditorSettings");
        self.reload_vim_controllers_after_save();
    }

    /// Called when "Reload Settings" button is pressed.
    /// Use this to apply changes made directly in Editor Settings.
    #[func]
    fn on_reload_settings_pressed(&mut self) {
        // Reload mappings from EditorSettings into the UI
        self.load_mappings_from_settings();

        // Reload all VimController settings
        if let Some(tree) = self.base().get_tree() {
            if let Some(root) = tree.get_root() {
                Self::reload_vim_controllers(&root.upcast());
            }
        }

        log::info!("Settings reloaded from Project Settings");
    }

    /// Recursively find `VimController` nodes and reload all settings.
    fn reload_vim_controllers(node: &Gd<Node>) {
        if node.get_class() == "VimController".into() {
            node.clone().call(callbacks::RELOAD_SETTINGS, &[]);
        }

        for child in node.get_children().iter_shared() {
            Self::reload_vim_controllers(&child);
        }
    }
}
