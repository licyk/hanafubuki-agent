use super::{credential_references, AgentMcpApplication};

impl AgentMcpApplication {
    pub(super) async fn reconcile_mcp_server(
        &self,
        server: &hanafubuki_agent_mcp::McpServerConfig,
        settings: &hanafubuki_agent_mcp::McpSettings,
    ) -> Result<(), String> {
        if server.enabled {
            self.connect_mcp_server(server, settings).await
        } else {
            self.mcp
                .disconnect(&server.id)
                .await
                .map_err(|error| error.to_string())
        }
    }

    /// Connects or refreshes `server`, observing the MCP lifetime: shutdown
    /// cancels a connect under way, which disposes the process it spawned.
    pub(super) async fn connect_mcp_server(
        &self,
        server: &hanafubuki_agent_mcp::McpServerConfig,
        settings: &hanafubuki_agent_mcp::McpSettings,
    ) -> Result<(), String> {
        if !server.enabled {
            return Err("MCP server is disabled".into());
        }
        let mut secrets =
            hanafubuki_agent_mcp::resolve_connection_secrets(server, &settings.credentials)
                .map_err(|error| error.to_string())?;
        if matches!(
            &server.transport,
            hanafubuki_agent_mcp::McpTransportConfig::StreamableHttp(http)
                if http.oauth != hanafubuki_agent_mcp::McpOAuthPolicy::Disabled
        ) {
            if let Some(token) = self
                .mcp_oauth
                .access_token(server)
                .await
                .map_err(|error| error.to_string())?
            {
                secrets = secrets
                    .with_oauth_bearer(token, self.mcp_oauth.status(server).credential_generation)
                    .map_err(|error| error.to_string())?;
            }
        }
        self.mcp
            .connect_or_refresh(server.clone(), secrets, self.lifetime.token())
            .await
            .map(|_| ())
            .map_err(|error| error.to_string())
    }

    pub(super) async fn reconnect_credential_consumers(
        &self,
        key: &hanafubuki_agent_mcp::McpCredentialKey,
        settings: &hanafubuki_agent_mcp::McpSettings,
    ) {
        let consumers = settings
            .servers
            .iter()
            .filter(|server| server.enabled && credential_references(server).contains(key))
            .cloned()
            .collect();
        for (server, outcome) in self
            .connect_concurrently(consumers, std::sync::Arc::new(settings.clone()))
            .await
        {
            if let Err(error) = outcome {
                self.mcp.report_connection_failure(server.id, error);
            }
        }
    }
}
