//! Filesystem/SQLite implementation of the Agent history store.
//!
//! Per-session JSONL files are canonical. SQLite is a rebuildable projection
//! used for bounded discovery and transcript reads.

mod paths;
mod refusals;
mod retention;
mod store;

pub use paths::AgentHistoryPaths;
pub use refusals::{AgentHistoryRefusalSink, NoopAgentHistoryRefusalSink};
pub use retention::{
    AgentHistoryRetentionCandidate, AgentHistoryRetentionPolicy, AgentHistoryRetentionReport,
};
pub use store::{
    AgentMemoryEvidence, AgentMemoryEvidenceItem, AgentMemoryEvidenceRole,
    AgentMemoryHistoryCandidate, AgentMemoryHistoryQuery, AgentMemorySourceState,
    FileAgentHistoryStore,
};
