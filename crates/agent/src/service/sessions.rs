use super::AgentApplicationService;
use crate::{model_api_protocol, AgentModelInfo, ResolvedAgentProvider};
use hanafubuki_agent_kernel::{
    AgentError, AgentEventSink, AgentGeneration, AgentRevision, AgentSessionConfig,
    AgentSessionForkOutcome, AgentSessionForkRequest, AgentSessionHeadForkOutcome,
    AgentSessionHeadForkRequest, AgentSessionId, AgentSessionMode, AgentSessionRuntimeSnapshot,
    AgentSessionSummary, AgentSessionTitleRenameRequest, ConfirmedAgentSessionDeletion,
};
use std::sync::atomic::Ordering;

impl AgentApplicationService {
    pub fn rename_session(
        &self,
        request: AgentSessionTitleRenameRequest,
        event_sink: &dyn AgentEventSink,
    ) -> Result<AgentSessionRuntimeSnapshot, String> {
        self.activate_session(request.session_id)
            .map_err(|error| error.to_string())?;
        self.core
            .rename_session(request, event_sink)
            .map_err(|error| error.to_string())
    }

    pub async fn create_session(
        &self,
        mode: AgentSessionMode,
    ) -> Result<AgentSessionRuntimeSnapshot, String> {
        if !self.accepting.load(Ordering::Acquire) {
            return Err("Agent service is shutting down".to_string());
        }
        // The memory plugin's observer records the new session's memory
        // contribution, so the plugins must be running.
        self.host.ensure_started().await;
        let registry = self.load_provider_settings().await?;
        let settings = self.load_agent_settings().await?;
        let provider_id = registry
            .default_provider_id
            .as_deref()
            .ok_or_else(|| "No default Agent Provider is configured".to_string())?;
        let resolved = self.resolve_provider(provider_id).await?;
        let service = self.clone();
        hanafubuki_agent_foundation::blocking::run_blocking_operation(
            self.blocking.as_ref(),
            "agent session creation worker",
            move || service.create_session_with_provider(mode, settings, resolved),
        )
        .await?
    }

    fn create_session_with_provider(
        &self,
        mode: AgentSessionMode,
        settings: hanafubuki_agent_config::AgentSettings,
        resolved: ResolvedAgentProvider,
    ) -> Result<AgentSessionRuntimeSnapshot, String> {
        let policy = resolved.policy;
        let provider = resolved.definition;
        let model_configured = !policy.model.is_empty();
        let instructions = model_configured
            .then(|| {
                AgentModelInfo::from_provider(
                    policy.model.clone(),
                    provider.protocol,
                    &self.base_instructions,
                )
                .resolve_instructions(None)
            })
            .unwrap_or_default();
        let protocol = model_api_protocol(provider.protocol);
        let compaction_capability = policy.compaction_capability;
        let turn_budget = policy.turn_budget;
        let config = AgentSessionConfig {
            provider_id: provider.id.clone(),
            provider_protocol: protocol,
            model: policy.model,
            instructions,
            mode,
            reasoning_effort: if model_configured {
                policy.reasoning_effort
            } else {
                None
            },
            context_window_tokens: if model_configured {
                policy.context_window_tokens
            } else {
                None
            },
            input_capabilities: policy.input_capabilities,
            auto_compaction_enabled: policy.auto_compaction_enabled,
            auto_compact_threshold_percent: policy.auto_compact_threshold_percent,
            compaction_capability: if model_configured {
                compaction_capability
            } else {
                hanafubuki_agent_kernel::ModelCompactionCapability::LocalSummary
            },
            sampling_retry_policy: hanafubuki_agent_kernel::AgentSamplingRetryPolicy::default(),
        };
        let snapshot = if model_configured {
            self.core.create_model_session(
                config,
                protocol,
                &provider.base_url,
                resolved.credential,
                resolved.headers,
                hanafubuki_agent_kernel::ModelRetryPolicy::default(),
                compaction_capability,
            )
        } else {
            self.core.create_unconfigured_session(config)
        }
        .map_err(|error| error.to_string())?;
        self.core
            .set_session_turn_budget_policy(snapshot.id, turn_budget)
            .map_err(|error| error.to_string())?;
        self.core
            .initialize_session_memory_contribution(
                snapshot.id,
                snapshot.memory_contribution.revision,
                settings.memory.enabled && settings.memory.generate_memories,
            )
            .map_err(|error| error.to_string())?;
        self.core
            .snapshot(snapshot.id)
            .map_err(|error| error.to_string())
    }

    pub fn fork_session_before_turn(
        &self,
        request: AgentSessionForkRequest,
    ) -> Result<AgentSessionForkOutcome, String> {
        self.activate_session(request.source_session_id)
            .map_err(|error| error.to_string())?;
        let settings = self.config.settings()?;
        let mut outcome = self
            .core
            .fork_session_before_turn(request)
            .map_err(|error| error.to_string())?;
        self.core
            .initialize_session_memory_contribution(
                outcome.session.id,
                outcome.session.memory_contribution.revision,
                settings.memory.enabled && settings.memory.generate_memories,
            )
            .map_err(|error| error.to_string())?;
        outcome.session = self
            .core
            .snapshot(outcome.session.id)
            .map_err(|error| error.to_string())?;
        Ok(outcome)
    }

    pub fn fork_session_at_head(
        &self,
        request: AgentSessionHeadForkRequest,
    ) -> Result<AgentSessionHeadForkOutcome, String> {
        self.activate_session(request.source_session_id)
            .map_err(|error| error.to_string())?;
        let settings = self.config.settings()?;
        let mut outcome = self
            .core
            .fork_session_at_head(request)
            .map_err(|error| error.to_string())?;
        self.core
            .initialize_session_memory_contribution(
                outcome.session.id,
                outcome.session.memory_contribution.revision,
                settings.memory.enabled && settings.memory.generate_memories,
            )
            .map_err(|error| error.to_string())?;
        outcome.session = self
            .core
            .snapshot(outcome.session.id)
            .map_err(|error| error.to_string())?;
        Ok(outcome)
    }

    pub fn set_session_mode(
        &self,
        session_id: AgentSessionId,
        generation: AgentGeneration,
        revision: AgentRevision,
        mode: AgentSessionMode,
    ) -> Result<AgentSessionRuntimeSnapshot, String> {
        self.activate_session(session_id)
            .map_err(|error| error.to_string())?;
        self.core
            .set_session_mode(session_id, generation, revision, mode)
            .map_err(|error| error.to_string())
    }

    pub fn archive_session(
        &self,
        session_id: AgentSessionId,
        generation: AgentGeneration,
        revision: AgentRevision,
    ) -> Result<AgentSessionSummary, AgentError> {
        self.activate_session(session_id)?;
        let before = self.core.snapshot(session_id)?;
        if before.generation != generation {
            return Err(AgentError::StaleGeneration);
        }
        if before.revision != revision {
            return Err(AgentError::StaleRevision);
        }
        self.quiesce_background_terminals(session_id)?;
        let revision = self.core.snapshot(session_id)?.revision;
        self.core.archive_session(session_id, generation, revision)
    }

    pub fn restore_session(
        &self,
        session_id: AgentSessionId,
        generation: AgentGeneration,
        revision: AgentRevision,
    ) -> Result<AgentSessionSummary, AgentError> {
        self.activate_session(session_id)?;
        self.core.restore_session(session_id, generation, revision)
    }

    pub fn delete_session_confirmed(
        &self,
        session_id: AgentSessionId,
        generation: AgentGeneration,
        revision: AgentRevision,
    ) -> Result<ConfirmedAgentSessionDeletion, AgentError> {
        self.activate_session(session_id)?;
        let before = self.core.snapshot(session_id)?;
        if before.generation != generation {
            return Err(AgentError::StaleGeneration);
        }
        if before.revision != revision {
            return Err(AgentError::StaleRevision);
        }
        let prepared = self
            .core
            .prepare_session_deletion(session_id, generation, revision)?;
        let projected = before.active_activities;
        let outcome = match self.stop_background_terminal_authority(session_id) {
            Ok(outcome) => outcome,
            Err(error) => {
                self.core.cancel_prepared_session_deletion(prepared);
                return Err(error);
            }
        };
        if !outcome.retained.is_empty() {
            self.core.cancel_prepared_session_deletion(prepared);
            self.publish_background_terminal_stop(session_id, &projected, &outcome)?;
            return Err(AgentError::SessionMutationNotConfirmed);
        }
        if let Err(error) = self.memory.revoke_session(session_id) {
            self.core.cancel_prepared_session_deletion(prepared);
            self.publish_background_terminal_stop(session_id, &projected, &outcome)?;
            return Err(AgentError::Persistence(error));
        }
        self.core.complete_session_deletion_confirmed(prepared)
    }
}
