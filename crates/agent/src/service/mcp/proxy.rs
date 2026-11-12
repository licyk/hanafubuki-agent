use hanafubuki_agent_mcp::McpHttpProxyPolicy;

use super::AgentMcpApplication;
use crate::{AgentProxy, NetworkSettingsPort};

/// The proxy MCP's Streamable HTTP and OAuth requests use.
pub(crate) fn mcp_proxy_policy(proxy: &AgentProxy) -> McpHttpProxyPolicy {
    match proxy {
        AgentProxy::System => McpHttpProxyPolicy::System,
        AgentProxy::Disabled => McpHttpProxyPolicy::Disabled,
        AgentProxy::Explicit(address) => McpHttpProxyPolicy::Explicit(address.clone()),
    }
}

impl AgentMcpApplication {
    /// Applies the host's latest proxy settings: when their policy
    /// differs from the one in use, later MCP HTTP and OAuth requests use a
    /// new client, and every enabled server reconnects, concurrently: the
    /// Streamable HTTP ones through the new client, the stdio ones started
    /// anew in the environment the host's processes now get (its proxy
    /// variables). Returns whether the policy changed.
    ///
    /// The settings are read under the MCP configuration gate, so of two
    /// calls the one that runs last applies the latest settings whatever
    /// order they were issued in.
    pub(crate) async fn apply_proxy_settings(
        &self,
        network: &dyn NetworkSettingsPort,
    ) -> Result<bool, String> {
        let _gate = self.mcp_configuration_gate.lock().await;
        let proxy = network.proxy()?;
        self.apply_proxy_policy(&proxy).await
    }

    /// [`Self::apply_proxy_settings`] for `proxy`; the caller holds the MCP
    /// configuration gate.
    pub(crate) async fn apply_proxy_policy(&self, proxy: &AgentProxy) -> Result<bool, String> {
        let policy = mcp_proxy_policy(proxy);
        if *self.lock_proxy() == policy {
            return Ok(false);
        }
        let client = hanafubuki_agent_mcp::build_mcp_http_client(policy.clone())
            .map_err(|error| error.to_string())?;
        self.mcp.set_http_client(client.clone());
        self.mcp_oauth.set_http_client(client);
        *self.lock_proxy() = policy;
        let mcp_settings = std::sync::Arc::new(self.load_settings().await?);
        let servers = mcp_settings
            .servers
            .iter()
            .filter(|server| server.enabled)
            .cloned()
            .collect();
        for (server, outcome) in self.connect_concurrently(servers, mcp_settings).await {
            if let Err(error) = outcome {
                self.mcp.report_connection_failure(server.id, error);
            }
        }
        Ok(true)
    }

    fn lock_proxy(&self) -> parking_lot::MutexGuard<'_, McpHttpProxyPolicy> {
        self.proxy.lock()
    }
}
