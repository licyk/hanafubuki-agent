use crate::service::AgentApplicationService;
use crate::{
    McpApplicationSnapshot, McpCredentialsWriteRequest, McpOAuthCallbackRequest,
    McpOAuthCancelRequest, McpOAuthLoginRequest, McpServerDeleteRequest, McpServerWriteRequest,
};

impl AgentApplicationService {
    pub async fn mcp_snapshot(&self) -> Result<McpApplicationSnapshot, String> {
        self.mcp_application.mcp_snapshot().await
    }

    pub async fn mcp_write_server(
        &self,
        request: McpServerWriteRequest,
    ) -> Result<McpApplicationSnapshot, String> {
        self.mcp_application.mcp_write_server(request).await
    }

    pub async fn mcp_delete_server(
        &self,
        request: McpServerDeleteRequest,
    ) -> Result<McpApplicationSnapshot, String> {
        self.mcp_application.mcp_delete_server(request).await
    }

    pub async fn mcp_write_credentials(
        &self,
        request: McpCredentialsWriteRequest,
    ) -> Result<McpApplicationSnapshot, String> {
        self.mcp_application.mcp_write_credentials(request).await
    }

    pub async fn mcp_connect(
        &self,
        server_id: hanafubuki_agent_mcp::McpServerId,
    ) -> Result<McpApplicationSnapshot, String> {
        self.mcp_application.mcp_connect(server_id).await
    }

    pub async fn mcp_disconnect(
        &self,
        server_id: hanafubuki_agent_mcp::McpServerId,
    ) -> Result<McpApplicationSnapshot, String> {
        self.mcp_application.mcp_disconnect(server_id).await
    }

    pub async fn mcp_refresh(
        &self,
        server_id: hanafubuki_agent_mcp::McpServerId,
    ) -> Result<McpApplicationSnapshot, String> {
        self.mcp_application.mcp_refresh(server_id).await
    }

    pub async fn start_mcp_servers(&self) -> Result<Vec<String>, String> {
        self.mcp_application.start_servers().await
    }

    pub async fn mcp_oauth_start(
        &self,
        request: McpOAuthLoginRequest,
    ) -> Result<hanafubuki_agent_mcp::McpOAuthLogin, String> {
        self.mcp_application.mcp_oauth_start(request).await
    }

    pub async fn mcp_oauth_complete(
        &self,
        request: McpOAuthCallbackRequest,
    ) -> Result<McpApplicationSnapshot, String> {
        self.mcp_application.mcp_oauth_complete(request).await
    }

    pub async fn mcp_oauth_cancel(
        &self,
        request: McpOAuthCancelRequest,
    ) -> Result<McpApplicationSnapshot, String> {
        self.mcp_application.mcp_oauth_cancel(request).await
    }

    pub async fn mcp_oauth_logout(
        &self,
        server_id: hanafubuki_agent_mcp::McpServerId,
    ) -> Result<McpApplicationSnapshot, String> {
        self.mcp_application.mcp_oauth_logout(server_id).await
    }

    pub async fn mcp_oauth_refresh(
        &self,
        server_id: hanafubuki_agent_mcp::McpServerId,
    ) -> Result<McpApplicationSnapshot, String> {
        self.mcp_application.mcp_oauth_refresh(server_id).await
    }

    pub fn mcp_resolve_elicitation(
        &self,
        request_id: hanafubuki_agent_mcp::McpElicitationRequestId,
        decision: hanafubuki_agent_mcp::McpElicitationDecision,
    ) -> Result<(), String> {
        self.mcp_application
            .mcp_resolve_elicitation(request_id, decision)
    }
}
