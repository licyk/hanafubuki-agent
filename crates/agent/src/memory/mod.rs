use std::sync::{mpsc, Arc};
use std::time::Duration;

use hanafubuki_agent_config::AgentConfig;
use hanafubuki_agent_contracts::AgentProcessedMemoryAnswer;
use hanafubuki_agent_foundation::background_worker::BackgroundWorkerGroup;
use hanafubuki_agent_foundation::cancellation::CancellationToken;
use hanafubuki_agent_history::{AgentMemoryHistoryQuery, FileAgentHistoryStore};
use hanafubuki_agent_kernel::{AgentMemoryCitationSnapshot, AgentService, AgentSessionId};
use hanafubuki_agent_memory::{
    generate_stage_one, run_memory_workspace_agent, AgentMemoryContext, AgentMemoryError,
    AgentMemoryRepository, AgentMemoryStatus, MemoryLease,
};

use super::model_api_protocol;
use super::service::resolve_provider_definition;

#[derive(Clone)]
pub struct AgentMemoryApplication {
    config: AgentConfig,
    /// The kernel, for the context window of the model a job samples with.
    core: AgentService,
    repository: AgentMemoryRepository,
    work_sender: mpsc::SyncSender<MemoryWork>,
    /// Set once a scan signal found the supervisor gone, so that is warned
    /// about once; shared by every clone.
    supervisor_gone: Arc<std::sync::atomic::AtomicBool>,
    /// Scan signals sent so far, shared by every clone.
    #[cfg(test)]
    scan_signals: Arc<std::sync::atomic::AtomicUsize>,
}

enum MemoryWork {
    Scan(Option<AgentSessionId>),
    RecordCitationUsage { source_session_ids: Vec<String> },
}

pub(crate) fn memory_stream_visible_prefix_len(text: &str) -> usize {
    let marker = hanafubuki_agent_memory::MEMORY_CITATION_OPEN;
    if let Some(start) = text.find(marker) {
        return text[..start].trim_end().len();
    }
    let partial_marker_len = (1..marker.len())
        .rev()
        .find(|length| text.ends_with(&marker[..*length]))
        .unwrap_or(0);
    text[..text.len() - partial_marker_len].trim_end().len()
}

mod application;
mod evidence;
mod step;
mod supervisor;
