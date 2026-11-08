//! Settling what the previous process left running in a recovered session.
//! Each repair bumps the session revision once when it changes something, in
//! the order the repairs run.

use super::*;

/// Settles every unfinished operation of a recovered session. Returns whether
/// the record differs from what was persisted and must be committed.
pub(super) fn repair_after_restart(record: &mut SessionRecord) -> Result<bool, AgentError> {
    let reasoning = record
        .runtime
        .turn
        .as_mut()
        .map(|turn| turn.reasoning.recovery_abort().map_err(map_reasoning_error))
        .transpose()?
        .unwrap_or(false);
    let user_input = record.runtime.pending_user_input.take().is_some();
    let permission_reviews = abort_permission_reviews(record);
    if reasoning {
        bump_revision(
            record,
            now_millis(),
            "agent snapshot revision exhausted during reasoning recovery",
        );
    }
    let tool_outputs = interrupt_turn(record)?;
    let compaction = fail_interrupted_compaction(record);
    let activities = interrupt_activities(record);
    let interrupted = record
        .runtime
        .turn
        .as_ref()
        .is_some_and(|turn| turn.phase == AgentTurnPhase::Interrupted);
    Ok(reasoning
        || user_input
        || permission_reviews
        || tool_outputs
        || compaction
        || activities
        || interrupted)
}

fn bump_revision(record: &mut SessionRecord, now: u128, exhausted: &'static str) {
    record.status.updated_at = now;
    record.status.revision = record.status.revision.checked_add(1).expect(exhausted);
}

/// Aborts permission reviews that were still running.
fn abort_permission_reviews(record: &mut SessionRecord) -> bool {
    let reviewing = record
        .conversation
        .history
        .iter()
        .enumerate()
        .filter_map(|(index, item)| {
            matches!(
                item.content,
                ConversationItem::PermissionReview {
                    status: PermissionReviewStatus::Reviewing,
                    ..
                }
            )
            .then_some(index)
        })
        .collect::<Vec<_>>();
    if reviewing.is_empty() {
        return false;
    }
    let recovered_at = now_millis_u64();
    let mut mutations = Vec::with_capacity(reviewing.len());
    let history = mutable_history(record);
    for index in reviewing {
        let item = Arc::make_mut(&mut history[index]);
        if let ConversationItem::PermissionReview {
            status,
            assessment,
            completed_at,
            ..
        } = &mut item.content
        {
            *status = PermissionReviewStatus::Aborted;
            *assessment = None;
            *completed_at = Some(recovered_at);
            item.item_revision = item.item_revision.saturating_add(1);
            mutations.push(AgentHistoryMutation::UpsertItemSnapshot { item: item.clone() });
        }
    }
    record
        .conversation
        .pending_history_mutations
        .extend(mutations);
    bump_revision(
        record,
        u128::from(recovered_at),
        "agent snapshot revision exhausted during permission recovery",
    );
    true
}

/// Answers tool calls left without output and ends a turn that was still
/// running as interrupted, committing the steering it had queued. Returns
/// whether tool outputs were added.
fn interrupt_turn(record: &mut SessionRecord) -> Result<bool, AgentError> {
    let recovery_outputs =
        interrupted_tool_outputs(&history_contents(&record.conversation.history))?;
    append_history(record, &recovery_outputs);
    if !record
        .runtime
        .turn
        .as_ref()
        .is_some_and(|turn| !turn.phase.is_terminal())
    {
        if !recovery_outputs.is_empty() {
            bump_revision(
                record,
                now_millis(),
                "agent snapshot revision exhausted during tool-pair recovery",
            );
        }
        return Ok(!recovery_outputs.is_empty());
    }
    let now = now_millis();
    let turn = record
        .runtime
        .turn
        .as_mut()
        .expect("active turn was checked");
    let recovered_steering = std::mem::take(&mut turn.pending_steering);
    let steering_items = steering_history_items(record, &recovered_steering);
    append_history(record, &steering_items);
    let turn = record
        .runtime
        .turn
        .as_mut()
        .expect("active turn was checked");
    turn.budget_ledger
        .consume_pending_input_items(recovered_steering.len())
        .map_err(map_budget_error)?;
    turn.last_consumed_steering_sequence = recovered_steering.back().map(|input| input.sequence);
    turn.phase = AgentTurnPhase::Interrupted;
    turn.final_answer_message_id = None;
    turn.terminal_reason = Some(AgentTurnTerminalReason::ApplicationRestarted);
    turn.last_error = Some(
        "Agent turn was interrupted by application shutdown or restart; committed tools were not replayed"
            .to_string(),
    );
    turn.updated_at = now;
    turn.recovery = Some(AgentTurnRecovery::InterruptedAtStartup {
        synthetic_tool_outputs: recovery_outputs.len(),
        recovered_steering_inputs: recovered_steering.len(),
    });
    let identity = TurnIdentity {
        session_id: record.session_id,
        turn_id: turn.id,
        generation: turn.generation,
    };
    bump_revision(
        record,
        now,
        "agent snapshot revision exhausted during recovery",
    );
    settle_turn_working_time(record, identity)?;
    Ok(!recovery_outputs.is_empty())
}

/// The history items of queued steering: each input's host context as a
/// world state update when it changed, then the message.
fn steering_history_items(
    record: &SessionRecord,
    steering: &VecDeque<AgentSteeringInput>,
) -> Vec<ConversationItem> {
    let mut world_state = crate::model::effective_world_state(&record.conversation.history);
    let mut items = Vec::with_capacity(steering.len().saturating_mul(2));
    for input in steering {
        if let Some(context) = &input.host_context {
            if let Some(update) = crate::model::replace_world_state_section(
                &mut world_state,
                AgentWorldStateSection::new(context.kind.clone(), context.content.clone()),
            ) {
                items.push(update);
            }
        }
        items.push(ConversationItem::SteeringMessage {
            sequence: input.sequence,
            text: input.text.clone(),
            images: input.images.clone(),
        });
    }
    items
}

/// Fails a compaction that was still running; the original history stays.
fn fail_interrupted_compaction(record: &mut SessionRecord) -> bool {
    let Some(compaction) = record
        .runtime
        .compaction
        .as_mut()
        .filter(|compaction| !compaction.snapshot.status.is_terminal())
    else {
        return false;
    };
    let now = now_millis();
    compaction.snapshot.status = AgentCompactionStatus::Failed;
    compaction.snapshot.last_error = Some(
        "Agent compaction was interrupted by application shutdown or restart; the original history was retained"
            .to_string(),
    );
    compaction.snapshot.updated_at = now_millis_u64();
    bump_revision(
        record,
        now,
        "agent snapshot revision exhausted during recovery",
    );
    true
}

/// Marks activities that were still running as interrupted.
fn interrupt_activities(record: &mut SessionRecord) -> bool {
    let mut activities = record.runtime.activities.persisted_snapshots();
    if activities
        .iter()
        .all(|activity| activity.phase.is_terminal())
    {
        return false;
    }
    let now = now_millis();
    for activity in activities
        .iter_mut()
        .filter(|activity| !activity.phase.is_terminal())
    {
        activity.phase = crate::AgentActivityPhase::Interrupted;
        activity.revision = activity.revision.saturating_add(1);
        activity.updated_at = now;
        activity.finished_at = Some(now);
    }
    record.runtime.activities = AgentActivityTracker::restore(activities);
    bump_revision(
        record,
        now,
        "agent snapshot revision exhausted during activity recovery",
    );
    true
}
