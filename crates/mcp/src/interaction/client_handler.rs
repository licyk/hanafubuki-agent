//! The rmcp client handler: client identity, elicitation requests, and tool-list change notices.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use rmcp::model::{ClientConfig, ElicitRequestParams, ElicitResult, ProtocolVersion};
use rmcp::service::{NotificationContext, RequestContext, RoleClient};
use rmcp::ClientHandler;

use super::McpInteractionHub;
use crate::{McpConnectionGeneration, McpServerId};

#[derive(Clone)]
pub(crate) struct McpClientHandler {
    server_id: McpServerId,
    connection_generation: McpConnectionGeneration,
    elicitation_timeout: Duration,
    interactions: McpInteractionHub,
    tools_changed: Arc<AtomicBool>,
    protocol_mode: crate::McpProtocolMode,
}

impl McpClientHandler {
    pub fn new(
        server_id: McpServerId,
        connection_generation: McpConnectionGeneration,
        elicitation_timeout: Duration,
        interactions: McpInteractionHub,
        protocol_mode: crate::McpProtocolMode,
    ) -> Self {
        Self {
            server_id,
            connection_generation,
            elicitation_timeout,
            interactions,
            tools_changed: Arc::new(AtomicBool::new(false)),
            protocol_mode,
        }
    }

    pub fn tools_changed(&self) -> bool {
        self.tools_changed.load(Ordering::Acquire)
    }

    pub fn clear_tools_changed(&self) {
        self.tools_changed.store(false, Ordering::Release);
    }
}

impl ClientHandler for McpClientHandler {
    fn get_info(&self) -> ClientConfig {
        let mut info = ClientConfig::default();
        info.protocol_version = match self.protocol_mode {
            crate::McpProtocolMode::Legacy => ProtocolVersion::V_2025_06_18,
            crate::McpProtocolMode::V20260728 => ProtocolVersion::V_2026_07_28,
        };
        info
    }

    async fn create_elicitation(
        &self,
        request: ElicitRequestParams,
        _context: RequestContext<RoleClient>,
    ) -> Result<ElicitResult, rmcp::model::ErrorData> {
        Ok(self
            .interactions
            .request(
                self.server_id.clone(),
                self.connection_generation,
                request,
                self.elicitation_timeout,
            )
            .await)
    }

    async fn on_tool_list_changed(&self, _context: NotificationContext<RoleClient>) {
        self.tools_changed.store(true, Ordering::Release);
    }
}
