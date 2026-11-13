use super::*;

impl AgentService {
    pub(super) async fn commit_sampling_response(
        &self,
        identity: TurnIdentity,
        items: &[ConversationItem],
        reported_usage: Option<hanafubuki_agent_model_api::ModelUsage>,
        estimated_input_tokens: u64,
        estimated_output_tokens: u64,
        event_sink: &dyn AgentEventSink,
    ) -> Result<(), AgentError> {
        let (staged, event) = {
            let cell = self.session(identity.session_id)?;
            let mut guard = cell.lock_after_persistence().await?;
            let record = &mut *guard;
            let previous = record.rollback_state();
            exact_turn_mut(record, identity)?;
            validate_history_append(record, items, self.inner.limits)?;
            let usage_update = next_model_usage(
                record,
                reported_usage,
                estimated_input_tokens,
                estimated_output_tokens,
            )?;
            if let Some(usage) = usage_update {
                let policy = record.settings.budget_policy;
                exact_turn_mut(record, identity)?
                    .budget_ledger
                    .record_model_usage(
                        usage.last_response_input_tokens,
                        usage.last_response_output_tokens,
                        policy,
                    )
                    .map_err(map_budget_error)?;
            }
            // A listing recorded while a commit was staged is taken here,
            // before the next step's compaction check, and the usage is
            // measured against the window taken.
            self.refresh_context_window(record);
            let usage_update = usage_update.map(|usage| AgentModelUsage {
                context_window_tokens: record.settings.config.context_window_tokens,
                ..usage
            });
            if let Some(usage) = usage_update {
                if record.runtime.context_window.prefill_input_tokens.is_none() {
                    record.runtime.context_window.prefill_input_tokens =
                        Some(usage.last_response_input_tokens);
                }
                record.runtime.usage = Some(usage);
            }
            let start_index = record.conversation.history.len();
            append_history(record, items);
            let event = record_event(
                record,
                identity,
                AgentEventKind::HistoryCommitted {
                    start_index,
                    item_count: items.len(),
                    usage: usage_update,
                },
                self.inner.limits,
            );
            (StagedPersistence::commit(&cell, record, previous), event)
        };
        self.persist_staged(staged).await?;
        event_sink.emit(event);
        Ok(())
    }
}
