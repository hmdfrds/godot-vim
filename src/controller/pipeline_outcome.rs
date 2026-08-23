use vim_core::execution::host_api::ProcessResult;

pub(crate) enum PipelineOutcome {
    VimdebugStep,
    CompletionConsumed,
    CompletionDeferred,
    Passthrough,
    EngineConsumed(#[allow(dead_code)] ProcessResult),
    EngineIgnored(#[allow(dead_code)] ProcessResult),
}

impl PipelineOutcome {
    pub(crate) fn should_mark_handled(&self) -> bool {
        matches!(
            self,
            Self::VimdebugStep | Self::CompletionConsumed | Self::EngineConsumed(_)
        )
    }

    pub(crate) fn may_have_moved_cursor(&self) -> bool {
        // CompletionConsumed always moves the cursor (replaces prefix with
        // full word). The deferred caret_changed must expect cursor movement
        // so it isn't falsely treated as an external edit (Fix 4C).
        matches!(self, Self::CompletionConsumed | Self::EngineConsumed(_))
    }

    pub(crate) fn log_label(&self) -> &'static str {
        match self {
            Self::VimdebugStep => "vimdebug-step",
            Self::CompletionConsumed => "completion-consumed",
            Self::CompletionDeferred => "completion-deferred",
            Self::Passthrough => "passthrough",
            Self::EngineConsumed(_) => "engine-consumed",
            Self::EngineIgnored(_) => "engine-ignored",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dummy_result() -> ProcessResult {
        ProcessResult {
            consumed: true,
            host_requests: Vec::new(),
            deferred_actions: Vec::new(),
        }
    }

    #[test]
    fn the_disposition_mapping_is_a_total_truth_table() {
        // A pure mirror of the match in `process_cycle_impl`, written out
        // rather than called so the assertion is not a tautology. Exhaustive:
        // a fourth `Disposition` is a compile error here before it is a
        // routing surprise there.
        use crate::actions::resolve::Disposition;
        const fn map(d: Disposition) -> Option<PipelineOutcome> {
            match d {
                Disposition::Consume => Some(PipelineOutcome::CompletionConsumed),
                Disposition::Handoff => Some(PipelineOutcome::CompletionDeferred),
                Disposition::Ignore => None,
            }
        }
        assert!(matches!(
            map(Disposition::Consume),
            Some(PipelineOutcome::CompletionConsumed)
        ));
        assert!(matches!(
            map(Disposition::Handoff),
            Some(PipelineOutcome::CompletionDeferred)
        ));
        assert!(map(Disposition::Ignore).is_none());
        // The two obligations the mapping carries downstream: a deferred key
        // must NOT be marked handled (CodeEdit still needs the event), and a
        // consumed completion always moved the cursor (Fix 4C).
        assert!(!PipelineOutcome::CompletionDeferred.should_mark_handled());
        assert!(PipelineOutcome::CompletionConsumed.may_have_moved_cursor());
    }

    #[test]
    fn should_mark_handled_truth_table() {
        assert!(PipelineOutcome::VimdebugStep.should_mark_handled());
        assert!(PipelineOutcome::CompletionConsumed.should_mark_handled());
        assert!(!PipelineOutcome::CompletionDeferred.should_mark_handled());
        assert!(!PipelineOutcome::Passthrough.should_mark_handled());
        assert!(PipelineOutcome::EngineConsumed(dummy_result()).should_mark_handled());
        assert!(!PipelineOutcome::EngineIgnored(dummy_result()).should_mark_handled());
    }

    #[test]
    fn may_have_moved_cursor_truth_table() {
        assert!(!PipelineOutcome::VimdebugStep.may_have_moved_cursor());
        assert!(PipelineOutcome::CompletionConsumed.may_have_moved_cursor());
        assert!(!PipelineOutcome::CompletionDeferred.may_have_moved_cursor());
        assert!(!PipelineOutcome::Passthrough.may_have_moved_cursor());
        assert!(PipelineOutcome::EngineConsumed(dummy_result()).may_have_moved_cursor());
        assert!(!PipelineOutcome::EngineIgnored(dummy_result()).may_have_moved_cursor());
    }

    #[test]
    fn log_label_truth_table() {
        assert_eq!(PipelineOutcome::VimdebugStep.log_label(), "vimdebug-step");
        assert_eq!(
            PipelineOutcome::CompletionConsumed.log_label(),
            "completion-consumed"
        );
        assert_eq!(
            PipelineOutcome::CompletionDeferred.log_label(),
            "completion-deferred"
        );
        assert_eq!(PipelineOutcome::Passthrough.log_label(), "passthrough");
        assert_eq!(
            PipelineOutcome::EngineConsumed(dummy_result()).log_label(),
            "engine-consumed"
        );
        assert_eq!(
            PipelineOutcome::EngineIgnored(dummy_result()).log_label(),
            "engine-ignored"
        );
    }
}
