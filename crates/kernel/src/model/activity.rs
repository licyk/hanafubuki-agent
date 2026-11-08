use hanafubuki_agent_contracts::{AgentActivityPresentation, AgentExecutionProgressStatus};
use serde::{Deserialize, Serialize};

use super::AgentActivityId;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentActivityKind {
    Sampling,
    ToolInvocation,
    Approval,
    ContinuationHook,
    Execution,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentActivityPhase {
    Running,
    WaitingForApproval,
    WaitingForCleanup,
    WaitingForUserInput,
    Succeeded,
    Failed,
    Cancelled,
    Interrupted,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentExecutionActivityRef {
    pub execution_id: u64,
    pub generation: u64,
    pub status: AgentExecutionProgressStatus,
    pub exit_code: Option<i32>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentActivityOutputPreview {
    pub head: Vec<String>,
    pub tail: Vec<String>,
    pub omitted_lines: u64,
    pub next_offset: u64,
    pub truncated: bool,
}

impl AgentActivityPhase {
    pub const fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Succeeded | Self::Failed | Self::Cancelled | Self::Interrupted
        )
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentActivitySnapshot {
    pub id: AgentActivityId,
    pub revision: u64,
    pub kind: AgentActivityKind,
    pub phase: AgentActivityPhase,
    pub presentation: AgentActivityPresentation,
    pub attempt: Option<u32>,
    pub started_at: u128,
    pub updated_at: u128,
    pub finished_at: Option<u128>,
    pub execution: Option<AgentExecutionActivityRef>,
    pub output: Option<AgentActivityOutputPreview>,
}

#[cfg(test)]
mod tests;
