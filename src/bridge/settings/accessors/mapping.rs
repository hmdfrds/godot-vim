use super::internal::{get_bool_setting, get_int_setting, get_mapping_array, get_mapping_dictionary};
use super::VimSettings;
use crate::bridge::settings::{defaults, keys};

impl VimSettings {
    /// Returns whether custom keymappings are enabled (like jj -> Esc).
    #[must_use]
    pub fn mapping_enabled() -> bool {
        get_bool_setting(keys::MAPPING_ENABLED, defaults::MAPPING_ENABLED)
    }

    /// Returns the timeout for key sequences in milliseconds.
    /// If another key isn't pressed within this time, pending keys are processed literally.
    /// Default: 500ms
    #[must_use]
    #[allow(
        clippy::cast_sign_loss,
        reason = "Timeout is always positive in settings"
    )]
    pub fn timeoutlen() -> u64 {
        get_int_setting(keys::MAPPING_TIMEOUTLEN, defaults::MAPPING_TIMEOUTLEN) as u64
    }

    /// Returns insert mode mappings as from->to pairs.
    #[must_use]
    pub fn imap() -> Vec<(String, String)> {
        get_mapping_dictionary(keys::IMAP)
    }

    /// Returns normal mode mappings as from->to pairs.
    #[must_use]
    pub fn nmap() -> Vec<(String, String)> {
        get_mapping_dictionary(keys::NMAP)
    }

    /// Returns visual mode mappings as from->to pairs.
    #[must_use]
    pub fn vmap() -> Vec<(String, String)> {
        get_mapping_dictionary(keys::VMAP)
    }

    /// Returns global mode mappings as from->to pairs.
    /// These mappings apply throughout the editor, including outside CodeEdit.
    #[must_use]
    pub fn gmap() -> Vec<(String, String)> {
        get_mapping_dictionary(keys::GMAP)
    }

    /// Returns all mappings from the unified array (from, to, modes).
    /// This is the preferred source of truth over imap/nmap/vmap.
    #[must_use]
    pub fn all_mappings() -> Vec<(String, String, String)> {
        get_mapping_array(keys::ALL_MAPPINGS)
    }
}
