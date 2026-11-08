use super::*;

impl AgentService {
    pub(in crate::service) async fn request_new_context(
        &self,
        identity: TurnIdentity,
    ) -> Result<AgentToolOutput, AgentError> {
        let cell = self.session(identity.session_id)?;
        let staged = {
            let mut guard = cell.lock_after_persistence().await?;
            let record = &mut *guard;
            ensure_no_staged_commit(record)?;
            if exact_turn_identity(record, identity)?.phase.is_terminal() {
                return Err(AgentError::TurnNotRunning);
            }
            if record.runtime.context_window.new_context_requested {
                return Ok(AgentToolOutput::success(json!({
                    "requested": true,
                    "alreadyRequested": true
                })));
            }
            let previous = record.rollback_state();
            record.runtime.context_window.new_context_requested = true;
            record.status.revision = record
                .status
                .revision
                .checked_add(1)
                .expect("revision exhausted");
            record.status.updated_at = now_millis();
            StagedPersistence::commit(&cell, record, previous)
        };
        self.persist_staged(staged).await?;
        Ok(AgentToolOutput::success(json!({
            "requested": true,
            "windowChange": "after_current_response"
        })))
    }

    pub async fn compact_session(
        &self,
        session_id: AgentSessionId,
        generation: AgentGeneration,
        revision: AgentRevision,
    ) -> Result<AgentCompactionOutcome, AgentError> {
        self.compact_session_with_sink(
            session_id,
            generation,
            revision,
            Arc::new(NoopAgentEventSink),
        )
        .await
    }

    pub async fn compact_session_with_sink(
        &self,
        session_id: AgentSessionId,
        generation: AgentGeneration,
        revision: AgentRevision,
        event_sink: Arc<dyn AgentEventSink>,
    ) -> Result<AgentCompactionOutcome, AgentError> {
        if !self.inner.accepting.load(Ordering::Acquire) {
            return Err(AgentError::ShuttingDown);
        }
        let transport = self.prepare_compaction_transport(session_id).await?;
        let plan = self
            .begin_compaction(
                session_id,
                AgentCompactionTrigger::Manual,
                AgentCompactionPhase::StandaloneTurn,
                AgentCompactionReason::UserRequested,
                Some((generation, revision)),
                None,
                transport,
            )
            .await?;
        self.execute_compaction(plan, event_sink.as_ref()).await
    }

    pub fn cancel_compaction(
        &self,
        session_id: AgentSessionId,
        compaction_id: AgentCompactionId,
        generation: AgentCompactionGeneration,
    ) -> Result<(), AgentError> {
        let cell = self.session(session_id)?;
        let record = cell.lock()?;
        let compaction = record
            .runtime
            .compaction
            .as_ref()
            .ok_or(AgentError::CompactionNotRunning)?;
        if compaction.snapshot.id != compaction_id || compaction.snapshot.generation != generation {
            return Err(AgentError::StaleCompaction);
        }
        if compaction.snapshot.status.is_terminal() {
            return Err(AgentError::CompactionNotRunning);
        }
        compaction.cancellation.cancel();
        Ok(())
    }

    /// Compacts before `step` samples when its request would reach the
    /// threshold of the context window (or the turn's budget), or when a new
    /// context was requested; the estimate counts the request `step` sends.
    pub(in crate::service) async fn maybe_auto_compact(
        &self,
        turn_identity: TurnIdentity,
        phase: AgentCompactionPhase,
        step: &SamplingStep,
        event_sink: &dyn AgentEventSink,
    ) -> Result<bool, AgentError> {
        if !matches!(
            phase,
            AgentCompactionPhase::PreTurn | AgentCompactionPhase::MidTurn
        ) {
            return Err(AgentError::Protocol(
                "automatic compaction requires a pre-turn or mid-turn phase".to_string(),
            ));
        }
        let cell = self.session(turn_identity.session_id)?;
        // Everything but the estimate, which needs the history's images:
        // `None` when a new context was requested, otherwise the images.
        let estimate = {
            let guard = cell.lock()?;
            let record = &*guard;
            let _ = exact_turn_identity(record, turn_identity)?;
            if record
                .runtime
                .compaction
                .as_ref()
                .is_some_and(|value| !value.snapshot.status.is_terminal())
            {
                return Err(AgentError::CompactionAlreadyRunning);
            }
            if phase == AgentCompactionPhase::PreTurn
                && record.conversation.history.iter().position(|item| {
                    matches!(
                        &item.content,
                        ConversationItem::TurnStart { turn_id }
                            if *turn_id == turn_identity.turn_id
                    )
                }) == Some(0)
            {
                return Ok(false);
            }
            if phase == AgentCompactionPhase::MidTurn
                && record.runtime.context_window.new_context_requested
            {
                None
            } else if !record.settings.config.auto_compaction_enabled {
                return Ok(false);
            } else {
                Some(referenced_images(
                    record.conversation.history.iter().map(|item| &item.content),
                ))
            }
        };
        let reason = match estimate {
            None => AgentCompactionReason::NewContextRequested,
            Some(attachments) => {
                let images = self
                    .load_images(turn_identity.session_id, attachments)
                    .await;
                let guard = cell.lock()?;
                let record = &*guard;
                let turn = exact_turn_identity(record, turn_identity)?;
                let budget_pressure = turn.budget_ledger.proactive_model_usage_limit_reached(
                    record.settings.budget_policy,
                    record.settings.config.auto_compact_threshold_percent,
                );
                let request = step_model_request(
                    record,
                    &images,
                    &step.tool_plan,
                    &step.prompt,
                    self.inner.limits.maximum_tool_output_bytes,
                )?;
                let estimated = estimate_model_request_tokens(&request)?;
                let active = record
                    .runtime
                    .usage
                    .and_then(|usage| usage.context_consumed_tokens)
                    .map_or(estimated, |reported| reported.max(estimated));
                let status = AgentContextTokenStatus::calculate(
                    active,
                    record.settings.config.context_window_tokens,
                    record.settings.config.auto_compaction_enabled,
                    record.settings.config.auto_compact_threshold_percent,
                );
                if !budget_pressure && !status.token_limit_reached {
                    return Ok(false);
                }
                AgentCompactionReason::ContextLimit
            }
        };
        self.run_automatic_compaction(turn_identity, phase, reason, step, event_sink)
            .await
    }

    pub(in crate::service) async fn compact_after_provider_context_error(
        &self,
        turn_identity: TurnIdentity,
        phase: AgentCompactionPhase,
        step: &SamplingStep,
        event_sink: &dyn AgentEventSink,
    ) -> Result<bool, AgentError> {
        {
            let cell = self.session(turn_identity.session_id)?;
            let guard = cell.lock()?;
            let record = &*guard;
            let _ = exact_turn_identity(record, turn_identity)?;
            if !record.settings.config.auto_compaction_enabled {
                return Ok(false);
            }
            if phase == AgentCompactionPhase::PreTurn
                && record.conversation.history.iter().position(|item| {
                    matches!(
                        &item.content,
                        ConversationItem::TurnStart { turn_id }
                            if *turn_id == turn_identity.turn_id
                    )
                }) == Some(0)
            {
                return Ok(false);
            }
        }
        self.run_automatic_compaction(
            turn_identity,
            phase,
            AgentCompactionReason::ContextLimit,
            step,
            event_sink,
        )
        .await
    }

    pub(super) async fn run_automatic_compaction(
        &self,
        turn_identity: TurnIdentity,
        phase: AgentCompactionPhase,
        reason: AgentCompactionReason,
        step: &SamplingStep,
        event_sink: &dyn AgentEventSink,
    ) -> Result<bool, AgentError> {
        let transport = CompactionTransport {
            model: step.model.clone(),
            tool_plan: step.tool_plan.clone(),
            prompt: step.prompt.clone(),
        };
        let plan = match self
            .begin_compaction(
                turn_identity.session_id,
                AgentCompactionTrigger::Automatic,
                phase,
                reason,
                None,
                Some(turn_identity),
                transport,
            )
            .await
        {
            Ok(plan) => plan,
            Err(AgentError::CompactionUnavailable) => return Ok(false),
            Err(error) => return Err(error),
        };
        self.execute_compaction(plan, event_sink).await?;
        Ok(true)
    }
}
