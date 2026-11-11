use super::*;

use crate::service::compaction::source::compaction_source;

impl AgentService {
    /// Launches a compaction of the completed turns in the background when
    /// the session's strategy is `background` and its context is in the band
    /// from the background threshold up to the foreground one. `estimated`
    /// is the step's estimate when the caller made one; the provider's last
    /// count is taken with it. Never fails the turn: what cannot launch does
    /// not, and the turn compacts in line when it must.
    pub(in crate::service) async fn maybe_launch_background_compaction(
        &self,
        identity: TurnIdentity,
        step: &SamplingStep,
        estimated: Option<u64>,
    ) {
        if let Err(error) = self
            .launch_background_compaction(identity, step, estimated)
            .await
        {
            log::debug!(
                "agent_diag stage=compaction.background_launch session_id={} outcome=skipped error={}",
                identity.session_id,
                bounded_diagnostic(error.to_string())
            );
        }
    }

    async fn launch_background_compaction(
        &self,
        identity: TurnIdentity,
        step: &SamplingStep,
        estimated: Option<u64>,
    ) -> Result<(), AgentError> {
        let cell = self.session(identity.session_id)?;
        let launchable = |record: &SessionRecord| {
            launch_attempt(
                record,
                identity,
                estimated,
                self.inner.accepting.load(Ordering::Acquire),
                self.max_output_tokens(record),
            )
        };
        // Checked again under the lock below; this spares a session that
        // will not launch the load of its images.
        if launchable(&*cell.lock()?)?.is_none() {
            return Ok(());
        }
        let images = self.load_history_images(identity.session_id).await?;
        let transport = CompactionTransport {
            model: step.model.clone(),
            tool_plan: step.tool_plan.clone(),
            prompt: step.prompt.clone(),
        };
        let (plan, run) = {
            let mut guard = cell.lock_after_persistence().await?;
            let record = &mut *guard;
            let Some(attempt) = launchable(record)? else {
                return Ok(());
            };
            self.refresh_image_input(record);
            let reasoning = self.request_reasoning(record);
            let source = compaction_source(
                record,
                AgentCompactionPhase::Background,
                Some(identity),
                &images,
                &transport,
                reasoning,
            )?;
            let run = Arc::new(BackgroundRun {
                attempt: attempt.clone(),
                turn_generation: identity.generation,
                cancellation: CancellationToken::default(),
                outcome: Mutex::new(BackgroundOutcome::Running),
            });
            let plan = source.into_plan(
                CompactionIdentity {
                    session_id: identity.session_id,
                    id: self.allocate_compaction_id(),
                    generation: AgentCompactionGeneration::new(0),
                },
                AgentCompactionPhase::Background,
                run.cancellation.clone(),
                transport,
                images,
            );
            let slot = &mut record.runtime.background_compaction;
            slot.cancel();
            slot.last_attempt = Some(attempt);
            slot.run = Some(run.clone());
            // Registered before the lock is released: a shutdown that cancels
            // the slot after this waits for the task.
            self.inner
                .managed_background
                .lock()
                .insert((identity.session_id, plan.identity.id));
            (plan, run)
        };
        let guard = BackgroundTaskGuard {
            service: self.clone(),
            session_id: identity.session_id,
            id: plan.identity.id,
            run: run.clone(),
        };
        let service = self.clone();
        tokio::spawn(async move {
            let _guard = guard;
            let result = hanafubuki_agent_foundation::panic::catch_panic_async(
                service.execute_compaction_inner(&plan),
            )
            .await;
            service
                .finish_background_compaction(plan, &run, result)
                .await;
        });
        Ok(())
    }

    /// Keeps the run's result for the next checkpoint, and records its usage
    /// in the session's now, adopted or not: it counts toward no turn.
    async fn finish_background_compaction(
        &self,
        plan: CompactionPlan,
        run: &BackgroundRun,
        result: Result<Result<CompactionSample, AgentError>, String>,
    ) {
        let session_id = plan.identity.session_id;
        let outcome = match result {
            Ok(Ok(sample)) => {
                if let Err(error) = self.record_background_usage(session_id, &sample).await {
                    log::debug!(
                        "agent_diag stage=compaction.background_usage session_id={session_id} outcome=error error={}",
                        bounded_diagnostic(error.to_string())
                    );
                }
                BackgroundOutcome::Ready(Box::new((plan, sample)))
            }
            Ok(Err(error)) => {
                log::debug!(
                    "agent_diag stage=compaction.background session_id={session_id} outcome=discarded error={}",
                    bounded_diagnostic(error.to_string())
                );
                BackgroundOutcome::Spent
            }
            Err(message) => {
                log::error!("background compaction panicked session_id={session_id}: {message}");
                self.inner
                    .kernel_faults
                    .kernel_fault("background compaction", &message);
                BackgroundOutcome::Spent
            }
        };
        *run.outcome.lock() = outcome;
        if let Some(cell) = self.inner.sessions.find(session_id) {
            cell.notify_terminal();
        }
    }

    /// Adds a background compaction's usage to the session's, persisted but
    /// not a revision: it changes nothing a caller checks a revision for.
    async fn record_background_usage(
        &self,
        session_id: AgentSessionId,
        sample: &CompactionSample,
    ) -> Result<(), AgentError> {
        let cell = self.session(session_id)?;
        let staged = {
            let mut guard = cell.lock_after_persistence().await?;
            let record = &mut *guard;
            if record.status.disposing || record.status.lifecycle == AgentSessionLifecycle::Archived
            {
                return Ok(());
            }
            let Some(mut usage) = next_model_usage(
                record,
                sample.reported_usage,
                sample.estimated_input_tokens,
                sample.estimated_output_tokens,
            )?
            else {
                return Ok(());
            };
            // It measured the compacted history, not the session's context.
            usage.context_consumed_tokens = record
                .runtime
                .usage
                .and_then(|value| value.context_consumed_tokens);
            let previous = record.rollback_state();
            record.runtime.usage = Some(usage);
            StagedPersistence::commit(&cell, record, previous)
        };
        self.persist_staged(staged).await
    }
}

/// The attempt to launch for the turn `identity` now, if any: the session's
/// strategy is `background` with automatic compaction on, the turn runs, is
/// not cancelled and is under no budget pressure, its context is in the
/// background band, a turn completed before it, and the session has no
/// usable result or run already and did not try these inputs.
fn launch_attempt(
    record: &SessionRecord,
    identity: TurnIdentity,
    estimated: Option<u64>,
    accepting: bool,
    max_output_tokens: Option<u64>,
) -> Result<Option<BackgroundAttempt>, AgentError> {
    let config = &record.settings.config;
    if !accepting
        || config.compaction_strategy != crate::AgentCompactionStrategy::Background
        || !config.auto_compaction_enabled
        || record.runtime.context_window.new_context_requested
        || record
            .runtime
            .compaction
            .as_ref()
            .is_some_and(|value| !value.snapshot.status.is_terminal())
    {
        return Ok(None);
    }
    let turn = exact_turn_identity(record, identity)?;
    if turn.phase.is_terminal()
        || turn.cancellation.is_cancelled()
        || turn.budget_ledger.proactive_model_usage_limit_reached(
            record.settings.budget_policy,
            config.auto_compact_threshold_percent,
        )
    {
        return Ok(None);
    }
    let Some(threshold) = turn
        .extensions
        .get::<crate::AgentBackgroundCompactionThreshold>()
    else {
        return Ok(None);
    };
    let reported = record
        .runtime
        .usage
        .and_then(|usage| usage.context_consumed_tokens);
    let Some(active) = estimated.max(reported) else {
        return Ok(None);
    };
    if !in_background_band(active, threshold.percent, config, max_output_tokens) {
        return Ok(None);
    }
    let history = &record.conversation.history;
    let Some(current) = history.iter().position(|item| {
        matches!(&item.content, ConversationItem::TurnStart { turn_id } if *turn_id == identity.turn_id)
    }) else {
        return Ok(None);
    };
    if !history[..current]
        .iter()
        .any(|item| matches!(item.content, ConversationItem::TurnStart { .. }))
    {
        return Ok(None);
    }
    let attempt = BackgroundAttempt {
        window_id: record.runtime.context_window.window_id,
        compacted_items: current,
        model: config.model.clone(),
        implementation: AgentCompactionImplementation::from(config.compaction_capability),
    };
    let slot = &record.runtime.background_compaction;
    let usable = slot
        .run
        .as_ref()
        .is_some_and(|run| run.attempt.matches(record) && run.outcome.lock().state().is_some());
    if usable || slot.last_attempt.as_ref() == Some(&attempt) {
        return Ok(None);
    }
    Ok(Some(attempt))
}

/// At or over the background threshold and under the foreground one.
fn in_background_band(
    active: u64,
    threshold_percent: u8,
    config: &AgentSessionConfig,
    max_output_tokens: Option<u64>,
) -> bool {
    let Some(window) = config.context_window_tokens else {
        return false;
    };
    let background_limit = (u128::from(window) * u128::from(threshold_percent)) / 100;
    u128::from(active) >= background_limit
        && !AgentContextTokenStatus::calculate(
            active,
            config.context_window_tokens,
            max_output_tokens,
            config.auto_compaction_enabled,
            config.auto_compact_threshold_percent,
        )
        .token_limit_reached
}
