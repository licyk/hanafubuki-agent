use super::*;

#[cfg(any(test, feature = "test-support"))]
type HistoryBacking = Arc<Vec<Arc<AgentHistoryItem>>>;

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

    #[cfg(any(test, feature = "test-support"))]
    pub fn test_model_context(
        &self,
        session_id: AgentSessionId,
    ) -> Result<Vec<ConversationItem>, AgentError> {
        let cell = self.session(session_id)?;
        let record = cell.lock_after_persistence_blocking()?;
        Ok(history_contents(&record.conversation.history))
    }

    /// The session's hierarchical compression state.
    #[cfg(any(test, feature = "test-support"))]
    pub fn test_compression_state(
        &self,
        session_id: AgentSessionId,
    ) -> Result<Option<crate::AgentCompressionState>, AgentError> {
        let cell = self.session(session_id)?;
        let record = cell.lock_after_persistence_blocking()?;
        Ok(record.runtime.compression.as_deref().cloned())
    }

    /// The session's content store (`hierarchical`'s originals).
    #[cfg(any(test, feature = "test-support"))]
    pub fn test_content_store(
        &self,
        session_id: AgentSessionId,
    ) -> Result<Option<crate::AgentContentStore>, AgentError> {
        let cell = self.session(session_id)?;
        let record = cell.lock_after_persistence_blocking()?;
        Ok(record.runtime.compression_runtime.content_store().cloned())
    }

    /// Fixes the clock the session's message timestamps read, until it is
    /// fixed again or the session is loaded again; `None` reads the real
    /// one.
    #[cfg(any(test, feature = "test-support"))]
    pub fn test_fix_message_clock(
        &self,
        session_id: AgentSessionId,
        clock: Option<crate::AgentLocalTime>,
    ) -> Result<(), AgentError> {
        let cell = self.session(session_id)?;
        let mut record = cell.lock_after_persistence_blocking()?;
        record.runtime.compression_runtime.fixed_clock = clock;
        Ok(())
    }

    /// Replaces the session's hierarchical compression state in memory, as
    /// a corrupt one would be read.
    #[cfg(any(test, feature = "test-support"))]
    pub fn test_set_compression_state(
        &self,
        session_id: AgentSessionId,
        state: Option<crate::AgentCompressionState>,
    ) -> Result<(), AgentError> {
        let cell = self.session(session_id)?;
        let mut record = cell.lock_after_persistence_blocking()?;
        record.runtime.compression = state.map(Arc::new);
        Ok(())
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn test_history_backing(
        &self,
        session_id: AgentSessionId,
    ) -> Result<(HistoryBacking, u64), AgentError> {
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

    #[allow(
        clippy::too_many_arguments,
        reason = "independent inputs that a parameter struct would only regroup"
    )]
    pub fn activate_model_session(
        &self,
        protocol: ModelApiProtocol,
        base_url: &str,
        credential: Option<String>,
        headers: Vec<hanafubuki_agent_model_api::ModelHeader>,
        request_retry_policy: ModelRetryPolicy,
        compaction_capability: hanafubuki_agent_model_api::ModelCompactionCapability,
        configured_facts: crate::AgentConfiguredModelFacts,
        session_id: AgentSessionId,
    ) -> Result<(), AgentError> {
        let model = configured_model(
            protocol,
            base_url,
            credential,
            headers,
            request_retry_policy,
            compaction_capability,
            self.model_proxy(),
        )?;
        self.ensure_session_loaded(session_id, model.clone())?;
        let cell = self.session(session_id)?;
        let mut guard = cell.lock_after_persistence_blocking()?;
        let record = &mut *guard;
        self.name_model_provider_at(&record.settings.config.provider_id, protocol, base_url);
        if record
            .runtime
            .turn
            .as_ref()
            .is_some_and(|turn| !turn.phase.is_terminal())
        {
            return Err(AgentError::TurnAlreadyRunning);
        }
        let compaction = reconciled_compaction_config(record, model.compaction_capability());
        // What the person configured for the model now; the session takes
        // it from its next turn (the window at that turn's admission).
        let facts_changed = record.settings.config.configured_facts != configured_facts
            && !has_active_operation(record)
            && ensure_no_staged_commit(record).is_ok();
        if compaction.is_none() && !facts_changed {
            record.settings.model = model;
            return Ok(());
        }
        let previous = record.rollback_state();
        if let Some(config) = compaction {
            // The configured remote compaction changed since the session
            // last took it: compaction checks the transport against the
            // session. A background result made with the old
            // implementation is unusable.
            record.runtime.background_compaction.cancel();
            record.settings.config = config;
        }
        if facts_changed {
            record.settings.config.configured_facts = configured_facts;
        }
        record.settings.model = model;
        // Not a revision: no snapshot shows the capability or the facts,
        // and a caller that read the revision before this activation must
        // not find it stale.
        let staged = StagedPersistence::commit(&cell, record, previous);
        drop(guard);
        self.persist_staged_blocking(staged)
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

/// The session's config with the transport's compaction `capability`, when
/// it differs and the session is idle; a capability the session's protocol
/// cannot use is left to fail at compaction, as it would after `/model`.
fn reconciled_compaction_config(
    record: &SessionRecord,
    capability: hanafubuki_agent_model_api::ModelCompactionCapability,
) -> Option<AgentSessionConfig> {
    if record.settings.config.compaction_capability == capability
        || has_active_operation(record)
        || ensure_no_staged_commit(record).is_err()
    {
        return None;
    }
    let mut config = record.settings.config.clone();
    config.compaction_capability = capability;
    normalize_session_config(&mut config);
    (config.compaction_capability == capability).then_some(config)
}
