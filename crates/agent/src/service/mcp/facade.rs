use crate::service::AgentApplicationService;
use crate::{
    McpApplicationSnapshot, McpCredentialsWriteRequest, McpOAuthCallbackRequest,
    McpOAuthCancelRequest, McpOAuthLoginRequest, McpServerDeleteRequest, McpServerWriteRequest,
};

impl AgentApplicationService {
    pub async fn mcp_snapshot(&self) -> Result<McpApplicationSnapshot, String> {
        self.mcp_application.mcp_snapshot().await
    }

    /// Writes the user's server; one of the same name an installed plugin
    /// runs gives way to it, and the plugins are reloaded in the
    /// background.
    pub async fn mcp_write_server(
        &self,
        request: McpServerWriteRequest,
    ) -> Result<McpApplicationSnapshot, String> {
        let snapshot = self.mcp_application.mcp_write_server(request).await?;
        self.spawn_package_reload();
        Ok(snapshot)
    }

    /// Deletes the user's server; an installed plugin's of the same name
    /// runs again once the plugins are reloaded, in the background.
    pub async fn mcp_delete_server(
        &self,
        request: McpServerDeleteRequest,
    ) -> Result<McpApplicationSnapshot, String> {
        let snapshot = self.mcp_application.mcp_delete_server(request).await?;
        self.spawn_package_reload();
        Ok(snapshot)
    }

    pub async fn mcp_write_credentials(
        &self,
        request: McpCredentialsWriteRequest,
    ) -> Result<McpApplicationSnapshot, String> {
        self.mcp_application.mcp_write_credentials(request).await
    }

    /// Sets the credential `key` to `value`, keeping the others; the
    /// servers that use it reconnect.
    pub async fn mcp_set_credential(
        &self,
        key: hanafubuki_agent_mcp::McpCredentialKey,
        value: String,
    ) -> Result<McpApplicationSnapshot, String> {
        self.mcp_application.mcp_set_credential(key, value).await
    }

    /// Removes the credential `key`, refused while an enabled server uses it.
    pub async fn mcp_remove_credential(
        &self,
        key: hanafubuki_agent_mcp::McpCredentialKey,
    ) -> Result<McpApplicationSnapshot, String> {
        self.mcp_application.mcp_remove_credential(key).await
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

    /// Logs in to the MCP server `server_id` through the browser.
    ///
    /// `open_url` gets the authorization URL to show or open; the login then
    /// waits for the browser's redirect to a loopback port
    /// (`http://127.0.0.1:<port>/callback`) for up to 5 minutes, or until
    /// `cancelled` completes, and cancels the flow if none comes.
    pub async fn mcp_oauth_login(
        &self,
        server_id: hanafubuki_agent_mcp::McpServerId,
        open_url: impl FnOnce(&str) + Send,
        cancelled: impl std::future::Future<Output = ()> + Send,
    ) -> Result<McpApplicationSnapshot, String> {
        self.mcp_application
            .mcp_oauth_login(server_id, open_url, cancelled)
            .await
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
