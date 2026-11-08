use super::AgentMcpApplication;
use crate::{
    McpApplicationSnapshot, McpOAuthCallbackRequest, McpOAuthCancelRequest, McpOAuthLoginRequest,
};

impl AgentMcpApplication {
    pub async fn mcp_oauth_start(
        &self,
        request: McpOAuthLoginRequest,
    ) -> Result<hanafubuki_agent_mcp::McpOAuthLogin, String> {
        let settings = self.load_settings().await?;
        let server = settings
            .server(&request.server_id)
            .ok_or_else(|| "MCP server does not exist".to_string())?;
        match self
            .mcp_oauth
            .start_login(server, request.redirect_uri)
            .await
        {
            Ok(login) => {
                self.mcp.record_diagnostic(
                    server,
                    hanafubuki_agent_mcp::McpDiagnosticStage::OAuth,
                    hanafubuki_agent_mcp::McpDiagnosticOutcome::Success,
                    "oauth_authorization_started",
                    "OAuth authorization handoff started",
                    false,
                    "complete_oauth",
                );
                Ok(login)
            }
            Err(error) => {
                self.mcp.record_diagnostic(
                    server,
                    hanafubuki_agent_mcp::McpDiagnosticStage::OAuth,
                    hanafubuki_agent_mcp::McpDiagnosticOutcome::Error,
                    "oauth_authorization_failed",
                    "OAuth authorization handoff failed",
                    true,
                    "inspect_diagnostics",
                );
                Err(error.to_string())
            }
        }
    }

    pub async fn mcp_oauth_complete(
        &self,
        request: McpOAuthCallbackRequest,
    ) -> Result<McpApplicationSnapshot, String> {
        let settings = self.load_settings().await?;
        let server = settings
            .server(&request.server_id)
            .ok_or_else(|| "MCP server does not exist".to_string())?;
        if let Err(error) = self
            .mcp_oauth
            .complete_login(server, request.flow_id, request.callback_url)
            .await
        {
            self.mcp.record_diagnostic(
                server,
                hanafubuki_agent_mcp::McpDiagnosticStage::OAuth,
                hanafubuki_agent_mcp::McpDiagnosticOutcome::Error,
                "oauth_callback_failed",
                "OAuth callback validation or token exchange failed",
                true,
                "restart_oauth",
            );
            return Err(error.to_string());
        }
        self.mcp.record_diagnostic(
            server,
            hanafubuki_agent_mcp::McpDiagnosticStage::OAuth,
            hanafubuki_agent_mcp::McpDiagnosticOutcome::Success,
            "oauth_authorized",
            "OAuth authorization completed",
            false,
            "connect",
        );
        if let Err(error) = self.connect_mcp_server(server, &settings).await {
            self.mcp.report_connection_failure(server.id.clone(), error);
        }
        self.mcp_snapshot_with_settings(settings).await
    }

    pub async fn mcp_oauth_cancel(
        &self,
        request: McpOAuthCancelRequest,
    ) -> Result<McpApplicationSnapshot, String> {
        let settings = self.load_settings().await?;
        let server = settings
            .server(&request.server_id)
            .ok_or_else(|| "MCP server does not exist".to_string())?;
        self.mcp_oauth
            .cancel_login(server, request.flow_id)
            .map_err(|error| error.to_string())?;
        self.mcp_snapshot_with_settings(settings).await
    }

    pub async fn mcp_oauth_logout(
        &self,
        server_id: hanafubuki_agent_mcp::McpServerId,
    ) -> Result<McpApplicationSnapshot, String> {
        let settings = self.load_settings().await?;
        let server = settings
            .server(&server_id)
            .ok_or_else(|| "MCP server does not exist".to_string())?;
        self.mcp
            .disconnect(&server.id)
            .await
            .map_err(|error| error.to_string())?;
        self.mcp_oauth
            .logout(server)
            .await
            .map_err(|error| error.to_string())?;
        self.mcp.record_diagnostic(
            server,
            hanafubuki_agent_mcp::McpDiagnosticStage::OAuth,
            hanafubuki_agent_mcp::McpDiagnosticOutcome::Success,
            "oauth_logged_out",
            "OAuth material was deleted after confirmed disconnect",
            false,
            "none",
        );
        self.mcp_snapshot_with_settings(settings).await
    }

    pub async fn mcp_oauth_refresh(
        &self,
        server_id: hanafubuki_agent_mcp::McpServerId,
    ) -> Result<McpApplicationSnapshot, String> {
        let settings = self.load_settings().await?;
        let server = settings
            .server(&server_id)
            .ok_or_else(|| "MCP server does not exist".to_string())?;
        if let Err(error) = self.mcp_oauth.refresh(server).await {
            self.mcp.record_diagnostic(
                server,
                hanafubuki_agent_mcp::McpDiagnosticStage::OAuth,
                hanafubuki_agent_mcp::McpDiagnosticOutcome::Error,
                "oauth_refresh_failed",
                "OAuth refresh failed; previously stored material was retained",
                true,
                "inspect_diagnostics",
            );
            return Err(error.to_string());
        }
        self.mcp.record_diagnostic(
            server,
            hanafubuki_agent_mcp::McpDiagnosticStage::OAuth,
            hanafubuki_agent_mcp::McpDiagnosticOutcome::Success,
            "oauth_refreshed",
            "OAuth material was refreshed",
            false,
            "connect",
        );
        if let Err(error) = self.connect_mcp_server(server, &settings).await {
            self.mcp.report_connection_failure(server.id.clone(), error);
        }
        self.mcp_snapshot_with_settings(settings).await
    }

    pub fn mcp_resolve_elicitation(
        &self,
        request_id: hanafubuki_agent_mcp::McpElicitationRequestId,
        decision: hanafubuki_agent_mcp::McpElicitationDecision,
    ) -> Result<(), String> {
        self.mcp
            .resolve_elicitation(request_id, decision)
            .map_err(|error| error.to_string())
    }
}
