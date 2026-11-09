use super::*;

impl AgentService {
    pub async fn request_turn_cancellation(
        &self,
        session_id: AgentSessionId,
        turn_id: AgentTurnId,
        generation: AgentGeneration,
    ) -> Result<(), AgentError> {
        let identity = TurnIdentity {
            session_id,
            turn_id,
            generation,
        };
        let cell = self.session(session_id)?;
        {
            let mut guard = cell.lock_after_persistence().await?;
            let record = &mut *guard;
            if exact_turn(record, turn_id, generation)?.phase.is_terminal() {
                return Err(AgentError::TurnNotRunning);
            }
            if let Some(compaction) = &record.runtime.compaction {
                if !compaction.snapshot.status.is_terminal()
                    && compaction.snapshot.trigger == AgentCompactionTrigger::Automatic
                    && compaction.snapshot.turn_generation == Some(generation)
                {
                    compaction.cancellation.cancel();
                }
            }
            let turn = exact_turn_mut(record, identity)?;
            let pending_steering_count = turn.pending_steering.len();
            turn.budget_ledger
                .consume_pending_input_items(pending_steering_count)
                .map_err(map_budget_error)?;
            turn.pending_steering.clear();
            turn.cancellation.cancel();
            // Wakes a turn waiting for its session's background compaction.
            cell.notify_terminal();
        }
        self.abort_pending_approvals(identity);
        self.abort_pending_user_inputs(identity).await;
        Ok(())
    }

    /// Cancels the turn and confirms it reached `Cancelled` durably: first
    /// cooperatively, then by aborting its managed task, within 5 s. The
    /// waits are asynchronous.
    pub async fn cancel_turn_confirmed(
        &self,
        session_id: AgentSessionId,
        turn_id: AgentTurnId,
        generation: AgentGeneration,
    ) -> Result<AgentTurnCancellationOutcome, AgentError> {
        const COOPERATIVE_GRACE: Duration = Duration::from_millis(500);
        const CONFIRMATION_DEADLINE: Duration = Duration::from_secs(5);

        let identity = TurnIdentity {
            session_id,
            turn_id,
            generation,
        };
        self.request_turn_cancellation(session_id, turn_id, generation)
            .await?;
        let started = Instant::now();
        if let Some(outcome) = self
            .wait_for_turn_terminal(identity, COOPERATIVE_GRACE)
            .await?
        {
            return Ok(outcome);
        }
        let abort = self
            .inner
            .managed_turns
            .lock()
            .get(&identity)
            .and_then(Clone::clone);
        if let Some(abort) = abort {
            // A turn that staged its end is committing it and emits its
            // events once that is durable, which a slow disk can hold past
            // the grace: aborting it then would drop them. Decided under the
            // record's lock, where a turn stages its end.
            let cell = self.session(session_id)?;
            let record = cell.lock()?;
            if exact_turn(&record, turn_id, generation).is_ok_and(|turn| !turn.phase.is_terminal())
            {
                abort.abort();
            }
        }
        let remaining = CONFIRMATION_DEADLINE.saturating_sub(started.elapsed());
        self.wait_for_turn_terminal(identity, remaining)
            .await?
            .ok_or(AgentError::CancellationNotConfirmed)
    }

    /// Waits until the turn's terminal state is durable, or `timeout` passes
    /// (`Ok(None)`).
    async fn wait_for_turn_terminal(
        &self,
        identity: TurnIdentity,
        timeout: Duration,
    ) -> Result<Option<AgentTurnCancellationOutcome>, AgentError> {
        let cell = self.session(identity.session_id)?;
        let terminal = cell.wait_until(|record| {
            let turn = exact_turn(record, identity.turn_id, identity.generation)?;
            if !turn.phase.is_terminal() || record.status.persisting_revision.is_some() {
                return Ok(None);
            }
            if turn.phase != AgentTurnPhase::Cancelled {
                return Err(AgentError::TurnNotRunning);
            }
            Ok(Some(AgentTurnCancellationOutcome {
                session_id: identity.session_id,
                turn_id: identity.turn_id,
                generation: identity.generation,
                phase: turn.phase,
                reason: turn
                    .terminal_reason
                    .ok_or(AgentError::CancellationNotConfirmed)?,
                revision: AgentRevision::new(record.status.revision),
            }))
        });
        match tokio::time::timeout(timeout, terminal).await {
            Ok(outcome) => outcome.map(Some),
            Err(_) => Ok(None),
        }
    }
}
