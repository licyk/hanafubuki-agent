use hanafubuki_agent_kernel::{AgentSessionId, AgentSessionLifecycle};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AgentHistoryRetentionPolicy {
    pub automatic_deletion_enabled: bool,
    pub maximum_total_bytes: Option<u64>,
    pub maximum_archived_sessions: Option<usize>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentHistoryRetentionCandidate {
    pub session_id: AgentSessionId,
    pub lifecycle: AgentSessionLifecycle,
    pub updated_at: u128,
    pub physical_bytes: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentHistoryRetentionReport {
    pub automatic_deletion_enabled: bool,
    pub total_physical_bytes: u64,
    pub session_count: usize,
    pub archived_session_count: usize,
    pub candidates: Vec<AgentHistoryRetentionCandidate>,
}
