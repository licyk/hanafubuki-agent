use super::*;

impl AgentService {
    /// Every live session's snapshot, by id, each taken once its own staged
    /// commit settled. Off the async runtime only.
    pub fn list_snapshots(&self) -> Vec<AgentSessionRuntimeSnapshot> {
        self.inner
            .sessions
            .cells()
            .into_iter()
            .filter_map(|cell| {
                let record = cell.lock_after_persistence_blocking().ok()?;
                Some(snapshot(cell.session_id(), &record))
            })
            .collect()
    }

    /// The permission mode the user chose for the session, if any. Turns
    /// take it in place of the configured one.
    pub fn session_permission_mode(
        &self,
        session_id: AgentSessionId,
    ) -> Result<Option<hanafubuki_agent_approval::AgentPermissionMode>, AgentError> {
        let cell = self.session(session_id)?;
        let record = cell.lock()?;
        Ok(record.runtime.permission_mode)
    }

    /// Sets the session's permission mode until the session ends. A running
    /// turn keeps the mode it was admitted with; the next one takes this.
    /// Off the async runtime only.
    pub fn set_session_permission_mode(
        &self,
        session_id: AgentSessionId,
        mode: hanafubuki_agent_approval::AgentPermissionMode,
    ) -> Result<(), AgentError> {
        let cell = self.session(session_id)?;
        // After the staged commit settles, so its rollback cannot undo this.
        let mut record = cell.lock_after_persistence_blocking()?;
        record.runtime.permission_mode = Some(mode);
        Ok(())
    }

    pub fn set_session_turn_budget_policy(
        &self,
        session_id: AgentSessionId,
        policy: AgentTurnBudgetPolicy,
    ) -> Result<(), AgentError> {
        let policy = policy
            .validate()
            .map_err(|error| AgentError::InvalidInput(error.to_string()))?;
        let cell = self.session(session_id)?;
        let mut guard = cell.lock_after_persistence_blocking()?;
        let record = &mut *guard;
        ensure_no_staged_commit(record)?;
        if record
            .runtime
            .turn
            .as_ref()
            .is_some_and(|turn| !turn.phase.is_terminal())
        {
            return Err(AgentError::TurnAlreadyRunning);
        }
        let previous = record.rollback_state();
        record.settings.budget_policy = policy;
        record.status.revision = record
            .status
            .revision
            .checked_add(1)
            .expect("agent snapshot revision exhausted");
        record.status.updated_at = now_millis();
        let staged = StagedPersistence::commit(&cell, record, previous);
        drop(guard);
        self.persist_staged_blocking(staged)?;
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub fn set_session_model_configuration(
        &self,
        session_id: AgentSessionId,
        generation: AgentGeneration,
        revision: AgentRevision,
        protocol: ModelApiProtocol,
        base_url: &str,
        credential: Option<String>,
        headers: Vec<hanafubuki_agent_model_api::ModelHeader>,
        request_retry_policy: ModelRetryPolicy,
        update: AgentSessionModelConfigurationUpdate,
    ) -> Result<AgentSessionRuntimeSnapshot, AgentError> {
        let model = configured_model(
            protocol,
            base_url,
            credential,
            headers,
            request_retry_policy,
            update.compaction_capability,
        )?;
        let cell = self.session(session_id)?;
        let mut guard = cell.lock_after_persistence_blocking()?;
        let record = &mut *guard;
        validate_session_mutation(record, generation, revision)?;
        ensure_no_staged_commit(record)?;
        if record
            .runtime
            .turn
            .as_ref()
            .is_some_and(|turn| !turn.phase.is_terminal())
        {
            return Err(AgentError::TurnAlreadyRunning);
        }
        if record
            .runtime
            .compaction
            .as_ref()
            .is_some_and(|compaction| !compaction.snapshot.status.is_terminal())
        {
            return Err(AgentError::CompactionAlreadyRunning);
        }
        let previous = record.rollback_state();
        record.settings.config.provider_id = update.provider_id;
        record.settings.config.provider_protocol = update.provider_protocol;
        record.settings.config.model = update.model;
        record.settings.config.instructions = update.instructions;
        record.settings.config.reasoning_effort = update.reasoning_effort;
        record.settings.config.context_window_tokens = self.inner.model_catalog.context_window(
            update.context_window_tokens,
            base_url,
            &record.settings.config.model,
        );
        record.settings.config.input_capabilities = update.input_capabilities;
        record.settings.config.auto_compaction_enabled = update.auto_compaction_enabled;
        record.settings.config.auto_compact_threshold_percent =
            update.auto_compact_threshold_percent;
        record.settings.config.compaction_capability = update.compaction_capability;
        normalize_session_config(&mut record.settings.config);
        validate_session_config(&record.settings.config, self.inner.limits)?;
        // Recovery refuses usage measured against another window.
        if let Some(usage) = record.runtime.usage.as_mut() {
            usage.context_window_tokens = record.settings.config.context_window_tokens;
        }
        record.settings.model = model;
        record.settings.effective_input_capabilities = record
            .settings
            .config
            .input_capabilities
            .effective(ModelImageInputCapability::Unknown);
        record.settings.budget_policy = update
            .turn_budget
            .validate()
            .map_err(|error| AgentError::InvalidInput(error.to_string()))?;
        record.status.revision = record
            .status
            .revision
            .checked_add(1)
            .expect("agent snapshot revision exhausted");
        record.status.updated_at = now_millis();
        let staged = StagedPersistence::commit(&cell, record, previous);
        drop(guard);
        self.persist_staged_blocking(staged)?;
        self.snapshot(session_id)
    }

    pub fn set_session_reasoning_effort(
        &self,
        session_id: AgentSessionId,
        generation: AgentGeneration,
        revision: AgentRevision,
        effort: Option<hanafubuki_agent_model_api::ModelReasoningEffort>,
    ) -> Result<AgentSessionRuntimeSnapshot, AgentError> {
        let cell = self.session(session_id)?;
        let mut guard = cell.lock_after_persistence_blocking()?;
        let record = &mut *guard;
        validate_session_mutation(record, generation, revision)?;
        ensure_no_staged_commit(record)?;
        if record
            .runtime
            .turn
            .as_ref()
            .is_some_and(|turn| !turn.phase.is_terminal())
        {
            return Err(AgentError::TurnAlreadyRunning);
        }
        if record
            .runtime
            .compaction
            .as_ref()
            .is_some_and(|compaction| !compaction.snapshot.status.is_terminal())
        {
            return Err(AgentError::CompactionAlreadyRunning);
        }
        let previous = record.rollback_state();
        record.settings.config.reasoning_effort = effort;
        // A protocol without reasoning effort ignores it, with a warning.
        normalize_session_config(&mut record.settings.config);
        validate_session_config(&record.settings.config, self.inner.limits)?;
        record.status.revision = record
            .status
            .revision
            .checked_add(1)
            .expect("agent snapshot revision exhausted");
        record.status.updated_at = now_millis();
        let staged = StagedPersistence::commit(&cell, record, previous);
        drop(guard);
        self.persist_staged_blocking(staged)?;
        self.snapshot(session_id)
    }
}
