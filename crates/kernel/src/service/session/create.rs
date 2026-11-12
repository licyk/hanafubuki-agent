use super::*;

impl AgentService {
    pub fn create_session(
        &self,
        config: AgentSessionConfig,
    ) -> Result<AgentSessionRuntimeSnapshot, AgentError> {
        let model = self
            .inner
            .default_model
            .clone()
            .ok_or_else(|| AgentError::InvalidInput("no default model provider".to_string()))?;
        self.create_session_with_model(config, model, crate::AgentSessionWorkspace::default())
    }

    /// Creates a session over a configured provider in `workspace`; the
    /// session's context window is the configured one or what the model
    /// metadata knows ([`crate::model_metadata`]).
    #[allow(
        clippy::too_many_arguments,
        reason = "independent inputs that a parameter struct would only regroup"
    )]
    pub fn create_model_session(
        &self,
        mut config: AgentSessionConfig,
        protocol: ModelApiProtocol,
        base_url: &str,
        credential: Option<String>,
        headers: Vec<hanafubuki_agent_model_api::ModelHeader>,
        request_retry_policy: ModelRetryPolicy,
        compaction_capability: hanafubuki_agent_model_api::ModelCompactionCapability,
        workspace: crate::AgentSessionWorkspace,
    ) -> Result<AgentSessionRuntimeSnapshot, AgentError> {
        let model = configured_model(
            protocol,
            base_url,
            credential,
            headers,
            request_retry_policy,
            compaction_capability,
            self.model_proxy(),
        )?;
        self.name_model_provider_at(&config.provider_id, protocol, base_url);
        config.adopt_configured_window();
        let window = self.session_context_window(
            config.configured_facts.context_window_tokens,
            model.as_ref(),
            &config.model,
        );
        config.set_context_window(window);
        self.create_session_with_model(config, model, workspace)
    }

    pub fn create_unconfigured_session(
        &self,
        config: AgentSessionConfig,
        workspace: crate::AgentSessionWorkspace,
    ) -> Result<AgentSessionRuntimeSnapshot, AgentError> {
        if !config.model.is_empty() {
            return Err(AgentError::InvalidInput(
                "an unconfigured Agent session cannot contain a model".to_string(),
            ));
        }
        self.create_session_with_model(config, Arc::new(UnavailableRecoveredModel), workspace)
    }

    /// Restores durable sessions without replaying any interrupted operation.
    ///
    /// Presentation events and pending approvals are intentionally rebuilt
    /// as empty registries. An active turn becomes terminal `Interrupted`, and
    /// each committed tool call without a committed output receives a durable
    /// synthetic error output so a later sampling request cannot replay it.
    pub(super) fn create_session_with_model(
        &self,
        mut config: AgentSessionConfig,
        model: Arc<dyn ModelPort>,
        workspace: crate::AgentSessionWorkspace,
    ) -> Result<AgentSessionRuntimeSnapshot, AgentError> {
        if !self.inner.accepting.load(Ordering::Acquire) {
            return Err(AgentError::ShuttingDown);
        }
        config.adopt_configured_window();
        normalize_session_config(&mut config);
        validate_session_config(&config, self.inner.limits)?;
        if model.compaction_capability() != config.compaction_capability {
            return Err(AgentError::InvalidInput(
                "model compaction capability does not match the frozen session config".to_string(),
            ));
        }
        let session_id = self
            .inner
            .history_store
            .allocate_session_id()
            .map_err(history_store_error)?;
        let now = now_millis();
        let effective_input_capabilities = self.session_input_capabilities(model.as_ref(), &config);
        let record = SessionRecord {
            session_id,
            forked_from_session_id: None,
            lineage: SessionLineage {
                workspace,
                ..SessionLineage::default()
            },
            created_at: now,
            settings: SessionSettings {
                model,
                title: None,
                goal: None,
                config,
                memory_contribution: AgentMemoryContributionState::default(),
                effective_input_capabilities,
                budget_policy: AgentTurnBudgetPolicy::default(),
            },
            status: SessionStatus::created(now),
            conversation: SessionConversation {
                history: Arc::new(Vec::new()),
                history_version: 0,
                pending_history_mutations: Vec::new(),
                next_history_item_id: 1,
                model_context_boundary: AgentModelContextBoundary::Full,
                history_base: None,
                history_bytes: 0,
                next_message_id: 1,
            },
            journal: SessionJournal::empty(),
            runtime: SessionRuntime::idle(),
        };
        let cell = self.inner.sessions.insert(record, &self.inner.accepting)?;
        if let Err(error) = self.persist_current_conversation(&cell, true) {
            self.remove_unpersisted_session(&cell);
            return Err(error);
        }
        let snapshot = self.snapshot(session_id)?;
        Ok(snapshot)
    }

    /// Removes a session whose first commit failed.
    pub(in crate::service) fn remove_unpersisted_session(&self, cell: &SessionCell) {
        if let Ok(record) = cell.lock() {
            self.inner.sessions.remove(record);
        }
    }
}
