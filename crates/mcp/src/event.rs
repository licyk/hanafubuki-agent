use serde::{Deserialize, Serialize};

use crate::McpServerId;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum McpEventKind {
    ServerStatus,
    ToolCatalog,
    Elicitation,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpEventNotice {
    pub server_id: McpServerId,
    pub kind: McpEventKind,
}

pub trait McpEventSink: Send + Sync {
    fn emit(&self, notice: McpEventNotice);
}

#[derive(Default)]
pub struct NoopMcpEventSink;

impl McpEventSink for NoopMcpEventSink {
    fn emit(&self, _notice: McpEventNotice) {}
}
