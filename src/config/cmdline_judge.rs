//! The judge of a command line that keys from a Sandbox mapping made, when
//! it runs.
//!
//! The load policy judges a mapping by what its keys can run, but the user
//! types into a line a mapping left open, and a command-line mapping types
//! into what the user started. vim-core tags such a line and asks this
//! judge about each of its commands before any runs.

use std::collections::HashSet;

use vim_core::execution::{CommandLineJudge, RuntimeCommand, Verdict};

use super::policy::{reached_refusal, sandbox_error, Tier};
use crate::actions::action::ActionRegistry;

/// Refuses a command of a Sandbox line that the load policy would refuse
/// in a Sandbox mapping; a line of Trusted and Builtin keys runs.
pub(crate) struct SandboxLineJudge {
    actions: HashSet<&'static str>,
}

impl SandboxLineJudge {
    /// A judge that allows the `<Action>` and `:action` names of `actions`.
    pub(crate) fn new(actions: Option<&ActionRegistry>) -> Self {
        Self {
            actions: actions
                .into_iter()
                .flat_map(|actions| actions.iter().map(|(_, spec)| spec.id))
                .collect(),
        }
    }
}

impl CommandLineJudge for SandboxLineJudge {
    fn judge(&mut self, command: &RuntimeCommand<'_>) -> Verdict {
        if !command
            .tags
            .iter()
            .any(|tag| Tier::of(*tag) == Tier::Sandbox)
        {
            return Verdict::Allow;
        }
        let is_action = |name: &str| self.actions.contains(name);
        match reached_refusal(command.command, command.reach, &is_action) {
            None => Verdict::Allow,
            Some(reason) => Verdict::Refuse(
                sandbox_error(command.text, &format!("Sandbox mapping: {reason}")).into(),
            ),
        }
    }
}
