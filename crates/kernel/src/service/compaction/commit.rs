use super::*;

impl AgentService {
    pub(super) async fn commit_compaction(
        &self,
        plan: CompactionPlan,
        sample: CompactionSample,
    ) -> Result<(AgentCompactionOutcome, AgentEvent), AgentError> {
        let cell = self.session(plan.identity.session_id)?;
        let (persisted, outcome, event) = {
            let mut guard = cell.lock_after_persistence().await?;
            let record = &mut *guard;
            let previous = record.rollback_state();
            {
                let compaction = exact_compaction_mut(record, plan.identity)?;
                if compaction.snapshot.status != AgentCompactionStatus::ReadyToInstall
                    || compaction.snapshot.source_revision != plan.source_revision
                    || compaction.snapshot.source_window_id != plan.source_window.window_id
                    || compaction.snapshot.target_window != plan.target_window
                    || compaction.operation_revision != record.status.revision
                    || record
                        .conversation
                        .history
                        .iter()
                        .map(|item| item.item_id)
                        .ne(plan.source_item_ids.iter().copied())
                {
                    return Err(AgentError::StaleRevision);
                }
            }
            let mut staged = record.clone();
            let mut replacement_history = sample.replacement_history.clone();
            if let Some(checkpoint) =
                crate::model::full_world_state_checkpoint(&record.conversation.history)
            {
                replacement_history.insert(0, checkpoint);
            }
            let mut installed_history = replacement_history.clone();
            installed_history.extend(
                plan.current_turn_suffix
                    .iter()
                    .map(|item| item.content.clone()),
            );
            validate_persisted_history(
                &installed_history,
                &staged.settings.config,
                self.inner.limits,
            )?;
            let previous_tokens = estimate_conversation_items_tokens(&plan.source_items)?;
            let compacted_prefix_tokens = estimate_conversation_items_tokens(&replacement_history)?;
            let replacement_tokens = estimate_conversation_items_tokens(&installed_history)?;
            let compaction_snapshot = &exact_compaction(&staged, plan.identity)?.snapshot;
            let automatic = compaction_snapshot.trigger == AgentCompactionTrigger::Automatic;
            let context_limit_compaction =
                automatic && compaction_snapshot.reason == AgentCompactionReason::ContextLimit;
            if context_limit_compaction && compacted_prefix_tokens >= previous_tokens {
                return Err(AgentError::CompactionDidNotReduceContext {
                    before: previous_tokens,
                    after: compacted_prefix_tokens,
                });
            }

            let mut replacement = allocate_replacement_history(&mut staged, &replacement_history);
            replacement.extend(plan.current_turn_suffix.iter().cloned());
            let display = ConversationItem::CompactionSummary {
                summary: sample.summary.clone().unwrap_or_default(),
                source_revision: plan.source_revision,
                compacted_item_count: plan.source_items.len(),
            };
            let checkpoint_item = append_history_item(&mut staged, display);
            let cutoff = checkpoint_item.item_id.value();
            let now = now_millis();
            let compaction_now = now_millis_u64();
            let snapshot = {
                let compaction = exact_compaction_mut(&mut staged, plan.identity)?;
                compaction.snapshot.status = AgentCompactionStatus::Completed;
                compaction.snapshot.summary = sample.summary.clone();
                compaction.snapshot.usage = Some(compaction_usage(&sample));
                compaction.snapshot.last_error = None;
                compaction.snapshot.updated_at = compaction_now;
                compaction.snapshot.target_window.prefill_input_tokens = Some(replacement_tokens);
                compaction.snapshot.clone()
            };
            staged.conversation.history_bytes =
                installed_history.iter().fold(0usize, |total, item| {
                    total.saturating_add(item.retained_bytes())
                });
            replace_history(&mut staged, replacement.clone());
            staged.runtime.context_window = plan.target_window;
            staged.runtime.context_window.prefill_input_tokens = Some(replacement_tokens);
            staged.runtime.context_window.new_context_requested = false;
            staged.conversation.model_context_boundary = AgentModelContextBoundary::Compacted {
                checkpoint: crate::AgentCompactionCheckpoint {
                    checkpoint_item_id: checkpoint_item.item_id,
                    snapshot: snapshot.clone(),
                    replacement_history: replacement,
                    window_chain: {
                        let mut window_chain = plan.target_window_chain;
                        if let Some(last) = window_chain.last_mut() {
                            last.prefill_input_tokens = Some(replacement_tokens);
                        }
                        window_chain
                    },
                    future_items_after_item_id: cutoff,
                    initial_context_digest: sample.initial_context_digest,
                },
            };
            staged.conversation.pending_history_mutations.push(
                AgentHistoryMutation::SetModelContextBoundary {
                    boundary: staged.conversation.model_context_boundary.clone(),
                },
            );
            if snapshot.trigger == AgentCompactionTrigger::Automatic {
                let identity = TurnIdentity {
                    session_id: plan.identity.session_id,
                    turn_id: staged
                        .runtime
                        .turn
                        .as_ref()
                        .ok_or(AgentError::TurnNotRunning)?
                        .id,
                    generation: snapshot
                        .turn_generation
                        .ok_or(AgentError::StaleGeneration)?,
                };
                let ledger = &mut exact_turn_mut(&mut staged, identity)?.budget_ledger;
                ledger.record_compaction().map_err(map_budget_error)?;
                ledger.reset_cumulative_model_usage();
            }
            staged.runtime.usage = next_model_usage(
                &staged,
                sample.reported_usage,
                sample.estimated_input_tokens,
                sample.estimated_output_tokens,
            )?
            .or(staged.runtime.usage);
            let next_request = step_model_request(
                &staged,
                &plan.images,
                &plan.next_tool_plan,
                &plan.next_prompt,
                self.inner.limits.maximum_tool_output_bytes,
            )?;
            let next_context_tokens = estimate_model_request_tokens(&next_request)?;
            if context_limit_compaction
                && AgentContextTokenStatus::calculate(
                    next_context_tokens,
                    staged.settings.config.context_window_tokens,
                    staged.settings.config.auto_compaction_enabled,
                    staged.settings.config.auto_compact_threshold_percent,
                )
                .token_limit_reached
            {
                return Err(AgentError::ContextBudgetExceeded {
                    consumed: next_context_tokens,
                    limit: staged
                        .settings
                        .config
                        .context_window_tokens
                        .map(|limit| {
                            ((u128::from(limit)
                                * u128::from(
                                    staged.settings.config.auto_compact_threshold_percent,
                                ))
                                / 100) as u64
                        })
                        .unwrap_or(next_context_tokens),
                    source: AgentUsageSource::Estimated,
                });
            }
            if let Some(usage) = staged.runtime.usage.as_mut() {
                usage.context_consumed_tokens = Some(next_context_tokens);
            }
            staged.status.updated_at = now;
            let event = record_compaction_event(&mut staged, snapshot.clone(), self.inner.limits);
            staged
                .runtime
                .compaction
                .as_mut()
                .expect("checked")
                .operation_revision = staged.status.revision;
            let persisted = StagedPersistence::commit(&cell, &mut staged, previous);
            let outcome = AgentCompactionOutcome {
                session_id: plan.identity.session_id,
                revision: AgentRevision::new(staged.status.revision),
                compaction: snapshot,
            };
            *record = staged;
            (persisted, outcome, event)
        };
        self.persist_staged(persisted).await?;
        cell.notify_terminal();
        Ok((outcome, event))
    }

    pub(super) async fn set_compaction_status(
        &self,
        identity: CompactionIdentity,
        status: AgentCompactionStatus,
        summary: Option<String>,
        usage: Option<AgentCompactionUsage>,
    ) -> Result<(), AgentError> {
        let cell = self.session(identity.session_id)?;
        let staged = {
            let mut guard = cell.lock_after_persistence().await?;
            let record = &mut *guard;
            let previous = record.rollback_state();
            let compaction = exact_compaction_mut(record, identity)?;
            compaction.snapshot.status = status;
            compaction.snapshot.summary = summary;
            compaction.snapshot.usage = usage;
            compaction.snapshot.updated_at = now_millis_u64();
            let snapshot = record
                .runtime
                .compaction
                .as_ref()
                .expect("checked")
                .snapshot
                .clone();
            record_compaction_event(record, snapshot, self.inner.limits);
            record
                .runtime
                .compaction
                .as_mut()
                .expect("checked")
                .operation_revision = record.status.revision;
            StagedPersistence::commit(&cell, record, previous)
        };
        self.persist_staged(staged).await?;
        Ok(())
    }

    pub(super) fn check_compaction_cancelled(
        &self,
        identity: CompactionIdentity,
    ) -> Result<(), AgentError> {
        let cell = self.session(identity.session_id)?;
        let record = cell.lock()?;
        if exact_compaction(&record, identity)?
            .cancellation
            .is_cancelled()
        {
            Err(AgentError::Cancelled)
        } else {
            Ok(())
        }
    }

    /// Commits the compaction's end and returns its event; nothing when the
    /// session or the compaction is gone or the compaction already ended.
    pub(super) async fn finish_compaction_terminal(
        &self,
        identity: CompactionIdentity,
        status: AgentCompactionStatus,
        error: Option<String>,
    ) -> Result<Option<AgentEvent>, AgentError> {
        let Some(cell) = self.inner.sessions.find(identity.session_id) else {
            return Ok(None);
        };
        let terminal = {
            let Ok(mut record) = cell.lock_after_persistence().await else {
                return Ok(None);
            };
            self.stage_compaction_terminal(&cell, &mut record, identity, status, error)?
        };
        let Some((staged, event)) = terminal else {
            return Ok(None);
        };
        self.persist_staged(staged).await?;
        cell.notify_terminal();
        Ok(Some(event))
    }

    /// `finish_compaction_terminal` for the drop guard, which can neither
    /// await nor wait: the terminal state is staged now, or once the commit
    /// in flight settles, and handed to the history store; its
    /// acknowledgement publishes it and wakes the terminal waiters.
    pub(super) fn finish_compaction_terminal_on_drop(
        &self,
        identity: CompactionIdentity,
        status: AgentCompactionStatus,
    ) {
        let Some(cell) = self.inner.sessions.find(identity.session_id) else {
            return;
        };
        let service = self.clone();
        let follow_up = cell.stage_or_defer(Box::new(move |cell, record| {
            let (staged, _) = service
                .stage_compaction_terminal(cell, record, identity, status, None)
                .unwrap_or_else(|error| {
                    log::warn!(
                        "agent_diag stage=compaction.drop_terminal session_id={} outcome=error error={}",
                        identity.session_id,
                        bounded_diagnostic(error.to_string())
                    );
                    None
                })?;
            let cell = cell.clone();
            Some(staged.into_follow_up(move || cell.notify_terminal()))
        }));
        self.hand_off(follow_up);
    }

    fn stage_compaction_terminal(
        &self,
        cell: &Arc<SessionCell>,
        record: &mut SessionRecord,
        identity: CompactionIdentity,
        status: AgentCompactionStatus,
        error: Option<String>,
    ) -> Result<Option<(StagedPersistence, AgentEvent)>, AgentError> {
        let Ok(compaction) = exact_compaction_mut(record, identity) else {
            return Ok(None);
        };
        if compaction.snapshot.status.is_terminal() {
            return Ok(None);
        }
        let previous = record.rollback_state();
        let compaction = exact_compaction_mut(record, identity)?;
        compaction.snapshot.status = status;
        compaction.snapshot.last_error = error.map(bounded_diagnostic);
        compaction.snapshot.updated_at = now_millis_u64();
        let snapshot = record
            .runtime
            .compaction
            .as_ref()
            .expect("checked")
            .snapshot
            .clone();
        let event = record_compaction_event(record, snapshot, self.inner.limits);
        record
            .runtime
            .compaction
            .as_mut()
            .expect("checked")
            .operation_revision = record.status.revision;
        Ok(Some((
            StagedPersistence::commit(cell, record, previous),
            event,
        )))
    }
}
