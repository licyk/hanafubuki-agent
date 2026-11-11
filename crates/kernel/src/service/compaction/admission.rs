use super::*;

/// What a sampling step needs before it samples.
enum CompactionNeed {
    /// Nothing; the estimate of its request when one was made.
    None {
        estimated: Option<u64>,
    },
    Required(AgentCompactionReason),
}

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
        let mut transport = self.prepare_compaction_transport(session_id).await?;
        let mut revision = revision;
        match self
            .adopt_manual_background_compaction(
                session_id,
                generation,
                revision,
                &transport,
                event_sink.as_ref(),
            )
            .await?
        {
            None => {}
            // A turn admitted while this waited took it: the turn compacts
            // further when it must.
            Some(AdoptedCompaction::InstalledElsewhere(adopted)) => return Ok(adopted),
            Some(AdoptedCompaction::Installed(adopted)) => {
                if !self.over_compaction_threshold(session_id)? {
                    return Ok(adopted);
                }
                revision = adopted.revision;
                transport = self.prepare_compaction_transport(session_id).await?;
            }
        }
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

    /// A `/compact` takes the session's background compaction when it would,
    /// waiting for one still running, once the session is checked as the
    /// caller saw it.
    async fn adopt_manual_background_compaction(
        &self,
        session_id: AgentSessionId,
        generation: AgentGeneration,
        revision: AgentRevision,
        transport: &CompactionTransport,
        event_sink: &dyn AgentEventSink,
    ) -> Result<Option<AdoptedCompaction>, AgentError> {
        {
            let cell = self.session(session_id)?;
            let record = cell.lock_after_persistence().await?;
            if record.runtime.background_compaction.state().is_none() {
                return Ok(None);
            }
            validate_session_mutation(&record, generation, revision)?;
        }
        self.adopt_background_compaction(
            session_id,
            BackgroundAdoption::Required(None),
            &transport.tool_plan,
            &transport.prompt,
            event_sink,
        )
        .await
    }

    /// Whether the session's next request, as its last install estimated
    /// it, still reaches the threshold of automatic compaction.
    fn over_compaction_threshold(&self, session_id: AgentSessionId) -> Result<bool, AgentError> {
        let cell = self.session(session_id)?;
        let record = cell.lock()?;
        let config = &record.settings.config;
        Ok(record
            .runtime
            .usage
            .and_then(|usage| usage.context_consumed_tokens)
            .is_some_and(|active| {
                AgentContextTokenStatus::calculate(
                    active,
                    config.context_window_tokens,
                    self.max_output_tokens(&record),
                    config.auto_compaction_enabled,
                    config.auto_compact_threshold_percent,
                )
                .token_limit_reached
            }))
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
    /// A background compaction that is ready is installed first, and a
    /// pre-turn step under the threshold may launch one.
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
        let adopted = self
            .adopt_background_compaction(
                turn_identity.session_id,
                BackgroundAdoption::Ready,
                &step.tool_plan,
                &step.prompt,
                event_sink,
            )
            .await?
            .is_some();
        let reason = match self
            .automatic_compaction_need(turn_identity, phase, step)
            .await?
        {
            CompactionNeed::Required(reason) => reason,
            CompactionNeed::None { estimated } => {
                if phase == AgentCompactionPhase::PreTurn && estimated.is_some() {
                    self.maybe_launch_background_compaction(turn_identity, step, estimated)
                        .await;
                }
                return Ok(adopted);
            }
        };
        Ok(self
            .run_automatic_compaction(turn_identity, phase, reason, step, event_sink)
            .await?
            || adopted)
    }

    /// Whether the request `step` sends needs a compaction first, and why;
    /// when it does not, the estimate of that request if one was made.
    async fn automatic_compaction_need(
        &self,
        turn_identity: TurnIdentity,
        phase: AgentCompactionPhase,
        step: &SamplingStep,
    ) -> Result<CompactionNeed, AgentError> {
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
                return Ok(CompactionNeed::None { estimated: None });
            }
            if phase == AgentCompactionPhase::MidTurn
                && record.runtime.context_window.new_context_requested
            {
                None
            } else if !record.settings.config.auto_compaction_enabled {
                return Ok(CompactionNeed::None { estimated: None });
            } else {
                Some(referenced_images(
                    record.conversation.history.iter().map(|item| &item.content),
                ))
            }
        };
        let Some(attachments) = estimate else {
            return Ok(CompactionNeed::Required(
                AgentCompactionReason::NewContextRequested,
            ));
        };
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
        let (request, compression) = step_model_request_projected(
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
            self.max_output_tokens(record),
            record.settings.config.auto_compaction_enabled,
            record.settings.config.auto_compact_threshold_percent,
        );
        // `hierarchical` owns the window: the model folds it, and a standard
        // compaction runs only once nothing can be folded or clipped any more
        // (its terminal escape). A turn whose projection failed is
        // `standard`'s.
        let limit_reached = if super::super::compression::CompressionMode::of(record)
            == super::super::compression::CompressionMode::Hierarchical
            && compression.failed.is_none()
        {
            compression.terminal_escape
        } else {
            status.token_limit_reached
        };
        Ok(if budget_pressure || limit_reached {
            CompactionNeed::Required(AgentCompactionReason::ContextLimit)
        } else {
            CompactionNeed::None {
                estimated: Some(estimated),
            }
        })
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

    /// Compacts in line, unless a background compaction the session would
    /// take ends the need: a compaction for the context limit waits for one
    /// still running, and compacts in line as well when its install leaves
    /// the next request over the threshold. A new context is always in line.
    pub(super) async fn run_automatic_compaction(
        &self,
        turn_identity: TurnIdentity,
        phase: AgentCompactionPhase,
        reason: AgentCompactionReason,
        step: &SamplingStep,
        event_sink: &dyn AgentEventSink,
    ) -> Result<bool, AgentError> {
        if reason == AgentCompactionReason::ContextLimit
            && self
                .adopt_background_compaction(
                    turn_identity.session_id,
                    BackgroundAdoption::Required(Some(&step.cancellation)),
                    &step.tool_plan,
                    &step.prompt,
                    event_sink,
                )
                .await?
                .is_some()
            && matches!(
                self.automatic_compaction_need(turn_identity, phase, step)
                    .await?,
                CompactionNeed::None { .. }
            )
        {
            return Ok(true);
        }
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
