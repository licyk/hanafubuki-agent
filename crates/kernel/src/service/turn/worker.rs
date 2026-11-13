use crate::loop_control::{sampling_progress_digest, AgentFollowUpReason};
use crate::model::AgentMessageId;
use crate::model::{AgentError, AgentEventSink, AgentTurnOutcome};
use crate::service::{exact_turn, AgentService, TurnExecutionGuard, TurnIdentity, TurnKind};
use hanafubuki_agent_foundation::panic::catch_panic_async;
use std::sync::Arc;
use std::time::Instant;

impl AgentService {
    /// Runs an admitted turn to its terminal commit; its outcome is `None`
    /// for a turn that ends with no answer (a `!command`'s).
    pub(super) async fn execute_admitted_turn(
        &self,
        identity: TurnIdentity,
        event_sink: Arc<dyn AgentEventSink>,
    ) -> Result<Option<AgentTurnOutcome>, AgentError> {
        let mut guard = TurnExecutionGuard {
            service: self.clone(),
            identity,
            event_sink: event_sink.clone(),
            armed: true,
        };
        let started = Instant::now();
        log::info!(
            "agent_diag stage=turn.loop.begin session_id={} turn_id={} generation={}",
            identity.session_id,
            identity.turn_id,
            identity.generation
        );
        // A panic in the turn's own work ends the turn as an internal
        // failure, not as the cancellation the drop guard would commit.
        let result = match catch_panic_async(self.drive_turn(identity, event_sink.as_ref())).await {
            Ok(result) => result,
            Err(message) => {
                log::error!(
                    "Agent turn panicked session_id={} turn_id={} generation={}: {message}",
                    identity.session_id,
                    identity.turn_id,
                    identity.generation
                );
                self.inner.kernel_faults.kernel_fault("turn", &message);
                self.abort_pending_approvals(identity);
                let error = AgentError::Internal("the turn failed unexpectedly".to_string());
                self.finish_failed(identity, &error, event_sink.as_ref())
                    .await?;
                Err(error)
            }
        };
        log::info!(
            "agent_diag stage=turn.loop.end session_id={} turn_id={} generation={} outcome={} elapsed_ms={}",
            identity.session_id,
            identity.turn_id,
            identity.generation,
            if result.is_ok() { "ok" } else { "error" },
            started.elapsed().as_millis()
        );
        guard.armed = false;
        log::info!(
            "agent_diag stage=turn.start.end session_id={} turn_id={} generation={} outcome={} elapsed_ms={}",
            identity.session_id,
            identity.turn_id,
            identity.generation,
            if result.is_ok() { "ok" } else { "error" },
            started.elapsed().as_millis()
        );
        result
    }

    /// Runs the turn's sampling loop (or its `!command`) to its terminal
    /// commit. At the turn's deadline the turn is cancelled as a user's
    /// cancel would, and its end is committed as a failure that names the
    /// limit.
    async fn drive_turn(
        &self,
        identity: TurnIdentity,
        event_sink: &dyn AgentEventSink,
    ) -> Result<Option<AgentTurnOutcome>, AgentError> {
        let deadline = tokio::time::sleep(self.inner.limits.turn_deadline);
        tokio::pin!(deadline);
        let mut deadline_passed = false;
        let mut deadline_cancelled = false;
        loop {
            let loop_result = {
                let run = self.run_turn(identity, event_sink);
                tokio::pin!(run);
                loop {
                    tokio::select! {
                        result = &mut run => break result,
                        () = &mut deadline, if !deadline_passed => {
                            deadline_passed = true;
                            deadline_cancelled = self.cancel_at_deadline(identity).await;
                        }
                    }
                }
            };
            match loop_result {
                Ok(answer) => match self.finish_completed(identity, answer, event_sink).await {
                    Err(AgentError::SteeringPending) => {
                        self.consume_pending_steering(identity, event_sink).await?;
                        let reasons = std::collections::BTreeSet::from([
                            AgentFollowUpReason::PendingUserInput,
                        ]);
                        let digest = sampling_progress_digest(&[], &[], &reasons);
                        self.observe_follow_up_progress(identity, digest).await?;
                        continue;
                    }
                    result => return result,
                },
                Err(AgentError::Cancelled) if deadline_cancelled => {
                    let error = AgentError::TurnDeadlineExceeded {
                        deadline: self.inner.limits.turn_deadline,
                    };
                    self.finish_failed(identity, &error, event_sink).await?;
                    return Err(error);
                }
                Err(AgentError::Cancelled) => {
                    self.finish_cancelled(identity, event_sink).await?;
                    return Err(AgentError::Cancelled);
                }
                Err(error) => {
                    self.finish_failed(identity, &error, event_sink).await?;
                    return Err(error);
                }
            }
        }
    }

    /// What the turn runs: the sampling loop, to its final answer, or the
    /// person's command, to no answer.
    async fn run_turn(
        &self,
        identity: TurnIdentity,
        event_sink: &dyn AgentEventSink,
    ) -> Result<Option<(AgentMessageId, String)>, AgentError> {
        let kind = {
            let cell = self.session(identity.session_id)?;
            let record = cell.lock()?;
            exact_turn(&record, identity.turn_id, identity.generation)?
                .kind
                .clone()
        };
        match kind {
            TurnKind::Regular => self.run_loop(identity, event_sink).await.map(Some),
            TurnKind::UserShellCommand(command) => self
                .run_user_shell_command(identity, command, event_sink)
                .await
                .map(|()| None),
        }
    }

    /// Cancels a turn that reached its deadline; false when the user had
    /// cancelled it already or it is no longer running.
    async fn cancel_at_deadline(&self, identity: TurnIdentity) -> bool {
        let cancelled_already = self
            .session(identity.session_id)
            .and_then(|cell| {
                let record = cell.lock()?;
                Ok(exact_turn(&record, identity.turn_id, identity.generation)?
                    .cancellation
                    .is_cancelled())
            })
            .unwrap_or(true);
        if cancelled_already {
            return false;
        }
        log::warn!(
            "agent_diag stage=turn.deadline session_id={} turn_id={} generation={} deadline_s={}",
            identity.session_id,
            identity.turn_id,
            identity.generation,
            self.inner.limits.turn_deadline.as_secs()
        );
        self.request_turn_cancellation(identity.session_id, identity.turn_id, identity.generation)
            .await
            .is_ok()
    }
}
