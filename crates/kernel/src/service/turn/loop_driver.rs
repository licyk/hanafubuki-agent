use super::*;

impl AgentService {
    pub(super) async fn run_loop(
        &self,
        identity: TurnIdentity,
        event_sink: &dyn AgentEventSink,
    ) -> Result<(AgentMessageId, String), AgentError> {
        let mut compaction_phase = AgentCompactionPhase::PreTurn;
        // A provider's context-window error is answered with one compaction
        // and a retry; a second one before a response succeeds ends the turn.
        let mut compacted_after_context_error = false;
        loop {
            self.check_cancelled(identity)?;
            let step = self.prepare_sampling_step(identity).await?;
            let sampling_phase = if self
                .maybe_auto_compact(identity, compaction_phase, &step, event_sink)
                .await?
            {
                AgentCompactionPhase::MidTurn
            } else {
                compaction_phase
            };
            compaction_phase = AgentCompactionPhase::MidTurn;
            self.check_cancelled(identity)?;
            let request = self.begin_sampling(identity, &step, event_sink).await?;
            let sampled = match self
                .run_sampling_request(
                    identity,
                    step.model.clone(),
                    request,
                    step.tool_plan.clone(),
                    step.memory.clone(),
                    step.cancellation.clone(),
                    event_sink,
                )
                .await
            {
                Ok(sampled) => sampled,
                Err(AgentError::ProviderContextWindowExceeded { message, .. })
                    if compacted_after_context_error =>
                {
                    return Err(AgentError::ContextWindowExceededAfterCompaction { message });
                }
                Err(error @ AgentError::ProviderContextWindowExceeded { .. }) => {
                    if self
                        .compact_after_provider_context_error(
                            identity,
                            sampling_phase,
                            &step,
                            event_sink,
                        )
                        .await?
                    {
                        compacted_after_context_error = true;
                        continue;
                    }
                    return Err(error);
                }
                Err(error) => return Err(error),
            };
            compacted_after_context_error = false;
            self.check_cancelled(identity)?;
            let assistant_messages = sampled
                .items
                .iter()
                .filter_map(|item| match item {
                    AgentSamplingItem::Assistant { phase, text, .. } => {
                        Some((*phase, text.clone()))
                    }
                    AgentSamplingItem::ToolCall(call) => {
                        let _ = call;
                        None
                    }
                })
                .collect::<Vec<_>>();
            let progress_tool_calls = sampled.tool_calls.clone();
            let mut needs_follow_up = sampled.result.needs_follow_up;
            let mut reasons = std::collections::BTreeSet::new();
            if !progress_tool_calls.is_empty() {
                reasons.insert(AgentFollowUpReason::ToolCalls);
            }
            if sampled.provider_end_turn == Some(false) {
                reasons.insert(AgentFollowUpReason::ProviderContinuation);
            }
            if self.consume_pending_steering(identity, event_sink).await? {
                needs_follow_up = true;
                reasons.insert(AgentFollowUpReason::PendingUserInput);
            }

            if needs_follow_up {
                let digest =
                    sampling_progress_digest(&assistant_messages, &progress_tool_calls, &reasons);
                self.observe_follow_up_progress(identity, digest).await?;
                continue;
            }

            let final_answer_message_id =
                sampled.result.last_agent_message_id.ok_or_else(|| {
                    AgentError::Protocol(
                        "model completed a turn without an assistant message".to_string(),
                    )
                })?;
            let final_answer = sampled
                .items
                .iter()
                .rev()
                .find_map(|item| match item {
                    AgentSamplingItem::Assistant {
                        message_id, text, ..
                    } if *message_id == final_answer_message_id => Some(text.clone()),
                    _ => None,
                })
                .ok_or_else(|| {
                    AgentError::Protocol(
                        "terminal assistant message was not present in sampling output".to_string(),
                    )
                })?;
            let continuation_items = self
                .run_continuation_hooks(
                    identity,
                    final_answer.clone(),
                    step.hook_catalog,
                    step.cancellation,
                    event_sink,
                )
                .await?;
            if continuation_items.is_empty() {
                return Ok((final_answer_message_id, final_answer));
            }
            let progress = serde_json::to_string(&continuation_items)
                .map_err(|error| AgentError::Protocol(error.to_string()))?;
            let digest = sampling_progress_digest(
                &[(Some(AgentMessagePhase::FinalAnswer), progress)],
                &[],
                &std::collections::BTreeSet::new(),
            );
            self.commit_history(identity, &continuation_items, event_sink)
                .await?;
            self.observe_follow_up_progress(identity, digest).await?;
        }
    }

    pub(super) async fn consume_pending_steering(
        &self,
        identity: TurnIdentity,
        event_sink: &dyn AgentEventSink,
    ) -> Result<bool, AgentError> {
        let (staged, history_event, consumed_event) = {
            let cell = self.session(identity.session_id)?;
            let mut guard = cell.lock_after_persistence().await?;
            let record = &mut *guard;
            let previous = record.rollback_state();
            let pending = {
                let turn = exact_turn_mut(record, identity)?;
                if turn.pending_steering.is_empty() {
                    return Ok(false);
                }
                turn.pending_steering.iter().cloned().collect::<Vec<_>>()
            };
            let mut world_state = crate::model::effective_world_state(&record.conversation.history);
            let mut items = Vec::with_capacity(pending.len().saturating_mul(2));
            for input in &pending {
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
            validate_history_append(record, &items, self.inner.limits)?;
            let turn = exact_turn_mut(record, identity)?;
            let mut ledger = turn.budget_ledger;
            ledger
                .consume_pending_input_items(pending.len())
                .map_err(map_budget_error)?;
            turn.budget_ledger = ledger;
            turn.pending_steering.clear();
            turn.last_consumed_steering_sequence = pending.last().map(|input| input.sequence);
            let first_sequence = pending
                .first()
                .expect("pending steering was checked")
                .sequence;
            let through_sequence = pending
                .last()
                .expect("pending steering was checked")
                .sequence;
            append_history(record, &items);
            record.status.revision = record
                .status
                .revision
                .checked_add(1)
                .expect("agent snapshot revision exhausted");
            record.status.updated_at = now_millis();
            let history_event = record_event(
                record,
                identity,
                AgentEventKind::HistoryCommitted {
                    start_index: record.conversation.history.len() - items.len(),
                    item_count: items.len(),
                    usage: None,
                },
                self.inner.limits,
            );
            let consumed_event = record_event(
                record,
                identity,
                AgentEventKind::SteeringConsumed {
                    first_sequence,
                    through_sequence,
                    item_count: items.len(),
                },
                self.inner.limits,
            );
            (
                StagedPersistence::commit(&cell, record, previous),
                history_event,
                consumed_event,
            )
        };
        self.persist_staged(staged).await?;
        event_sink.emit(history_event);
        event_sink.emit(consumed_event);
        Ok(true)
    }
}
