mod capabilities;
mod catalog;
mod client_adapter;
mod config;
mod connection;
mod credentials;
mod diagnostics;
mod error;
mod event;
mod identity;
mod interaction;
mod listing;
mod liveness;
mod manager;
mod oauth;
mod output;
mod plugin_config;
mod program;
mod request;
mod settings;
mod stdio;
mod streamable_http;
#[cfg(any(test, feature = "test-support"))]
pub mod test_support;

pub use capabilities::{
    McpCapabilitySnapshot, MAX_MCP_INSTRUCTIONS_BYTES, MAX_MCP_INSTRUCTIONS_SUMMARY_BYTES,
};
pub use catalog::{
    McpCatalogIssue, McpCatalogIssueKind, McpToolBinding, McpToolCatalog, McpToolHints,
    MAX_MCP_CATALOG_ISSUE_NAME_BYTES, MAX_MCP_TOOLS_PER_SERVER, MAX_MCP_TOOL_DESCRIPTION_BYTES,
};
pub use client_adapter::{build_mcp_http_client, McpHttpProxyPolicy};
pub use config::{
    McpApprovalPolicy, McpExposurePolicy, McpOAuthPolicy, McpProtocolMode, McpServerConfig,
    McpServerDefaults, McpStdioConfig, McpStreamableHttpConfig, McpTransportConfig,
    MAX_MCP_ARGUMENTS, MAX_MCP_CONCURRENT_CALLS, MAX_MCP_ENVIRONMENT_ENTRIES, MAX_MCP_FIELD_BYTES,
    MAX_MCP_FILTERS, MAX_MCP_HEADERS, MAX_MCP_SERVERS, MAX_MCP_TIMEOUT_SECONDS,
};
pub use connection::{McpConnectionPhase, McpReconnectStatus, McpServerStatus};
pub use credentials::{
    resolve_connection_secrets, validate_credential_value, FoundationMcpSecretStore,
    McpCredentialSettings, McpSecretFuture, McpSecretStore, MemoryMcpSecretStore,
    MAX_MCP_SECRET_BYTES,
};
pub use diagnostics::{
    sanitize_mcp_diagnostic, McpDiagnosticEntry, McpDiagnosticOutcome, McpDiagnosticSnapshot,
    McpDiagnosticStage, McpDiagnosticStore, McpDiagnosticSummary, MCP_DIAGNOSTIC_CAPACITY,
    MCP_DIAGNOSTIC_MESSAGE_BYTES, MCP_STDERR_TAIL_BYTES, MCP_STDERR_TAIL_LINES,
};
pub use error::McpError;
pub use event::{McpEventKind, McpEventNotice, McpEventSink, NoopMcpEventSink};
pub use identity::{McpCatalogGeneration, McpConnectionGeneration, McpCredentialKey, McpServerId};
use interaction::McpClientHandler;
pub use interaction::{
    McpActiveCallGuard, McpCallContext, McpElicitationDecision, McpElicitationKind,
    McpElicitationRequest, McpElicitationRequestId, McpInteractionHub,
    MAXIMUM_MCP_ELICITATION_WAIT,
};
pub use manager::{
    McpConnectionSecrets, McpManager, McpResourcePage, McpStepBinding, McpStepServerBinding,
    McpToolLease,
};
pub use oauth::{McpOAuthFlowId, McpOAuthLogin, McpOAuthManager, McpOAuthPhase, McpOAuthStatus};
pub use output::convert_tool_output;
pub use plugin_config::{
    agent_plugin_mcp_servers, plugin_mcp_servers, PluginMcpServerError, PluginMcpServers,
};
pub use program::{resolve_stdio_program, validate_stdio_program_shape, McpResolvedProgram};
pub use settings::{repair_settings, McpSettings, McpSettingsRepair, McpSettingsRepairKind};
pub use stdio::{
    McpExecutorTransport, McpExecutorTransportError, McpResolvedSecretEnvironment,
    McpStdioConnection,
};
pub use streamable_http::{McpResolvedSecretHeaders, McpStreamableHttpConnection};

#[cfg(test)]
mod tests;
