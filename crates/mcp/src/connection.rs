use serde::{Deserialize, Serialize};

use crate::{
    McpCapabilitySnapshot, McpCatalogGeneration, McpConnectionGeneration, McpResolvedProgram,
    McpServerId,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum McpConnectionPhase {
    Disabled,
    Starting,
    Ready,
    Refreshing,
    Stopping,
    Failed,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpServerStatus {
    pub server_id: McpServerId,
    pub phase: McpConnectionPhase,
    pub connection_generation: Option<McpConnectionGeneration>,
    pub catalog_generation: Option<McpCatalogGeneration>,
    pub tool_count: usize,
    pub resource_count: usize,
    pub resource_template_count: usize,
    pub resolved_program: Option<McpResolvedProgram>,
    pub capabilities: Option<McpCapabilitySnapshot>,
    pub cleanup_pending: bool,
    pub last_error: Option<String>,
    /// Set while the server is `Failed` after its connection closed.
    pub reconnect: Option<McpReconnectStatus>,
}

/// The automatic reconnects of a server whose connection closed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpReconnectStatus {
    /// The number of the reconnect that is scheduled, counted in the
    /// window; `None` once the window's reconnects are used up.
    pub next_attempt: Option<usize>,
    /// How many reconnects the window allows.
    pub maximum: usize,
}

impl McpServerStatus {
    pub fn disabled(server_id: McpServerId) -> Self {
        Self {
            server_id,
            phase: McpConnectionPhase::Disabled,
            connection_generation: None,
            catalog_generation: None,
            tool_count: 0,
            resource_count: 0,
            resource_template_count: 0,
            resolved_program: None,
            capabilities: None,
            cleanup_pending: false,
            last_error: None,
            reconnect: None,
        }
    }
}
