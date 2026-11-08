//! Tauri-free cross-session Agent memory domain.
//!
//! Session JSONL remains the evidence authority. This crate owns only derived
//! state, durable leases, provenance, bounded read APIs, and atomically
//! published model-readable artifacts. It never receives a general filesystem
//! capability or lets a model write application data.

mod citation;
mod generation;
mod model;
mod paths;
mod redaction;
mod repository;
mod workspace_agent;

pub use citation::{
    parse_memory_citations, AgentMemoryCitation, ParsedMemoryCitations, MEMORY_CITATION_CLOSE,
    MEMORY_CITATION_OPEN,
};
pub use generation::{
    generate_stage_one, AgentMemoryGenerationError, MEMORY_GENERATION_TIMEOUT, PHASE_ONE_TOOL_NAME,
};
pub use hanafubuki_agent_foundation::agent_settings::AgentMemoryContributionMode;
pub use model::{
    AdHocMemoryNote, AdHocMemoryNoteOperation, AgentMemoryContext, AgentMemoryError,
    AgentMemoryEvidence, AgentMemoryModelResult, AgentMemoryModelUsage, AgentMemoryReadResult,
    AgentMemoryResource, AgentMemorySearchMatch, AgentMemoryStatus, AppendAdHocMemoryNote,
    MemoryConsolidationWorkspace, MemoryLease, MemoryWorkspaceSource, StageOneResult,
    MAX_AD_HOC_MEMORY_NOTE_BYTES, MAX_MEMORY_CITATIONS, MAX_MEMORY_RESOURCE_BYTES,
    MAX_MEMORY_SEARCH_RESULTS, MAX_MEMORY_SUMMARY_BYTES, MAX_USAGE_SOURCES,
};
pub use paths::AgentMemoryPaths;
pub use redaction::redact_memory_text;
pub use repository::AgentMemoryRepository;
pub use workspace_agent::run_memory_workspace_agent;
