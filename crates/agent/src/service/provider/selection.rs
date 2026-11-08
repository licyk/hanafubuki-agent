use crate::service::AgentApplicationService;
use crate::{
    model_api_protocol, AgentModelInfo, AgentSessionModelSelectionRequest,
    AgentSessionReasoningEffortRequest,
};
use hanafubuki_agent_kernel::AgentSessionRuntimeSnapshot;

impl AgentApplicationService {
    pub async fn set_session_model(
        &self,
        request: AgentSessionModelSelectionRequest,
    ) -> Result<AgentSessionRuntimeSnapshot, String> {
        if request.model.trim().is_empty() {
            return Err("Agent model must contain 1 to 256 bytes".to_string());
        }
        let resolved = self.resolve_provider(&request.provider_id).await?;
        let policy = resolved.policy;
        let provider = resolved.definition;
        let model = request.model;
        let model_info = AgentModelInfo::from_provider(
            model.clone(),
            provider.protocol,
            &self.base_instructions,
        );
        let service = self.clone();
        hanafubuki_agent_foundation::blocking::run_blocking_operation(
            self.blocking.as_ref(),
            "agent session model mutation worker",
            move || {
                service
                    .core
                    .activate_session(request.session_id)
                    .map_err(|error| error.to_string())?;
                service
                    .core
                    .set_session_model_configuration(
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

    pub async fn set_session_reasoning_effort(
        &self,
        request: AgentSessionReasoningEffortRequest,
    ) -> Result<AgentSessionRuntimeSnapshot, String> {
        self.activate_provider_session(request.session_id).await?;
        let service = self.clone();
        hanafubuki_agent_foundation::blocking::run_blocking_operation(
            self.blocking.as_ref(),
            "agent session reasoning effort worker",
            move || {
                service
                    .core
                    .set_session_reasoning_effort(
                        request.session_id,
                        request.generation,
                        request.revision,
                        request.effort,
                    )
                    .map_err(|error| error.to_string())
            },
        )
        .await?
    }
}
