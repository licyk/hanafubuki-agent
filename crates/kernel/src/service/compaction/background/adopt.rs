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

/// A background compaction installed for a checkpoint.
pub(in crate::service) enum AdoptedCompaction {
    /// The checkpoint installed it.
    Installed(AgentCompactionOutcome),
    /// Another checkpoint took it while this one waited, and installed it
    /// (a turn admitted while `/compact` waited).
    InstalledElsewhere(AgentCompactionOutcome),
}

/// What a checkpoint finds in a run that is no longer going.
enum Taken {
    /// The result, now the taker's to install.
    Result(Box<(CompactionPlan, CompactionSample)>),
    InstalledElsewhere(Box<AgentCompactionOutcome>),
    Nothing,
}

impl BackgroundRun {
    /// Takes what the run left; `None` while it runs or another checkpoint
    /// installs its result.
    fn take(&self) -> Option<Taken> {
        let mut outcome = self.outcome.lock();
        let (kept, taken) = match std::mem::replace(&mut *outcome, BackgroundOutcome::Spent) {
            BackgroundOutcome::Ready(result) => {
                (BackgroundOutcome::Installing, Some(Taken::Result(result)))
            }
            BackgroundOutcome::Installed(installed) => (
                BackgroundOutcome::Installed(installed.clone()),
                Some(Taken::InstalledElsewhere(installed)),
            ),
            BackgroundOutcome::Spent => (BackgroundOutcome::Spent, Some(Taken::Nothing)),
            pending @ (BackgroundOutcome::Running | BackgroundOutcome::Installing) => {
                (pending, None)
            }
        };
        *outcome = kept;
        taken
    }
}

/// A taken result being installed: what it ends as, for a checkpoint that
/// waits for it, is `Installed` once the install succeeds and `Spent`
/// otherwise, however the installing checkpoint stops.
struct InstallClaim {
    cell: Arc<SessionCell>,
    run: Arc<BackgroundRun>,
    installed: Option<AgentCompactionOutcome>,
}

impl Drop for InstallClaim {
    fn drop(&mut self) {
        *self.run.outcome.lock() = match self.installed.take() {
            Some(outcome) => BackgroundOutcome::Installed(Box::new(outcome)),
            None => BackgroundOutcome::Spent,
        };
        self.cell.notify_terminal();
    }
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
    ) -> Result<Option<AdoptedCompaction>, AgentError> {
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
        let taken = match adoption {
            BackgroundAdoption::Ready => match run.take() {
                Some(taken) => taken,
                None => return Ok(None),
            },
            BackgroundAdoption::Required(cancellation) => {
                cell.wait_until(|_| {
                    if cancellation.is_some_and(CancellationToken::is_cancelled) {
                        return Err(AgentError::Cancelled);
                    }
                    Ok(run.take())
                })
                .await?
            }
        };
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
        let result = match taken {
            Taken::Result(result) => result,
            Taken::InstalledElsewhere(outcome) => {
                return Ok(Some(AdoptedCompaction::InstalledElsewhere(*outcome)))
            }
            Taken::Nothing => return Ok(None),
        };
        let mut claim = InstallClaim {
            cell: cell.clone(),
            run: run.clone(),
            installed: None,
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
                claim.installed = Some(outcome.clone());
                event_sink.emit(event);
                Ok(Some(AdoptedCompaction::Installed(outcome)))
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
