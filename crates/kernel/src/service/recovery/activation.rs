use super::*;

impl AgentService {
    pub(in crate::service) fn ensure_session_loaded(
        &self,
        session_id: AgentSessionId,
        model: Arc<dyn ModelPort>,
    ) -> Result<(), AgentError> {
        if self.inner.sessions.contains(session_id) {
            return Ok(());
        }
        let mut runtime = self
            .inner
            .history_store
            .load_runtime_state(session_id)
            .map_err(history_store_error)?;
        let model_context = self
            .inner
            .history_store
            .load_latest_model_context(session_id)
            .map_err(history_store_error)?;
        let history_contents = history_contents(&model_context);
        self.validate_persisted_session(&mut runtime, &model_context, &history_contents)?;
        if runtime
            .turn
            .as_ref()
            .is_some_and(|turn| !turn.phase.is_terminal())
            || runtime
                .compaction
                .as_ref()
                .is_some_and(|compaction| !compaction.status.is_terminal())
            || runtime.pending_user_input.is_some()
            || runtime
                .activities
                .iter()
                .any(|activity| !activity.phase.is_terminal())
            || runtime
                .turn
                .as_ref()
                .is_some_and(|turn| turn.reasoning.is_active())
        {
            return Err(AgentError::Persistence(format!(
                "Agent session {session_id} has an unresolved recovery obligation"
            )));
        }
        let record = self.persisted_record(runtime, model_context, &history_contents, model);
        let next_turn_id = record
            .runtime
            .turn
            .as_ref()
            .map(|turn| turn.id.value().saturating_add(1));
        if self.inner.sessions.insert_if_absent(record) {
            if let Some(next_turn_id) = next_turn_id {
                self.inner
                    .next_turn_id
                    .fetch_max(next_turn_id, Ordering::AcqRel);
            }
        }
        Ok(())
    }
}
