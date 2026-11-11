use crate::service::AgentApplicationService;
use crate::{model_compaction_capability, ResolvedAgentProvider, ResolvedModelPolicy};
use hanafubuki_agent_config::{
    AgentImageInputPolicy, AgentInputCapabilitiesSettings, AgentModelPolicySettings,
    AgentProviderRegistrySettings, AgentProviderSettings, AgentTurnBudgetSettings,
};
use hanafubuki_agent_kernel::{AgentConfiguredModelFacts, ModelCredential, ModelHeader};

/// The shared model policy as the session receives it. Values only some
/// protocols carry are passed through: the kernel narrows them for the
/// session's protocol (`normalize_session_config`), the one place that rule
/// lives.
pub(super) fn resolve_model_policy(policy: &AgentModelPolicySettings) -> ResolvedModelPolicy {
    ResolvedModelPolicy {
        model: policy.model.trim().to_string(),
        reasoning_effort: policy.reasoning_effort.clone(),
        input_capabilities: provider_input_capabilities(policy.input_capabilities),
        context_window_tokens: policy.context_window_tokens,
        auto_compaction_enabled: policy.auto_compaction_enabled,
        auto_compact_threshold_percent: policy.auto_compact_threshold_percent,
        compaction_capability: model_compaction_capability(policy.remote_compaction),
        turn_budget: provider_turn_budget(policy.turn_budget),
    }
}

impl AgentApplicationService {
    pub(in crate::service) async fn resolve_provider(
        &self,
        provider_id: &str,
    ) -> Result<ResolvedAgentProvider, String> {
        let registry = self.load_provider_settings().await?;
        resolve_settings_provider(&registry, provider_id)
    }
}

/// The enabled Provider `provider_id` of `registry`, with the global model
/// policy applied.
pub(crate) fn resolve_settings_provider(
    registry: &AgentProviderRegistrySettings,
    provider_id: &str,
) -> Result<ResolvedAgentProvider, String> {
    let provider = registry
        .providers
        .iter()
        .find(|provider| provider.id == provider_id)
        .cloned()
        .ok_or_else(|| format!("Agent Provider {provider_id} no longer exists"))?;
    if !provider.enabled {
        return Err(format!(
            "Agent Provider {} is disabled",
            provider.display_name
        ));
    }
    resolve_provider_definition(provider, &registry.model_policy)
}
pub(crate) fn resolve_provider_definition(
    provider: AgentProviderSettings,
    policy: &AgentModelPolicySettings,
) -> Result<ResolvedAgentProvider, String> {
    let credential = provider
        .api_token
        .as_deref()
        .map(|token| {
            ModelCredential::new(token)
                .map(|_| token.to_string())
                .map_err(|error| error.to_string())
        })
        .transpose()?;
    let mut headers = Vec::with_capacity(provider.request_headers.len());
    for header in &provider.request_headers {
        headers.push(
            ModelHeader::new(&header.name, header.value.clone())
                .map_err(|error| error.to_string())?,
        );
    }
    let policy = resolve_model_policy(policy);
    Ok(ResolvedAgentProvider {
        definition: provider,
        credential,
        headers,
        policy,
    })
}

/// What the person configured for `model` on `provider`: its override
/// (`models."<id>"`), and its window, else `policy_window` (the policy's
/// `contextWindowTokens`, which the override beats for this model).
pub(crate) fn configured_model_facts(
    provider: &AgentProviderSettings,
    policy_window: Option<u64>,
    model: &str,
) -> AgentConfiguredModelFacts {
    let overrides = provider.models.get(model).cloned().unwrap_or_default();
    AgentConfiguredModelFacts {
        context_window_tokens: overrides.context_window_tokens.or(policy_window),
        max_output_tokens: overrides.max_output_tokens,
        image_input: overrides.image_input.configured(),
        reasoning_efforts: overrides.reasoning_efforts,
    }
}

/// A Provider may be the default only while it is available. The model is no
/// longer its property, so availability is the whole condition.
fn provider_input_capabilities(
    capabilities: AgentInputCapabilitiesSettings,
) -> hanafubuki_agent_kernel::ModelInputCapabilityPolicy {
    hanafubuki_agent_kernel::ModelInputCapabilityPolicy {
        image_input: match capabilities.image_input {
            AgentImageInputPolicy::Auto => hanafubuki_agent_kernel::ModelImageInputPolicy::Auto,
            AgentImageInputPolicy::Disabled => {
                hanafubuki_agent_kernel::ModelImageInputPolicy::Disabled
            }
        },
    }
}

fn provider_turn_budget(
    policy: Option<AgentTurnBudgetSettings>,
) -> hanafubuki_agent_kernel::AgentTurnBudgetPolicy {
    policy
        .map(|policy| hanafubuki_agent_kernel::AgentTurnBudgetPolicy {
            maximum_cumulative_model_input_tokens: policy.maximum_cumulative_model_input_tokens,
            maximum_cumulative_model_output_tokens: policy.maximum_cumulative_model_output_tokens,
            maximum_tool_invocations: policy.maximum_tool_invocations,
            maximum_tool_output_bytes: policy.maximum_tool_output_bytes,
            maximum_hook_fragment_bytes: policy.maximum_hook_fragment_bytes,
            maximum_pending_input_items: policy.maximum_pending_input_items,
            maximum_in_flight_tools: policy.maximum_in_flight_tools,
            maximum_consecutive_stalled_steps: policy.maximum_consecutive_stalled_steps,
            model_stream_idle_timeout_seconds: policy.model_stream_idle_timeout_seconds,
            tool_idle_timeout_seconds: policy.tool_idle_timeout_seconds,
            approval_timeout_seconds: policy.approval_timeout_seconds,
        })
        .unwrap_or_default()
}
