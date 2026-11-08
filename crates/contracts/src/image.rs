use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentImageDetail {
    Low,
    High,
    #[default]
    Auto,
}

impl From<AgentImageDetail> for hanafubuki_agent_tool_contracts::ToolOutputImageDetail {
    fn from(value: AgentImageDetail) -> Self {
        match value {
            AgentImageDetail::Low => Self::Low,
            AgentImageDetail::High => Self::High,
            AgentImageDetail::Auto => Self::Auto,
        }
    }
}

/// An image a tool or the user attached, normalized and ready to be staged
/// into history.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PreparedAgentImage {
    pub bytes: Vec<u8>,
    pub mime_type: String,
    pub width: u32,
    pub height: u32,
    pub display_name: String,
    pub detail: AgentImageDetail,
}
