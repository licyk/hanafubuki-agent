use super::*;

impl AgentService {
    pub fn snapshot(
        &self,
        session_id: AgentSessionId,
    ) -> Result<AgentSessionRuntimeSnapshot, AgentError> {
        self.refresh_session_image_input_capability_blocking(session_id)?;
        let cell = self.session(session_id)?;
        let record = cell.lock_after_persistence_blocking()?;
        Ok(snapshot(session_id, &record))
    }

    /// The text of the user message that started `turn_id`, read from the
    /// session's loaded history. `None` when the history no longer holds
    /// that turn's start (a compaction replaced it).
    pub fn turn_user_message(
        &self,
        session_id: AgentSessionId,
        turn_id: AgentTurnId,
    ) -> Result<Option<String>, AgentError> {
        let cell = self.session(session_id)?;
        let record = cell.lock()?;
        let history = &record.conversation.history;
        let Some(start) = history.iter().rposition(|item| {
            matches!(&item.content, ConversationItem::TurnStart { turn_id: id } if *id == turn_id)
        }) else {
            return Ok(None);
        };
        Ok(history[start..]
            .iter()
            .find_map(|item| match &item.content {
                ConversationItem::UserMessage { text, .. } => Some(text.clone()),
                _ => None,
            }))
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn test_model_context(
        &self,
        session_id: AgentSessionId,
    ) -> Result<Vec<ConversationItem>, AgentError> {
        let cell = self.session(session_id)?;
        let record = cell.lock_after_persistence_blocking()?;
        Ok(history_contents(&record.conversation.history))
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn test_history_backing(
        &self,
        session_id: AgentSessionId,
    ) -> Result<(Arc<Vec<Arc<AgentHistoryItem>>>, u64), AgentError> {
        let cell = self.session(session_id)?;
        let record = cell.lock_after_persistence_blocking()?;
        Ok((
            record.conversation.history.clone(),
            record.conversation.history_version,
        ))
    }

    pub fn list_sessions_page(
        &self,
        request: AgentSessionListRequest,
    ) -> Result<AgentSessionPage, AgentError> {
        self.inner
            .history_store
            .list_sessions_page(request)
            .map_err(history_store_error)
    }

    pub fn read_transcript_page(
        &self,
        request: AgentTranscriptPageRequest,
    ) -> Result<AgentTranscriptPage, AgentError> {
        self.inner
            .history_store
            .list_items_page(request)
            .map_err(history_store_error)
    }

    pub fn read_turn_page(
        &self,
        request: AgentTurnPageRequest,
    ) -> Result<AgentTranscriptPage, AgentError> {
        self.inner
            .history_store
            .list_turn_items_page(request)
            .map_err(history_store_error)
    }

    pub fn open_session(
        &self,
        session_id: AgentSessionId,
        page_size: usize,
    ) -> Result<AgentSessionOpenResult, AgentError> {
        self.open_loaded_session(session_id, page_size)
    }

    pub fn activate_session(&self, session_id: AgentSessionId) -> Result<(), AgentError> {
        self.ensure_session_loaded(session_id, Arc::new(UnavailableRecoveredModel))
    }

    pub fn activate_model_session(
        &self,
        protocol: ModelApiProtocol,
        base_url: &str,
        credential: Option<String>,
        headers: Vec<hanafubuki_agent_model_api::ModelHeader>,
        request_retry_policy: ModelRetryPolicy,
        compaction_capability: hanafubuki_agent_model_api::ModelCompactionCapability,
        session_id: AgentSessionId,
    ) -> Result<(), AgentError> {
        let model = configured_model(
            protocol,
            base_url,
            credential,
            headers,
            request_retry_policy,
            compaction_capability,
        )?;
        self.ensure_session_loaded(session_id, model.clone())?;
        let cell = self.session(session_id)?;
        let mut guard = cell.lock_after_persistence_blocking()?;
        let record = &mut *guard;
        if record
            .runtime
            .turn
            .as_ref()
            .is_some_and(|turn| !turn.phase.is_terminal())
        {
            return Err(AgentError::TurnAlreadyRunning);
        }
        record.settings.model = model;
        Ok(())
    }

    pub(super) fn open_loaded_session(
        &self,
        session_id: AgentSessionId,
        page_size: usize,
    ) -> Result<AgentSessionOpenResult, AgentError> {
        let (runtime, durable_revision) = match self.snapshot(session_id) {
            Ok(snapshot) => {
                let durable_revision = self
                    .inner
                    .history_store
                    .load_runtime_state(session_id)
                    .map_err(history_store_error)?
                    .revision;
                (snapshot, durable_revision)
            }
            Err(AgentError::SessionNotFound) => {
                let persisted = self
                    .inner
                    .history_store
                    .load_runtime_state(session_id)
                    .map_err(history_store_error)?;
                let durable_revision = persisted.revision;
                (persisted_runtime_snapshot(persisted), durable_revision)
            }
            Err(error) => return Err(error),
        };
        let transcript_revision =
            AgentRevision::new(runtime.revision.value().min(durable_revision.value()));
        let transcript = self.read_transcript_page(AgentTranscriptPageRequest {
            session_id,
            page_size,
            cursor: None,
            direction: AgentHistorySortDirection::Descending,
            through_revision: Some(transcript_revision),
        })?;
        let through_revision = transcript.through_revision;
        let next_event_sequence = runtime.next_event_sequence;
        Ok(AgentSessionOpenResult {
            runtime,
            transcript,
            through_revision,
            next_event_sequence,
        })
    }
}
