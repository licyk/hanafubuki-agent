use crate::{
    McpDiagnosticOutcome, McpDiagnosticStage, McpError, McpServerConfig, McpTransportConfig,
};
use hanafubuki_agent_foundation::cancellation::CancellationToken;
use hanafubuki_agent_tool_contracts::canonical_digest;
use std::time::Duration;

pub(super) fn bounded_error(message: String) -> String {
    crate::sanitize_mcp_diagnostic(&message, 2048)
}

pub(super) fn config_generation(config: &McpServerConfig) -> Result<[u8; 32], McpError> {
    canonical_digest(config).map_err(|_| McpError::LimitExceeded)
}

pub(super) fn transport_name(config: &McpServerConfig) -> &'static str {
    match &config.transport {
        McpTransportConfig::Stdio(_) => "stdio",
        McpTransportConfig::StreamableHttp(_) => "streamable_http",
    }
}

pub(super) fn diagnostic_error(
    error: &McpError,
) -> (McpDiagnosticStage, McpDiagnosticOutcome, &'static str) {
    match error {
        McpError::ResolveProgram(_) => (
            McpDiagnosticStage::ResolveProgram,
            McpDiagnosticOutcome::Error,
            "resolve_program_failed",
        ),
        McpError::Cancelled => (
            McpDiagnosticStage::Initialize,
            McpDiagnosticOutcome::Cancelled,
            "operation_cancelled",
        ),
        McpError::CleanupUnconfirmed(_) => (
            McpDiagnosticStage::Cleanup,
            McpDiagnosticOutcome::OutcomeUnknown,
            "cleanup_unconfirmed",
        ),
        McpError::Protocol(_) => (
            McpDiagnosticStage::Initialize,
            McpDiagnosticOutcome::Error,
            "protocol_failed",
        ),
        McpError::Timeout => (
            McpDiagnosticStage::Initialize,
            McpDiagnosticOutcome::Error,
            "initialize_timeout",
        ),
        McpError::Unavailable(_) => (
            McpDiagnosticStage::Spawn,
            McpDiagnosticOutcome::Error,
            "spawn_failed",
        ),
        _ => (
            McpDiagnosticStage::Initialize,
            McpDiagnosticOutcome::Error,
            "operation_failed",
        ),
    }
}

pub(super) async fn wait_cancelled(cancellation: &CancellationToken) {
    while !cancellation.is_cancelled() {
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}
