use super::*;

use crate::service::compaction::commit::{InstallMode, RebaseInstall};

/// When a checkpoint takes the session's background compaction.
pub(in crate::service) enum BackgroundAdoption<'a> {
    /// Installs a result that is ready, waiting for nothing.
    Ready,
    /// A compaction is required now: waits for a run that is still going
    /// and would be taken, until it ends or `cancellation` (the turn's)
    /// fires.
    Required(Option<&'a CancellationToken>),
}

impl AgentService {
    /// Installs the session's background compaction when its result is
    /// usable, before the history added since; the request that follows is
    /// estimated with `tool_plan` and `prompt`. A result for another model
    /// or implementation, or one the install refuses (another window, a
    /// history that is no longer the one it read), is discarded, and so is
    /// a failed run: nothing is installed and the caller compacts in line
    /// when it must.
    pub(in crate::service) async fn adopt_background_compaction(
        &self,
        session_id: AgentSessionId,
        adoption: BackgroundAdoption<'_>,
        tool_plan: &AgentToolPlan,
        prompt: &RequestContext,
        event_sink: &dyn AgentEventSink,
    ) -> Result<Option<AgentCompactionOutcome>, AgentError> {
        let cell = self.session(session_id)?;
        let run = {
            let mut record = cell.lock()?;
            let Some(run) = record.runtime.background_compaction.run.clone() else {
                return Ok(None);
            };
            // The model asked for a new context now: the turn compacts in
            // line, leaving this result stale.
            if record.runtime.context_window.new_context_requested {
                return Ok(None);
            }
            if matches!(*run.outcome.lock(), BackgroundOutcome::Running) {
                let BackgroundAdoption::Required(_) = adoption else {
                    return Ok(None);
                };
                if !run.attempt.matches(&record) {
                    run.cancellation.cancel();
                    record.runtime.background_compaction.run = None;
                    return Ok(None);
                }
            }
            run
        };
        if let BackgroundAdoption::Required(cancellation) = adoption {
            cell.wait_until(|_| {
                if cancellation.is_some_and(CancellationToken::is_cancelled) {
                    return Err(AgentError::Cancelled);
                }
                Ok((!matches!(*run.outcome.lock(), BackgroundOutcome::Running)).then_some(()))
            })
            .await?;
        }
        let model_changed = {
            let mut record = cell.lock()?;
            let slot = &mut record.runtime.background_compaction;
            if slot
                .run
                .as_ref()
                .is_some_and(|current| Arc::ptr_eq(current, &run))
            {
                slot.run = None;
            }
            !run.attempt.same_model(&record)
        };
        let BackgroundOutcome::Ready(result) =
            std::mem::replace(&mut *run.outcome.lock(), BackgroundOutcome::Spent)
        else {
            return Ok(None);
        };
        if model_changed {
            return Ok(None);
        }
        let (plan, sample) = *result;
        let images = self.load_history_images(session_id).await?;
        let install = InstallMode::Rebase(RebaseInstall {
            turn_generation: run.turn_generation,
            images: &images,
            tool_plan,
            prompt,
        });
        match self.commit_compaction(&plan, &sample, install).await {
            Ok((outcome, event)) => {
                event_sink.emit(event);
                Ok(Some(outcome))
            }
            Err(error) => {
                log::debug!(
                    "agent_diag stage=compaction.background_install session_id={session_id} outcome=discarded error={}",
                    bounded_diagnostic(error.to_string())
                );
                Ok(None)
            }
        }
    }
}
