use super::*;

use super::replacement::{compacted_history, CompactedHistory};
use super::status::compaction_usage;

/// How an install proves that the history it compacted is still the
/// session's.
pub(super) enum InstallMode<'a> {
    /// Nothing was committed to the session since the compaction began.
    Cas,
    /// The session went on while it ran (`background`): the window is still
    /// the one it compacted, the history still begins with the one it read,
    /// and what was added since stays after the replacement.
    Rebase(RebaseInstall<'a>),
}

/// What a rebasing install records and estimates the next request with,
/// none of which the compaction knew when it began.
pub(super) struct RebaseInstall<'a> {
    pub(super) turn_generation: AgentGeneration,
    pub(super) images: &'a LoadedImages,
    pub(super) tool_plan: &'a AgentToolPlan,
    pub(super) prompt: &'a RequestContext,
}

impl AgentService {
    /// Installs a compaction that is ready, on a clone of the session record
    /// that replaces it only once every check passed.
    pub(super) async fn commit_compaction(
        &self,
        plan: &CompactionPlan,
        sample: &CompactionSample,
        mode: InstallMode<'_>,
    ) -> Result<(AgentCompactionOutcome, AgentEvent), AgentError> {
        let cell = self.session(plan.identity.session_id)?;
        let (persisted, outcome, event) = {
            let mut guard = cell.lock_after_persistence().await?;
            let record = &mut *guard;
            let previous = record.rollback_state();
            let compacted = match &mode {
                InstallMode::Cas => {
                    validate_unchanged_since_plan(record, plan)?;
                    compacted_history(
                        sample,
                        plan.world_state_checkpoint.clone(),
                        &plan.current_turn_suffix,
                    )
                }
                InstallMode::Rebase(_) => compacted_history(
                    sample,
                    plan.world_state_checkpoint.clone(),
                    rebased_suffix(record, plan)?,
                ),
            };
            let mut staged = record.clone();
            let identity = match &mode {
                InstallMode::Cas => plan.identity,
                InstallMode::Rebase(rebase) => {
                    reserve_background_install(&mut staged, plan, rebase.turn_generation)
                }
            };
            let installed = compacted.contents();
            validate_persisted_history(&installed, &staged.settings.config, self.inner.limits)?;
            let previous_tokens = estimate_conversation_items_tokens(&plan.source_items)?;
            let compacted_prefix_tokens = estimate_conversation_items_tokens(&compacted.prefix)?;
            let installed_tokens = estimate_conversation_items_tokens(&installed)?;
            let compaction_snapshot = &exact_compaction(&staged, identity)?.snapshot;
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
                identity,
                plan,
                sample,
                compacted,
                &installed,
                installed_tokens,
            )?;
            // A background compaction counts toward no turn's budget, and its
            // usage was recorded when it finished (`background`).
            let next_context_tokens = match &mode {
                InstallMode::Cas => {
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
                    estimate_next_request(
                        &staged,
                        NextRequest {
                            images: &plan.images,
                            tool_plan: &plan.next_tool_plan,
                            prompt: &plan.next_prompt,
                        },
                        context_limit_compaction,
                        self.inner.limits,
                    )?
                }
                // Still over the threshold is not a failure: the turn then
                // compacts in line as well.
                InstallMode::Rebase(rebase) => estimate_next_request(
                    &staged,
                    NextRequest {
                        images: rebase.images,
                        tool_plan: rebase.tool_plan,
                        prompt: rebase.prompt,
                    },
                    false,
                    self.inner.limits,
                )?,
            };
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

/// The rebase's proof that the history the compaction read is a prefix of
/// the session's, and the history added after the part it compacted. The
/// session keeps no other history in a window (a compaction opens a new one,
/// editing forks), so the window and the prefix of item ids prove it.
fn rebased_suffix<'a>(
    record: &'a SessionRecord,
    plan: &CompactionPlan,
) -> Result<&'a [Arc<AgentHistoryItem>], AgentError> {
    let history = &record.conversation.history;
    if record
        .runtime
        .compaction
        .as_ref()
        .is_some_and(|value| !value.snapshot.status.is_terminal())
    {
        return Err(AgentError::CompactionAlreadyRunning);
    }
    if record.runtime.context_window.window_id != plan.source_window.window_id
        || history.len() < plan.source_item_ids.len()
        || history
            .iter()
            .zip(&plan.source_item_ids)
            .any(|(item, id)| item.item_id != *id)
    {
        return Err(AgentError::StaleRevision);
    }
    Ok(&history[plan.source_boundary..])
}

/// Records a background compaction as the session's, ready to install: it
/// had no reserved or running record, since nothing of it was persisted.
fn reserve_background_install(
    staged: &mut SessionRecord,
    plan: &CompactionPlan,
    turn_generation: AgentGeneration,
) -> CompactionIdentity {
    let generation = AgentCompactionGeneration::new(
        staged
            .runtime
            .compaction
            .as_ref()
            .map_or(0, |value| value.snapshot.generation.value())
            .checked_add(1)
            .expect("agent compaction generation exhausted"),
    );
    let now = now_millis_u64();
    staged.runtime.compaction = Some(CompactionRecord {
        snapshot: AgentCompactionSnapshot {
            id: plan.identity.id,
            generation,
            trigger: AgentCompactionTrigger::Automatic,
            phase: AgentCompactionPhase::Background,
            reason: AgentCompactionReason::ContextLimit,
            implementation: plan.implementation,
            status: AgentCompactionStatus::ReadyToInstall,
            source_revision: plan.source_revision,
            source_window_id: plan.source_window.window_id,
            target_window: plan.target_window,
            turn_generation: Some(turn_generation),
            summary: None,
            usage: None,
            last_error: None,
            created_at: now,
            updated_at: now,
        },
        cancellation: CancellationToken::default(),
        operation_revision: staged.status.revision,
    });
    CompactionIdentity {
        session_id: plan.identity.session_id,
        id: plan.identity.id,
        generation,
    }
}

/// Replaces the staged history with `compacted` and its display summary,
/// opens the target window on it behind a `Compacted` boundary, and returns
/// the completed compaction.
fn stage_compacted_history(
    staged: &mut SessionRecord,
    identity: CompactionIdentity,
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
        let compaction = exact_compaction_mut(staged, identity)?;
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
    super::super::compression::after_standard_compaction(staged);
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

/// What the request that follows an install is built with.
struct NextRequest<'a> {
    images: &'a LoadedImages,
    tool_plan: &'a AgentToolPlan,
    prompt: &'a RequestContext,
}

/// The estimate of the request that follows the install. A compaction for
/// the context limit fails when that request would still reach the
/// threshold.
fn estimate_next_request(
    staged: &SessionRecord,
    next: NextRequest<'_>,
    context_limit_compaction: bool,
    limits: AgentHardLimits,
) -> Result<u64, AgentError> {
    let next_request = step_model_request(
        staged,
        next.images,
        next.tool_plan,
        next.prompt,
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
