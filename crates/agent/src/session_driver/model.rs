//! A session's model selection, which the facade's `set_session_model` and
//! a spawned agent's role share.

use hanafubuki_agent_foundation::blocking::run_blocking_operation;
use hanafubuki_agent_kernel::{AgentSessionId, AgentSessionRuntimeSnapshot, ModelReasoningEffort};

use super::AgentSessionDriver;
use crate::service::resolve_settings_provider;
use crate::{model_api_protocol, AgentModelInfo, AgentSessionModelSelectionRequest};

impl AgentSessionDriver {
    /// Selects `request.model` of the Provider `request.provider_id` for
    /// the session, with that Provider's model policy: its reasoning
    /// effort, context window, input capabilities, compaction and turn
    /// budget.
    pub(crate) async fn select_model(
        &self,
        request: AgentSessionModelSelectionRequest,
    ) -> Result<AgentSessionRuntimeSnapshot, String> {
        if request.model.trim().is_empty() {
            return Err("Agent model must contain 1 to 256 bytes".to_string());
        }
        let config = self.config.clone();
        let registry = run_blocking_operation(
            self.blocking.as_ref(),
            "agent provider settings read worker",
            move || config.providers(),
        )
        .await??;
        let resolved = resolve_settings_provider(&registry, &request.provider_id)?;
        let policy = resolved.policy;
        let provider = resolved.definition;
        let model = request.model;
        let model_info = AgentModelInfo::from_provider(
            model.clone(),
            provider.protocol,
            &self.base_instructions,
        );
        let core = self.core.clone();
        run_blocking_operation(
            self.blocking.as_ref(),
            "agent session model mutation worker",
            move || {
                core.activate_session(request.session_id)
                    .map_err(|error| error.to_string())?;
                core.set_session_model_configuration(
                    request.session_id,
                    request.generation,
                    request.revision,
                    model_api_protocol(provider.protocol),
                    &provider.base_url,
                    resolved.credential,
                    resolved.headers,
                    hanafubuki_agent_kernel::ModelRetryPolicy::default(),
                    hanafubuki_agent_kernel::AgentSessionModelConfigurationUpdate {
                        provider_id: provider.id,
                        provider_protocol: model_api_protocol(provider.protocol),
                        model,
                        instructions: model_info.resolve_instructions(None),
                        reasoning_effort: policy.reasoning_effort,
                        context_window_tokens: policy.context_window_tokens,
                        input_capabilities: policy.input_capabilities,
                        auto_compaction_enabled: policy.auto_compaction_enabled,
                        auto_compact_threshold_percent: policy.auto_compact_threshold_percent,
                        compaction_capability: policy.compaction_capability,
                        turn_budget: policy.turn_budget,
                    },
                )
                .map_err(|error| error.to_string())
            },
        )
        .await?
    }

    /// Gives the child `session_id`, which has its parent's model, the
    /// model and reasoning effort of the role it was spawned as: the
    /// role's `model` if it sets one (of the parent's Provider), else the
    /// parent's; the role's `reasoning_effort` if it sets one, else the
    /// parent's, even across a change of model.
    pub(crate) async fn select_role_model(
        &self,
        session_id: AgentSessionId,
        model: Option<String>,
        reasoning_effort: Option<ModelReasoningEffort>,
    ) -> Result<(), String> {
        let parent = self.blocking_snapshot(session_id).await?;
        let effort = reasoning_effort.or(parent.reasoning_effort.clone());
        let snapshot = match model.filter(|model| *model != parent.model) {
            Some(model) => {
                self.select_model(AgentSessionModelSelectionRequest {
                    session_id,
                    generation: parent.generation,
                    revision: parent.revision,
                    provider_id: parent.provider_id.clone(),
                    model,
                })
                .await?
            }
            None => parent,
        };
        if snapshot.reasoning_effort == effort {
            return Ok(());
        }
        let (generation, revision) = (snapshot.generation, snapshot.revision);
        let core = self.core.clone();
        run_blocking_operation(
            self.blocking.as_ref(),
            "agent session reasoning effort worker",
            move || {
                core.set_session_reasoning_effort(session_id, generation, revision, effort)
                    .map(|_| ())
                    .map_err(|error| error.to_string())
            },
        )
        .await?
    }

    async fn blocking_snapshot(
        &self,
        session_id: AgentSessionId,
    ) -> Result<AgentSessionRuntimeSnapshot, String> {
        let driver = self.clone();
        run_blocking_operation(
            self.blocking.as_ref(),
            "agent session snapshot worker",
            move || driver.snapshot(session_id),
        )
        .await?
        .map_err(|error| error.to_string())
    }
}
