use crate::{SkillAvailabilityState, SkillToolDependency};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpApplicationSnapshot {
    pub settings: hanafubuki_agent_mcp::McpSettings,
    pub servers: Vec<hanafubuki_agent_mcp::McpServerStatus>,
    pub oauth: Vec<hanafubuki_agent_mcp::McpOAuthStatus>,
    pub tools: Vec<McpToolSnapshot>,
    pub hidden_tool_count: usize,
    pub pending_elicitations: Vec<hanafubuki_agent_mcp::McpElicitationRequest>,
    /// Each configured server's current diagnostics, in `settings.servers`
    /// order.
    pub diagnostics: Vec<hanafubuki_agent_mcp::McpDiagnosticSummary>,
    /// What reading the settings file repaired since the Agent started; the
    /// repaired file no longer shows it.
    pub settings_repairs: Vec<hanafubuki_agent_mcp::McpSettingsRepair>,
    /// What a server the user adds starts with.
    pub server_defaults: hanafubuki_agent_mcp::McpServerDefaults,
    /// The servers the enabled installed plugins run, which `settings`
    /// does not hold and the user does not edit.
    pub plugin_servers: Vec<super::McpPluginServerSnapshot>,
}

/// Whether a referenced credential is configured.
///
/// Diagnostics are read by the Agent model and surfaced in status output, so
/// they report only that a value exists. The value itself belongs to the
/// settings surface the user edits, never to a tool result.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpCredentialPresence {
    pub key: hanafubuki_agent_mcp::McpCredentialKey,
    pub present: bool,
    pub generation: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpToolSnapshot {
    pub server_id: hanafubuki_agent_mcp::McpServerId,
    pub remote_name: String,
    pub model_name: String,
    pub title: Option<String>,
    pub description: String,
    pub model_visible: bool,
    pub app_visible: bool,
    pub hints: hanafubuki_agent_mcp::McpToolHints,
    pub exposure: hanafubuki_agent_mcp::McpExposurePolicy,
    pub approval: hanafubuki_agent_mcp::McpApprovalPolicy,
    pub connection_generation: String,
    pub catalog_generation: String,
    pub tool_generation: String,
    pub input_schema: serde_json::Value,
    pub output_schema: Option<serde_json::Value>,
    pub schema_digest: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpLocalStatusSnapshot {
    pub verbose: bool,
    pub settings_revision: u64,
    pub server_count: usize,
    pub connected_count: usize,
    pub connecting_count: usize,
    pub error_count: usize,
    pub disabled_count: usize,
    pub runtime_nonhidden_tool_count: usize,
    pub hidden_tool_count: usize,
    pub credential_key_count: usize,
    pub credential_present_count: usize,
    pub diagnostics_volatile_until_restart: bool,
    pub servers: Vec<McpLocalServerStatus>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpLocalServerStatus {
    pub server_id: hanafubuki_agent_mcp::McpServerId,
    pub transport: String,
    pub configured_target: String,
    pub phase: hanafubuki_agent_mcp::McpConnectionPhase,
    pub enabled: bool,
    pub required: bool,
    pub settings_generation: String,
    pub connection_generation: Option<String>,
    pub catalog_generation: Option<String>,
    pub runtime_tool_count: usize,
    pub hidden_tool_count: usize,
    pub credential_presence: Vec<McpCredentialPresence>,
    pub oauth: hanafubuki_agent_mcp::McpOAuthStatus,
    pub cleanup_pending: bool,
    pub last_error: Option<String>,
    pub reconnect: Option<hanafubuki_agent_mcp::McpReconnectStatus>,
    pub diagnostic_summary: hanafubuki_agent_mcp::McpDiagnosticSummary,
    pub resolved_program: Option<hanafubuki_agent_mcp::McpResolvedProgram>,
    pub capabilities: Option<hanafubuki_agent_mcp::McpCapabilitySnapshot>,
    pub policy: Option<McpLocalPolicySummary>,
    pub tools: Vec<McpToolSnapshot>,
    pub diagnostics: Option<hanafubuki_agent_mcp::McpDiagnosticSnapshot>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpLocalPolicySummary {
    pub connect_timeout_seconds: u64,
    pub initialize_timeout_seconds: u64,
    pub list_timeout_seconds: u64,
    pub call_timeout_seconds: u64,
    pub shutdown_timeout_seconds: u64,
    pub maximum_concurrent_calls: usize,
    pub tool_maximum_concurrent_calls: BTreeMap<String, usize>,
    pub approval_policy: hanafubuki_agent_mcp::McpApprovalPolicy,
    pub tool_approval_policies: BTreeMap<String, hanafubuki_agent_mcp::McpApprovalPolicy>,
    pub exposure_policy: hanafubuki_agent_mcp::McpExposurePolicy,
    pub tool_exposure_policies: BTreeMap<String, hanafubuki_agent_mcp::McpExposurePolicy>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillMcpDependencyStatus {
    pub dependency: SkillToolDependency,
    pub state: SkillAvailabilityState,
    pub reason: Option<String>,
    pub server_generation: Option<String>,
    pub connection_generation: Option<String>,
    pub action: String,
}

/// The complete desired set of credential values.
///
/// The settings surface edits every credential together, so it writes them
/// together: one revision-gated update instead of a chain of per-key writes
/// that can stop halfway and leave the configuration inconsistent. Keys absent
/// from the request are removed.
#[derive(Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct McpCredentialsWriteRequest {
    pub expected_revision: u64,
    pub credentials: BTreeMap<hanafubuki_agent_mcp::McpCredentialKey, String>,
}

impl std::fmt::Debug for McpCredentialsWriteRequest {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("McpCredentialsWriteRequest")
            .field("expected_revision", &self.expected_revision)
            .field("keys", &self.credentials.keys().collect::<Vec<_>>())
            .finish()
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct McpServerWriteRequest {
    pub expected_revision: u64,
    pub server: hanafubuki_agent_mcp::McpServerConfig,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct McpServerDeleteRequest {
    pub expected_revision: u64,
    pub server_id: hanafubuki_agent_mcp::McpServerId,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct McpOAuthLoginRequest {
    pub server_id: hanafubuki_agent_mcp::McpServerId,
    pub redirect_uri: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct McpOAuthCallbackRequest {
    pub server_id: hanafubuki_agent_mcp::McpServerId,
    pub flow_id: hanafubuki_agent_mcp::McpOAuthFlowId,
    pub callback_url: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct McpOAuthCancelRequest {
    pub server_id: hanafubuki_agent_mcp::McpServerId,
    pub flow_id: hanafubuki_agent_mcp::McpOAuthFlowId,
}
