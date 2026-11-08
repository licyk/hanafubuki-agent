use std::sync::Arc;

use hanafubuki_agent_foundation::blocking::InlineBlockingOperationPort;

use crate::service::AgentMcpApplication;

/// An MCP application with no server configured or connected; its settings
/// live in `settings_file`, which need not exist.
pub(crate) fn disconnected_mcp_application(
    settings_file: std::path::PathBuf,
) -> AgentMcpApplication {
    AgentMcpApplication {
        mcp: Arc::new(hanafubuki_agent_mcp::McpManager::new_with_event_sink(
            Arc::new(hanafubuki_agent_process::executor::ExecutorManager::default()),
            hanafubuki_agent_foundation::background_worker::BackgroundWorkerGroup::default(),
            Arc::new(hanafubuki_agent_mcp::NoopMcpEventSink),
        )),
        mcp_settings: hanafubuki_agent_config::McpSettingsStore::new(settings_file),
        mcp_oauth: hanafubuki_agent_mcp::McpOAuthManager::new(
            Arc::new(hanafubuki_agent_mcp::MemoryMcpSecretStore::default()),
            hanafubuki_agent_mcp::build_mcp_http_client(
                hanafubuki_agent_mcp::McpHttpProxyPolicy::Disabled,
            )
            .expect("an MCP HTTP client without a proxy builds"),
        ),
        mcp_configuration_gate: Arc::new(tokio::sync::Mutex::new(())),
        blocking: Arc::new(InlineBlockingOperationPort),
        proxy: Arc::new(parking_lot::Mutex::new(
            hanafubuki_agent_mcp::McpHttpProxyPolicy::Disabled,
        )),
        lifetime: Default::default(),
    }
}
