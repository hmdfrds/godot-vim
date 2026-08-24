use crate::actions::resolve::Disposition;
use vim_core::execution::host_api::ProcessResult;

/// How the overlay's disposition becomes a pipeline outcome.
///
/// Production logic, not a test mirror. It lived inline in
/// `process_cycle_impl`, which takes `&mut Gd<CodeEdit>` and so cannot be
/// called under `cargo test`; a test could only restate the match, and a
/// restated match keeps passing while the real one is edited. Extracted for
/// the reason the rest of this codebase extracts pure logic from Gd-bound
/// call sites: so the assertion has something real to bite on.
///
/// `None` means the overlay did not claim the key, so `process_cycle_impl`
/// falls through to `should_passthrough_key`, exactly as the transport's
/// `None` did before the pipeline was unified.
pub(crate) const fn outcome_for(disposition: Disposition) -> Option<PipelineOutcome> {
    match disposition {
        Disposition::Consume => Some(PipelineOutcome::CompletionConsumed),
        // Not consumed and the engine skipped, so CodeEdit still needs the
        // event: `should_mark_handled()` must stay false for this one.
        Disposition::Handoff => Some(PipelineOutcome::CompletionDeferred),
        Disposition::Ignore => None,
    }
}

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

    /// `process_cycle_impl` still wires the overlay to the pipeline.
    ///
    /// Source-level, in the shape `plugin/signals.rs` already uses, because
    /// `process_cycle_impl` takes `&mut Gd<CodeEdit>` and cannot be called
    /// under `cargo test`. Every unit below it is tested on its own, which is
    /// exactly the gap: `advance` has a six-row table and `outcome_for` has a
    /// truth table, and deleting the CALL to either left the whole suite
    /// green under mutation. These three lines are the wiring no other test
    /// can see.
    #[test]
    fn the_overlay_is_still_wired_into_the_keystroke_pipeline() {
        let src = include_str!("process.rs");
        let production = src.split_once("\n#[cfg(test)]").map_or(src, |(h, _)| h);
        for (needle, why) in [
            (
                "completion::advance(ctx.transient.completion, &plan.facts)",
                "the provenance machine must advance on every keystroke, or an \
                 inherited selection survives the typing that should clear it and \
                 <CR> confirms a candidate the user never chose",
            ),
            (
                "pipeline_outcome::outcome_for(disposition)",
                "the overlay's disposition must reach the pipeline outcome, or a \
                 handoff is indistinguishable from a consume",
            ),
            (
                "completion::dispatch_overlay(",
                "the overlay plan must actually be dispatched",
            ),
        ] {
            assert!(
                production.contains(needle),
                "process.rs no longer contains `{needle}`: {why}"
            );
        }
    }

    #[test]
    fn the_disposition_mapping_is_a_total_truth_table() {
        // Calls the PRODUCTION mapping. It used to restate the match instead,
        // on an anti-tautology argument that does not hold here: duplicating
        // `dispose` in the completion tests is sound because `dispose` is an
        // independent oracle, but the mapping IS the thing under test, so a
        // copy asserts only that the copy is self-consistent. Verified by
        // mutation: flipping `Handoff` or `Ignore` in `process_cycle_impl`
        // left the whole 1498-test suite green.
        assert!(matches!(
            outcome_for(Disposition::Consume),
            Some(PipelineOutcome::CompletionConsumed)
        ));
        assert!(matches!(
            outcome_for(Disposition::Handoff),
            Some(PipelineOutcome::CompletionDeferred)
        ));
        assert!(
            outcome_for(Disposition::Ignore).is_none(),
            "Ignore must fall through to should_passthrough_key, not claim the key"
        );
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
