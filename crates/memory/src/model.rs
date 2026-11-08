use serde::{Deserialize, Serialize};

pub const MAX_MEMORY_SUMMARY_BYTES: usize = 16 * 1024;
pub const MAX_MEMORY_RESOURCE_BYTES: usize = 128 * 1024;
pub const MAX_MEMORY_SEARCH_RESULTS: usize = 50;
pub const MAX_MEMORY_CITATIONS: usize = 32;
pub const MAX_AD_HOC_MEMORY_NOTE_BYTES: usize = 16 * 1024;
/// The most cited sources one usage update takes.
pub const MAX_USAGE_SOURCES: usize = 64;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentMemoryEvidence {
    pub physical_session_id: String,
    pub source_revision: u64,
    pub updated_at_millis: u128,
    pub task_ids: Vec<u64>,
    pub bounded_text: String,
    pub truncated: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StageOneResult {
    pub raw_memory: String,
    pub rollout_summary: String,
    pub rollout_slug: String,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AgentMemoryModelUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentMemoryModelResult<T> {
    pub value: T,
    pub usage: AgentMemoryModelUsage,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MemoryConsolidationWorkspace {
    pub path: std::path::PathBuf,
    pub generation: u64,
    pub sources: Vec<MemoryWorkspaceSource>,
    pub notes: Vec<AdHocMemoryNote>,
    pub baseline_generation: Option<u64>,
    pub has_semantic_changes: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AdHocMemoryNoteOperation {
    Remember,
    Forget,
    Update,
}

impl AdHocMemoryNoteOperation {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Remember => "remember",
            Self::Forget => "forget",
            Self::Update => "update",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AppendAdHocMemoryNote {
    pub created_at_millis: u64,
    pub session_id: String,
    pub turn_id: u64,
    pub source_revision: u64,
    pub operation: AdHocMemoryNoteOperation,
    pub content: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AdHocMemoryNote {
    pub version: u8,
    pub note_id: String,
    pub created_at_millis: u64,
    pub session_id: String,
    pub turn_id: u64,
    pub source_revision: u64,
    pub operation: AdHocMemoryNoteOperation,
    pub content: String,
    pub reset_epoch: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MemoryWorkspaceSource {
    pub session_id: String,
    pub source_revision: u64,
    pub stage_one_id: String,
    pub raw_memory_sha256: String,
    pub rollout_summary_sha256: String,
    pub rollout_path: String,
    pub rollout_sha256: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MemoryLease {
    pub scope: String,
    pub token: String,
    pub source_revision: u64,
    pub reset_epoch: u64,
    pub expires_at_millis: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentMemoryContext {
    pub generation: u64,
    pub summary: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentMemoryStatus {
    pub enabled: bool,
    pub generation: u64,
    pub source_count: u64,
    pub phase_one_count: u64,
    pub running_jobs: u64,
    pub last_success_at_millis: Option<u64>,
    pub last_error: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentMemoryResource {
    pub id: String,
    pub bytes: usize,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentMemorySearchMatch {
    pub resource_id: String,
    pub line: usize,
    pub excerpt: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentMemoryReadResult {
    pub generation: u64,
    pub resource_id: String,
    pub start_line: usize,
    pub end_line: usize,
    pub content: String,
    pub truncated: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AgentMemoryError {
    InvalidInput(String),
    NotFound,
    StaleRevision,
    LeaseLost,
    ResetInProgress,
    Corrupt(String),
    Io(String),
}

impl std::fmt::Display for AgentMemoryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidInput(message) => write!(f, "invalid Agent memory input: {message}"),
            Self::NotFound => f.write_str("Agent memory resource not found"),
            Self::StaleRevision => f.write_str("stale Agent memory revision"),
            Self::LeaseLost => f.write_str("Agent memory lease is no longer current"),
            Self::ResetInProgress => f.write_str("Agent memory reset is in progress"),
            Self::Corrupt(message) => write!(f, "corrupt Agent memory state: {message}"),
            Self::Io(message) => write!(f, "Agent memory I/O failed: {message}"),
        }
    }
}

impl std::error::Error for AgentMemoryError {}
