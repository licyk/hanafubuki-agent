//! The model Provider registry: where models come from, with their
//! credentials, and the one policy every Provider's model runs under.

use std::collections::BTreeMap;

use hanafubuki_agent_foundation::agent_settings::{
    parse_model_request_header, ModelReasoningEffort,
};
use hanafubuki_agent_foundation::url_policy::{UrlPolicy, UrlRejection};

mod decode;
mod models;

pub(crate) use decode::decode_registry;
pub use models::{AgentModelImageInput, AgentModelOverrideSettings};

pub const MAX_AGENT_PROVIDERS: usize = 32;
pub const MAX_AGENT_PROVIDER_HEADERS: usize = 32;
pub const MAX_AGENT_PROVIDER_CREDENTIAL_BYTES: usize = 64 * 1024;

#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentProviderRegistrySettings {
    pub revision: u64,
    pub default_provider_id: Option<String>,
    pub model_policy: AgentModelPolicySettings,
    pub providers: Vec<AgentProviderSettings>,
}

/// How the Agent runs a model, independent of where the model comes from.
///
/// One policy applies to every Provider, the model included. A Provider
/// contributes its endpoint and credentials; it does not carry its own model,
/// reasoning, context, or compaction settings. Values that only one protocol understands are stored
/// here regardless and narrowed at resolution time, so switching the active
/// Provider never silently discards what the user configured.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentModelPolicySettings {
    /// The model to run. Empty until one is chosen; a conversation may be
    /// created in that state, but it cannot admit a turn.
    pub model: String,
    pub reasoning_effort: Option<ModelReasoningEffort>,
    pub input_capabilities: AgentInputCapabilitiesSettings,
    pub context_window_tokens: Option<u64>,
    pub auto_compaction_enabled: bool,
    pub auto_compact_threshold_percent: u8,
    pub remote_compaction: AgentRemoteCompactionCapability,
    pub turn_budget: Option<AgentTurnBudgetSettings>,
}

impl Default for AgentModelPolicySettings {
    fn default() -> Self {
        Self {
            model: String::new(),
            reasoning_effort: None,
            input_capabilities: AgentInputCapabilitiesSettings::default(),
            context_window_tokens: None,
            auto_compaction_enabled: true,
            auto_compact_threshold_percent: DEFAULT_AGENT_AUTO_COMPACT_THRESHOLD_PERCENT,
            remote_compaction: AgentRemoteCompactionCapability::Disabled,
            turn_budget: None,
        }
    }
}

/// Where a model comes from: one endpoint and its credentials. Which model runs
/// there, and how, belongs to `AgentModelPolicySettings`.
///
/// The API token and custom request-header values are stored verbatim: this
/// application deliberately keeps the original string the user typed, so the
/// value can be read back, displayed, and edited in place. There is no
/// platform keyring indirection, no generation counter, and no presence-only
/// projection — the settings file is the single authority for the value.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentProviderSettings {
    pub id: String,
    pub display_name: String,
    pub enabled: bool,
    pub protocol: AgentModelProtocol,
    pub base_url: String,
    pub api_token: Option<String>,
    #[serde(default)]
    pub request_headers: Vec<AgentProviderHeaderSettings>,
    /// What the person configured per model, by model id.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub models: BTreeMap<String, AgentModelOverrideSettings>,
}

/// A custom request header. The name is the identity: header names are unique
/// per Provider, so no synthetic id is carried alongside them.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentProviderHeaderSettings {
    pub name: String,
    pub value: String,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentInputCapabilitiesSettings {
    pub image_input: AgentImageInputPolicy,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentImageInputPolicy {
    #[default]
    Auto,
    Disabled,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentModelProtocol {
    Responses,
    ChatCompletions,
    AnthropicMessages,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentRemoteCompactionCapability {
    #[default]
    Disabled,
    ResponsesCompact,
    ResponsesCompactV2,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentTurnBudgetSettings {
    pub maximum_cumulative_model_input_tokens: u64,
    pub maximum_cumulative_model_output_tokens: u64,
    pub maximum_tool_invocations: u64,
    pub maximum_tool_output_bytes: u64,
    pub maximum_hook_fragment_bytes: u64,
    pub maximum_pending_input_items: usize,
    pub maximum_in_flight_tools: usize,
    pub maximum_consecutive_stalled_steps: u32,
    pub model_stream_idle_timeout_seconds: u64,
    pub tool_idle_timeout_seconds: u64,
    pub approval_timeout_seconds: u64,
}

pub const DEFAULT_AGENT_AUTO_COMPACT_THRESHOLD_PERCENT: u8 = 80;
pub const MAX_AGENT_CUMULATIVE_MODEL_INPUT_TOKENS: u64 = 100_000_000;
pub const MAX_AGENT_CUMULATIVE_MODEL_OUTPUT_TOKENS: u64 = 10_000_000;
pub const MAX_AGENT_TOOL_INVOCATIONS: u64 = 10_000;
pub const MAX_AGENT_TOOL_OUTPUT_BYTES: u64 = 256 * 1024 * 1024;
pub const MAX_AGENT_HOOK_FRAGMENT_BYTES: u64 = 16 * 1024 * 1024;
pub const MAX_AGENT_PENDING_INPUT_ITEMS: usize = 1_024;
pub const MAX_AGENT_IN_FLIGHT_TOOLS: usize = 64;
pub const MAX_AGENT_CONSECUTIVE_STALLED_STEPS: u32 = 32;
pub const MAX_AGENT_IDLE_TIMEOUT_SECONDS: u64 = 24 * 60 * 60;

pub fn validate_agent_provider_registry(
    registry: &AgentProviderRegistrySettings,
) -> Result<(), String> {
    if registry.providers.len() > MAX_AGENT_PROVIDERS {
        return Err(format!(
            "agentProviders.providers must contain at most {MAX_AGENT_PROVIDERS} entries"
        ));
    }
    validate_agent_model_policy(&registry.model_policy)?;
    let mut provider_ids = std::collections::BTreeSet::new();
    for provider in &registry.providers {
        validate_agent_provider(provider)?;
        if !provider_ids.insert(provider.id.as_str()) {
            return Err("agentProviders contains a duplicate Provider id".to_string());
        }
    }
    if let Some(default_provider_id) = registry.default_provider_id.as_deref() {
        let Some(provider) = registry
            .providers
            .iter()
            .find(|provider| provider.id == default_provider_id)
        else {
            return Err("agentProviders.defaultProviderId does not exist".to_string());
        };
        if !provider.enabled {
            return Err(
                "agentProviders.defaultProviderId must reference an enabled Provider".into(),
            );
        }
    }
    Ok(())
}

pub fn validate_agent_provider(provider: &AgentProviderSettings) -> Result<(), String> {
    validate_stable_id("agentProviders.providers[].id", &provider.id)?;
    let display_name = provider.display_name.trim();
    if display_name.is_empty() || display_name.len() > 128 {
        return Err(
            "agentProviders.providers[].displayName must contain 1 to 128 bytes".to_string(),
        );
    }
    if let Some(api_token) = provider.api_token.as_deref() {
        validate_agent_provider_credential("apiToken", api_token)?;
    }
    if provider.request_headers.len() > MAX_AGENT_PROVIDER_HEADERS {
        return Err(format!(
            "agentProviders requestHeaders must contain at most {MAX_AGENT_PROVIDER_HEADERS} entries"
        ));
    }
    let mut header_names = std::collections::BTreeSet::new();
    for header in &provider.request_headers {
        let normalized_name = header.name.to_ascii_lowercase();
        if !header_names.insert(normalized_name) {
            return Err("agentProviders contains a duplicate request header name".to_string());
        }
        validate_agent_provider_credential("requestHeaders[].value", &header.value)?;
        // The stored value is the one that will be sent, so validate the real
        // pair rather than a placeholder, by the rule the transport applies.
        parse_model_request_header(&header.name, &header.value)
            .map_err(|message| format!("invalid model request: {message}"))?;
    }
    validate_agent_provider_base_url(&provider.base_url)
}

/// Bound the shared model policy.
///
/// Protocol support is deliberately not checked here: the policy is global, so
/// a value only Responses understands stays configured while an Anthropic
/// Provider is active and is narrowed by the Agent kernel for each session
/// on another protocol.
pub fn validate_agent_model_policy(policy: &AgentModelPolicySettings) -> Result<(), String> {
    if policy.model.len() > 256 || (!policy.model.is_empty() && policy.model.trim().is_empty()) {
        return Err(
            "agentProviders.modelPolicy.model must be empty or contain 1 to 256 bytes".to_string(),
        );
    }
    if policy.context_window_tokens == Some(0) {
        return Err("agentProviders.modelPolicy.contextWindowTokens must be positive".to_string());
    }
    if !(1..=100).contains(&policy.auto_compact_threshold_percent) {
        return Err(
            "agentProviders.modelPolicy.autoCompactThresholdPercent must be between 1 and 100"
                .to_string(),
        );
    }
    if let Some(budget) = policy.turn_budget {
        validate_agent_turn_budget(budget)?;
    }
    Ok(())
}

/// Bound the representation of a stored credential. The value itself is never
/// transformed: it is rejected or kept exactly as written.
pub fn validate_agent_provider_credential(field: &str, value: &str) -> Result<(), String> {
    if value.is_empty()
        || value.len() > MAX_AGENT_PROVIDER_CREDENTIAL_BYTES
        || value.contains('\0')
        || value.contains('\r')
        || value.contains('\n')
    {
        return Err(format!(
            "agentProviders.providers[].{field} must contain 1 to {MAX_AGENT_PROVIDER_CREDENTIAL_BYTES} bytes without NUL or line breaks"
        ));
    }
    Ok(())
}

fn validate_stable_id(label: &str, value: &str) -> Result<(), String> {
    if value.is_empty()
        || value.len() > 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        return Err(format!(
            "{label} must contain 1 to 64 ASCII letters, digits, '-' or '_'"
        ));
    }
    Ok(())
}

fn validate_agent_turn_budget(policy: AgentTurnBudgetSettings) -> Result<(), String> {
    let values = [
        (
            "maximumCumulativeModelInputTokens",
            policy.maximum_cumulative_model_input_tokens,
            MAX_AGENT_CUMULATIVE_MODEL_INPUT_TOKENS,
        ),
        (
            "maximumCumulativeModelOutputTokens",
            policy.maximum_cumulative_model_output_tokens,
            MAX_AGENT_CUMULATIVE_MODEL_OUTPUT_TOKENS,
        ),
        (
            "maximumToolInvocations",
            policy.maximum_tool_invocations,
            MAX_AGENT_TOOL_INVOCATIONS,
        ),
        (
            "maximumToolOutputBytes",
            policy.maximum_tool_output_bytes,
            MAX_AGENT_TOOL_OUTPUT_BYTES,
        ),
        (
            "maximumHookFragmentBytes",
            policy.maximum_hook_fragment_bytes,
            MAX_AGENT_HOOK_FRAGMENT_BYTES,
        ),
        (
            "maximumPendingInputItems",
            policy.maximum_pending_input_items as u64,
            MAX_AGENT_PENDING_INPUT_ITEMS as u64,
        ),
        (
            "maximumInFlightTools",
            policy.maximum_in_flight_tools as u64,
            MAX_AGENT_IN_FLIGHT_TOOLS as u64,
        ),
        (
            "maximumConsecutiveStalledSteps",
            u64::from(policy.maximum_consecutive_stalled_steps),
            u64::from(MAX_AGENT_CONSECUTIVE_STALLED_STEPS),
        ),
        (
            "modelStreamIdleTimeoutSeconds",
            policy.model_stream_idle_timeout_seconds,
            MAX_AGENT_IDLE_TIMEOUT_SECONDS,
        ),
        (
            "toolIdleTimeoutSeconds",
            policy.tool_idle_timeout_seconds,
            MAX_AGENT_IDLE_TIMEOUT_SECONDS,
        ),
        (
            "approvalTimeoutSeconds",
            policy.approval_timeout_seconds,
            MAX_AGENT_IDLE_TIMEOUT_SECONDS,
        ),
    ];
    for (name, value, ceiling) in values {
        if value == 0 || value > ceiling {
            return Err(format!(
                "agentProviders.modelPolicy.turnBudget.{name} must be between 1 and {ceiling}"
            ));
        }
    }
    Ok(())
}

/// Validate the endpoint stored in the Provider registry. Credentials are
/// validated separately by `validate_agent_provider_credential`, which bounds
/// their representation without altering the stored value.
pub fn validate_agent_provider_base_url(base_url: &str) -> Result<(), String> {
    match UrlPolicy::ENDPOINT.parse(base_url) {
        Ok(_) => Ok(()),
        Err(UrlRejection::Invalid(_)) => {
            Err("agentProviders baseUrl must be a valid URL".to_string())
        }
        Err(_) => Err(
            "agentProviders baseUrl must be an HTTP(S) URL without credentials, query, or fragment"
                .to_string(),
        ),
    }
}
