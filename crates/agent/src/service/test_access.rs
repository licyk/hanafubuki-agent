use super::*;

impl AgentApplicationService {
    pub(crate) fn test_session_driver(&self) -> &AgentSessionDriver {
        &self.driver
    }

    pub(crate) fn test_executions(&self) -> &AgentExecutionService {
        &self.executions
    }

    pub(crate) fn test_replace_executions(&mut self, executions: AgentExecutionService) {
        if let Some(directory) = self.executions.working_directory() {
            executions.set_working_directory(directory);
        }
        self.executions = executions;
    }

    pub(crate) fn test_plugin_registries(&self) -> hanafubuki_agent_kernel::AgentKernelRegistries {
        self.host.registries().clone()
    }

    /// The gate an MCP reconnect holds while it runs.
    pub(crate) fn test_mcp_configuration_gate(&self) -> Arc<tokio::sync::Mutex<()>> {
        self.mcp_application.mcp_configuration_gate.clone()
    }

    /// The proxy policy the MCP HTTP client was last built with.
    pub(crate) fn test_mcp_proxy_policy(&self) -> hanafubuki_agent_mcp::McpHttpProxyPolicy {
        self.mcp_application.proxy.lock().clone()
    }

    #[cfg(unix)]
    pub(crate) fn test_mcp_status(
        &self,
        server_id: &hanafubuki_agent_mcp::McpServerId,
    ) -> Option<hanafubuki_agent_mcp::McpServerStatus> {
        self.mcp
            .statuses()
            .into_iter()
            .find(|status| &status.server_id == server_id)
    }
}

impl AgentApplicationService {
    /// Panics in a member of the Agent's worker group: a kernel fault.
    pub(crate) fn test_panic_in_agent_worker(&self) {
        self.workers
            .spawn("agent-test-worker", |_| panic!("an Agent worker panicked"))
            .unwrap();
    }
}
