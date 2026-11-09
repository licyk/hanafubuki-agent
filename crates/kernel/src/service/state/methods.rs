use super::*;

impl AgentService {
    pub(in crate::service) async fn set_phase_and_emit(
        &self,
        identity: TurnIdentity,
        phase: AgentTurnPhase,
        event: AgentEventKind,
        event_sink: &dyn AgentEventSink,
    ) -> Result<(), AgentError> {
        let cell = self.session(identity.session_id)?;
        let (staged, event) = {
            let mut guard = cell.lock_after_persistence().await?;
            let record = &mut *guard;
            let previous = record.rollback_state();
            exact_turn_mut(record, identity)?.phase = phase;
            let event = record_event(record, identity, event, self.inner.limits);
            (StagedPersistence::delta(&cell, record, previous), event)
        };
        self.persist_staged(staged).await?;
        event_sink.emit(event);
        Ok(())
    }

    pub(in crate::service) async fn emit(
        &self,
        identity: TurnIdentity,
        event: AgentEventKind,
        event_sink: &dyn AgentEventSink,
    ) -> Result<(), AgentError> {
        let cell = self.session(identity.session_id)?;
        let event = {
            let mut guard = cell.lock_after_persistence().await?;
            let record = &mut *guard;
            exact_turn_mut(record, identity)?;
            record_event(record, identity, event, self.inner.limits)
        };
        event_sink.emit(event);
        Ok(())
    }

    pub(in crate::service) async fn reasoning_item_started(
        &self,
        identity: TurnIdentity,
        event_sink: &dyn AgentEventSink,
    ) -> Result<(), AgentError> {
        self.commit_reasoning_transition(identity, event_sink, |checkpoint, now| {
            checkpoint.item_started(now)
        })
        .await
    }

    pub(in crate::service) async fn reasoning_item_completed(
        &self,
        identity: TurnIdentity,
        event_sink: &dyn AgentEventSink,
    ) -> Result<(), AgentError> {
        self.commit_reasoning_transition(identity, event_sink, |checkpoint, now| {
            checkpoint.item_completed(now)
        })
        .await
    }

    async fn commit_reasoning_transition(
        &self,
        identity: TurnIdentity,
        event_sink: &dyn AgentEventSink,
        transition: impl FnOnce(
            &mut AgentReasoningCheckpoint,
            u128,
        ) -> Result<bool, AgentReasoningTransitionError>,
    ) -> Result<(), AgentError> {
        let cell = self.session(identity.session_id)?;
        let (staged, event) = {
            let mut guard = cell.lock_after_persistence().await?;
            let record = &mut *guard;
            let previous = record.rollback_state();
            let (reasoning, publish_snapshot) = {
                let turn = exact_turn_mut(record, identity)?;
                let mut next = turn.reasoning;
                let publish_snapshot =
                    transition(&mut next, now_millis()).map_err(map_reasoning_error)?;
                turn.reasoning = next;
                (turn.reasoning.snapshot(), publish_snapshot)
            };
            let event = if publish_snapshot {
                Some(record_event(
                    record,
                    identity,
                    AgentEventKind::ReasoningUpdated { reasoning },
                    self.inner.limits,
                ))
            } else {
                let now = now_millis();
                record.status.updated_at = now;
                exact_turn_mut(record, identity)?.updated_at = now;
                record.status.revision = record
                    .status
                    .revision
                    .checked_add(1)
                    .expect("agent snapshot revision exhausted");
                None
            };
            (StagedPersistence::delta(&cell, record, previous), event)
        };
        self.persist_staged(staged).await?;
        if let Some(event) = event {
            event_sink.emit(event);
        }
        Ok(())
    }

    pub(in crate::service) async fn reserve_message_id(
        &self,
        identity: TurnIdentity,
    ) -> Result<AgentMessageId, AgentError> {
        let cell = self.session(identity.session_id)?;
        let mut guard = cell.lock_after_persistence().await?;
        let record = &mut *guard;
        exact_turn_mut(record, identity)?;
        let message_id = AgentMessageId::new(record.conversation.next_message_id);
        record.conversation.next_message_id = record
            .conversation
            .next_message_id
            .checked_add(1)
            .expect("agent message id exhausted");
        Ok(message_id)
    }

    /// Reasoning ids are drawn from the message counter: one sequence for
    /// every item of the conversation that streams.
    pub(in crate::service) async fn reserve_reasoning_id(
        &self,
        identity: TurnIdentity,
    ) -> Result<AgentReasoningId, AgentError> {
        let message_id = self.reserve_message_id(identity).await?;
        Ok(AgentReasoningId::new(message_id.value()))
    }

    pub(in crate::service) fn current_sampling_step(
        &self,
        identity: TurnIdentity,
    ) -> Result<u32, AgentError> {
        let cell = self.session(identity.session_id)?;
        let record = cell.lock()?;
        Ok(exact_turn_identity(&record, identity)?.sampling_step)
    }

    pub(in crate::service) fn check_cancelled(
        &self,
        identity: TurnIdentity,
    ) -> Result<(), AgentError> {
        let cell = self.session(identity.session_id)?;
        let record = cell.lock()?;
        if exact_turn_identity(&record, identity)?
            .cancellation
            .is_cancelled()
        {
            return Err(AgentError::Cancelled);
        }
        Ok(())
    }

    pub(in crate::service) fn turn_budget_policy(
        &self,
        identity: TurnIdentity,
    ) -> Result<AgentTurnBudgetPolicy, AgentError> {
        let cell = self.session(identity.session_id)?;
        let record = cell.lock()?;
        exact_turn_identity(&record, identity)?;
        Ok(record.settings.budget_policy)
    }

    pub(in crate::service) async fn reserve_tool_invocations(
        &self,
        identity: TurnIdentity,
        count: usize,
    ) -> Result<(), AgentError> {
        let cell = self.session(identity.session_id)?;
        let staged = {
            let mut guard = cell.lock_after_persistence().await?;
            let record = &mut *guard;
            let previous = record.rollback_state();
            let policy = record.settings.budget_policy;
            exact_turn_mut(record, identity)?
                .budget_ledger
                .record_tool_invocations(count, policy)
                .map_err(map_budget_error)?;
            StagedPersistence::delta(&cell, record, previous)
        };
        self.persist_staged(staged).await?;
        Ok(())
    }

    pub(in crate::service) async fn observe_follow_up_progress(
        &self,
        identity: TurnIdentity,
        digest: crate::AgentProgressDigest,
    ) -> Result<(), AgentError> {
        let cell = self.session(identity.session_id)?;
        let staged = {
            let mut guard = cell.lock_after_persistence().await?;
            let record = &mut *guard;
            let previous = record.rollback_state();
            let policy = record.settings.budget_policy;
            exact_turn_mut(record, identity)?
                .no_progress
                .observe_follow_up(digest, policy.maximum_consecutive_stalled_steps)
                .map_err(map_no_progress_error)?;
            StagedPersistence::delta(&cell, record, previous)
        };
        self.persist_staged(staged).await?;
        Ok(())
    }

    pub(in crate::service) fn allocate_turn_id(&self) -> AgentTurnId {
        let id = self.inner.next_turn_id.fetch_add(1, Ordering::AcqRel);
        assert!(id != u64::MAX, "agent turn id exhausted");
        AgentTurnId::new(id)
    }

    pub(in crate::service) fn allocate_compaction_id(&self) -> AgentCompactionId {
        let id = self.inner.next_compaction_id.fetch_add(1, Ordering::AcqRel);
        assert!(id != u64::MAX, "agent compaction id exhausted");
        AgentCompactionId::new(id)
    }
}
