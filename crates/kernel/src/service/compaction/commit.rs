use super::*;

use super::replacement::{compacted_history, CompactedHistory};
use super::status::compaction_usage;

impl AgentService {
    /// Installs a compaction that is ready, on a clone of the session record
    /// that replaces it only once every check passed.
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
            validate_unchanged_since_plan(record, &plan)?;
            let compacted = compacted_history(
                &sample,
                plan.world_state_checkpoint.clone(),
                &plan.current_turn_suffix,
            );
            let mut staged = record.clone();
            let installed = compacted.contents();
            validate_persisted_history(&installed, &staged.settings.config, self.inner.limits)?;
            let previous_tokens = estimate_conversation_items_tokens(&plan.source_items)?;
            let compacted_prefix_tokens = estimate_conversation_items_tokens(&compacted.prefix)?;
            let installed_tokens = estimate_conversation_items_tokens(&installed)?;
            let compaction_snapshot = &exact_compaction(&staged, plan.identity)?.snapshot;
            let context_limit_compaction = compaction_snapshot.trigger
                == AgentCompactionTrigger::Automatic
                && compaction_snapshot.reason == AgentCompactionReason::ContextLimit;
            if context_limit_compaction && compacted_prefix_tokens >= previous_tokens {
                return Err(AgentError::CompactionDidNotReduceContext {
                    before: previous_tokens,
                    after: compacted_prefix_tokens,
                });
            }
            let snapshot = stage_compacted_history(
                &mut staged,
                &plan,
                &sample,
                compacted,
                &installed,
                installed_tokens,
            )?;
            if snapshot.trigger == AgentCompactionTrigger::Automatic {
                charge_turn_budget(&mut staged, plan.identity.session_id, &snapshot)?;
            }
            staged.runtime.usage = next_model_usage(
                &staged,
                sample.reported_usage,
                sample.estimated_input_tokens,
                sample.estimated_output_tokens,
            )?
            .or(staged.runtime.usage);
            let next_context_tokens =
                estimate_next_request(&staged, &plan, context_limit_compaction, self.inner.limits)?;
            if let Some(usage) = staged.runtime.usage.as_mut() {
                usage.context_consumed_tokens = Some(next_context_tokens);
            }
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
}

/// The compare-and-swap: the compaction is ready and nothing was committed
/// to the session since it began, so the history it compacted is the
/// session's history.
fn validate_unchanged_since_plan(
    record: &SessionRecord,
    plan: &CompactionPlan,
) -> Result<(), AgentError> {
    let compaction = exact_compaction(record, plan.identity)?;
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
    Ok(())
}

/// Replaces the staged history with `compacted` and its display summary,
/// opens the target window on it behind a `Compacted` boundary, and returns
/// the completed compaction.
fn stage_compacted_history(
    staged: &mut SessionRecord,
    plan: &CompactionPlan,
    sample: &CompactionSample,
    compacted: CompactedHistory,
    installed: &[ConversationItem],
    installed_tokens: u64,
) -> Result<AgentCompactionSnapshot, AgentError> {
    let mut replacement = allocate_replacement_history(staged, &compacted.prefix);
    replacement.extend(compacted.suffix);
    let display = ConversationItem::CompactionSummary {
        summary: sample.summary.clone().unwrap_or_default(),
        source_revision: plan.source_revision,
        compacted_item_count: plan.source_items.len(),
    };
    let checkpoint_item = append_history_item(staged, display);
    let snapshot = {
        let compaction = exact_compaction_mut(staged, plan.identity)?;
        compaction.snapshot.status = AgentCompactionStatus::Completed;
        compaction.snapshot.summary = sample.summary.clone();
        compaction.snapshot.usage = Some(compaction_usage(sample));
        compaction.snapshot.last_error = None;
        compaction.snapshot.updated_at = now_millis_u64();
        compaction.snapshot.target_window.prefill_input_tokens = Some(installed_tokens);
        compaction.snapshot.clone()
    };
    staged.conversation.history_bytes = installed.iter().fold(0usize, |total, item| {
        total.saturating_add(item.retained_bytes())
    });
    replace_history(staged, replacement.clone());
    staged.runtime.context_window = plan.target_window;
    staged.runtime.context_window.prefill_input_tokens = Some(installed_tokens);
    staged.runtime.context_window.new_context_requested = false;
    let mut window_chain = plan.target_window_chain.clone();
    if let Some(last) = window_chain.last_mut() {
        last.prefill_input_tokens = Some(installed_tokens);
    }
    staged.conversation.model_context_boundary = AgentModelContextBoundary::Compacted {
        checkpoint: crate::AgentCompactionCheckpoint {
            checkpoint_item_id: checkpoint_item.item_id,
            snapshot: snapshot.clone(),
            replacement_history: replacement,
            window_chain,
            future_items_after_item_id: checkpoint_item.item_id.value(),
            initial_context_digest: sample.initial_context_digest,
        },
    };
    staged.conversation.pending_history_mutations.push(
        AgentHistoryMutation::SetModelContextBoundary {
            boundary: staged.conversation.model_context_boundary.clone(),
        },
    );
    Ok(snapshot)
}

fn allocate_replacement_history(
    record: &mut SessionRecord,
    replacement: &[ConversationItem],
) -> Vec<AgentHistoryItem> {
    replacement
        .iter()
        .map(|content| {
            let item_id = AgentHistoryItemId::new(record.conversation.next_history_item_id);
            record.conversation.next_history_item_id = record
                .conversation
                .next_history_item_id
                .checked_add(1)
                .expect("Agent history item id exhausted");
            AgentHistoryItem {
                source_session_id: record.session_id,
                item_id,
                item_revision: 0,
                turn_id: None,
                logical_segment: 0,
                logical_order: item_id.value(),
                turn_working_millis: None,
                content: content.clone(),
            }
        })
        .collect()
}

/// Counts an automatic compaction against its turn's budget, whose model
/// usage starts again in the new window.
fn charge_turn_budget(
    staged: &mut SessionRecord,
    session_id: AgentSessionId,
    snapshot: &AgentCompactionSnapshot,
) -> Result<(), AgentError> {
    let identity = TurnIdentity {
        session_id,
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
    let ledger = &mut exact_turn_mut(staged, identity)?.budget_ledger;
    ledger.record_compaction().map_err(map_budget_error)?;
    ledger.reset_cumulative_model_usage();
    Ok(())
}

/// The estimate of the request that follows the install. A compaction for
/// the context limit fails when that request would still reach the
/// threshold.
fn estimate_next_request(
    staged: &SessionRecord,
    plan: &CompactionPlan,
    context_limit_compaction: bool,
    limits: AgentHardLimits,
) -> Result<u64, AgentError> {
    let next_request = step_model_request(
        staged,
        &plan.images,
        &plan.next_tool_plan,
        &plan.next_prompt,
        limits.maximum_tool_output_bytes,
    )?;
    let next_context_tokens = estimate_model_request_tokens(&next_request)?;
    let config = &staged.settings.config;
    if context_limit_compaction
        && AgentContextTokenStatus::calculate(
            next_context_tokens,
            config.context_window_tokens,
            config.auto_compaction_enabled,
            config.auto_compact_threshold_percent,
        )
        .token_limit_reached
    {
        return Err(AgentError::ContextBudgetExceeded {
            consumed: next_context_tokens,
            limit: config
                .context_window_tokens
                .map(|limit| {
                    ((u128::from(limit) * u128::from(config.auto_compact_threshold_percent)) / 100)
                        as u64
                })
                .unwrap_or(next_context_tokens),
            source: AgentUsageSource::Estimated,
        });
    }
    Ok(next_context_tokens)
}
