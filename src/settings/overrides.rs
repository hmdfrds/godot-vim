//! Project overrides of the keys only the user decides ([`keys::USER_ONLY`]).
//!
//! The reader already ignores them. This tells the user, once per override,
//! through Godot's warning channel, because Log Level defaults to Off.

use super::keys;

/// ProjectSettings prefix of a project's Editor Settings overrides
/// (`EDITOR_SETTING_OVERRIDE_PREFIX`, core/config/project_settings.h).
const OVERRIDE_PREFIX: &str = "editor_overrides/";

/// The user-only keys a project overrides, given ProjectSettings' `has_setting`.
pub(crate) fn overridden(has_setting: impl Fn(&str) -> bool) -> Vec<&'static str> {
    keys::USER_ONLY
        .iter()
        .copied()
        .filter(|key| has_setting(&format!("{OVERRIDE_PREFIX}{key}")))
        .collect()
}

/// Which overrides were already reported.
#[derive(Debug, Default)]
pub(crate) struct OverrideNotice {
    reported: Vec<&'static str>,
}

impl OverrideNotice {
    /// The warning for overrides not reported yet. An override that goes away
    /// is forgotten, so adding it back reports it again.
    pub(crate) fn update(&mut self, current: &[&'static str]) -> Option<String> {
        self.reported.retain(|key| current.contains(key));
        let new: Vec<&str> = current
            .iter()
            .copied()
            .filter(|key| !self.reported.contains(key))
            .collect();
        if new.is_empty() {
            return None;
        }
        self.reported.extend(&new);
        Some(format!(
            "GodotVim ignores this project's editor_overrides for {}. A project cannot \
             change these settings; your own Editor Settings values apply.",
            new.join(", ")
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_user_only_key_is_found_under_the_override_prefix() {
        let found = overridden(|name| name.starts_with("editor_overrides/plugins/GodotVim/"));
        assert_eq!(found, keys::USER_ONLY);
    }

    #[test]
    fn an_override_of_another_setting_is_not_ours_to_report() {
        let found = overridden(|name| name == "editor_overrides/plugins/GodotVim/editor/textwidth");
        assert!(found.is_empty());
    }

    #[test]
    fn a_plain_project_setting_with_the_same_path_is_not_an_override() {
        let found = overridden(|name| name == keys::PROJECT_VIMRC);
        assert!(found.is_empty());
    }

    #[test]
    fn each_override_is_reported_once() {
        let mut notice = OverrideNotice::default();
        let message = notice
            .update(&[keys::PROJECT_VIMRC])
            .expect("first sighting");
        assert!(message.contains(keys::PROJECT_VIMRC), "{message}");
        assert_eq!(notice.update(&[keys::PROJECT_VIMRC]), None);
    }

    #[test]
    fn a_new_override_is_reported_alone() {
        let mut notice = OverrideNotice::default();
        notice.update(&[keys::PROJECT_VIMRC]);
        let message = notice
            .update(&[keys::PROJECT_VIMRC, keys::SHELL_EXECUTION])
            .expect("a new override");
        assert!(message.contains(keys::SHELL_EXECUTION), "{message}");
        assert!(!message.contains(keys::PROJECT_VIMRC), "{message}");
    }

    #[test]
    fn an_override_removed_and_added_back_is_reported_again() {
        let mut notice = OverrideNotice::default();
        notice.update(&[keys::SHELL_EXECUTION]);
        assert_eq!(notice.update(&[]), None);
        assert!(notice.update(&[keys::SHELL_EXECUTION]).is_some());
    }

    #[test]
    fn no_override_says_nothing() {
        assert_eq!(OverrideNotice::default().update(&[]), None);
    }
}
