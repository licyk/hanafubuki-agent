use super::AgentMcpApplication;
use crate::McpApplicationSnapshot;

impl AgentMcpApplication {
    pub async fn mcp_connect(
        &self,
        server_id: hanafubuki_agent_mcp::McpServerId,
    ) -> Result<McpApplicationSnapshot, String> {
        let settings = self.load_settings().await?;
        let server = self
            .server_config(&settings, &server_id)
            .ok_or_else(|| "MCP server does not exist".to_string())?;
        self.connect_mcp_server(&server, &settings).await?;
        self.mcp_snapshot_with_settings(settings).await
    }

    pub async fn mcp_disconnect(
        &self,
        server_id: hanafubuki_agent_mcp::McpServerId,
    ) -> Result<McpApplicationSnapshot, String> {
        self.mcp
            .disconnect(&server_id)
            .await
            .map_err(|error| error.to_string())?;
        self.mcp_snapshot().await
    }

    pub async fn mcp_refresh(
        &self,
        server_id: hanafubuki_agent_mcp::McpServerId,
    ) -> Result<McpApplicationSnapshot, String> {
        self.mcp_connect(server_id).await
    }
}
