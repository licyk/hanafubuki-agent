use hanafubuki_agent_config::{
    AgentModelPolicySettings, AgentModelProtocol, AgentProviderHeaderSettings,
    AgentProviderRegistrySettings, AgentProviderSettings,
};
use hanafubuki_agent_kernel::{
    AgentGeneration, AgentRevision, AgentSessionId, ModelCatalogEntry, ModelHeader,
    ModelReasoningEffort,
};
use serde::{Deserialize, Serialize};

pub type AgentProviderRegistrySnapshot = AgentProviderRegistrySettings;

#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AgentProviderModelCatalogRequest {
    pub provider_ids: Option<Vec<String>>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentProviderModelCatalogResult {
    pub provider_id: String,
    pub display_name: String,
    pub models: Vec<ModelCatalogEntry>,
    pub error: Option<String>,
}

/// One Provider as edited by the user: where a model comes from.
///
/// The draft carries the connection, the credentials, and the model id
/// together, so creating or updating a Provider is a single durable write
/// rather than a sequence of partial mutations that can half-succeed. How the
/// model is run is not here — that is the shared `AgentModelPolicySettings`.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AgentProviderDraft {
    pub display_name: String,
    pub enabled: bool,
    pub protocol: AgentModelProtocol,
    pub base_url: String,
    pub api_token: Option<String>,
    pub request_headers: Vec<AgentProviderHeaderSettings>,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AgentProviderWriteRequest {
    pub expected_revision: u64,
    /// `None` creates a Provider; the backend owns the new identifier.
    pub provider_id: Option<String>,
    pub provider: AgentProviderDraft,
}

/// The whole "which model, run how" decision, written together.
///
/// The active Provider and the model it serves are one choice, so the default
/// Provider travels with the policy rather than needing a second call that
/// could land separately.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AgentModelPolicyWriteRequest {
    pub expected_revision: u64,
    pub default_provider_id: Option<String>,
    pub policy: AgentModelPolicySettings,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AgentProviderEnabledRequest {
    pub expected_revision: u64,
    pub provider_id: String,
    pub enabled: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AgentProviderDeleteRequest {
    pub expected_revision: u64,
    pub provider_id: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AgentSessionModelSelectionRequest {
    pub session_id: AgentSessionId,
    pub generation: AgentGeneration,
    pub revision: AgentRevision,
    pub provider_id: String,
    pub model: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AgentSessionReasoningEffortRequest {
    pub session_id: AgentSessionId,
    pub generation: AgentGeneration,
    pub revision: AgentRevision,
    pub effort: Option<ModelReasoningEffort>,
}

#[derive(Debug)]
pub(crate) struct ResolvedAgentProvider {
    pub definition: AgentProviderSettings,
    pub credential: Option<String>,
    pub headers: Vec<ModelHeader>,
    pub policy: ResolvedModelPolicy,
}

/// The shared model policy, resolved for a session.
///
/// Remote compaction exists only on the Responses protocol. The stored
/// policy keeps whatever the user configured and this carries it
/// unchanged; the kernel narrows it for a session on another protocol
/// (`normalize_session_config`), so switching the active Provider degrades
/// instead of failing. The reasoning effort, which every protocol carries,
/// is narrowed per request to what the model is known to take.
#[derive(Debug)]
pub(crate) struct ResolvedModelPolicy {
    pub model: String,
    pub reasoning_effort: Option<ModelReasoningEffort>,
    pub input_capabilities: hanafubuki_agent_kernel::ModelInputCapabilityPolicy,
    pub context_window_tokens: Option<u64>,
    pub auto_compaction_enabled: bool,
    pub auto_compact_threshold_percent: u8,
    pub compaction_capability: hanafubuki_agent_kernel::ModelCompactionCapability,
    pub turn_budget: hanafubuki_agent_kernel::AgentTurnBudgetPolicy,
}
