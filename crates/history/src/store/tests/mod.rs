use std::fs::{self, OpenOptions};
use std::io::Write;
use std::sync::Arc;
use std::time::{Duration, Instant};

use hanafubuki_agent_foundation::background_worker::BackgroundWorkerGroup;
use hanafubuki_agent_kernel::{
    AgentCompactionCheckpoint, AgentCompactionGeneration, AgentCompactionId,
    AgentCompactionImplementation, AgentCompactionPhase, AgentCompactionReason,
    AgentCompactionSnapshot, AgentCompactionStatus, AgentCompactionTrigger,
    AgentContextWindowSnapshot, AgentConversationCommit, AgentConversationDelta, AgentGeneration,
    AgentHistoryItem, AgentHistoryItemId, AgentHistoryMutation, AgentHistorySortDirection,
    AgentHistoryStore, AgentModelContextBoundary, AgentRevision, AgentSessionConfig,
    AgentSessionId, AgentSessionLifecycle, AgentSessionListRequest, AgentSessionMode,
    AgentToolOutputTruncation, AgentTranscriptPageRequest, AgentTurnBudgetPolicy, AgentTurnId,
    AgentTurnPageRequest, ConversationItem, PersistedAgentRuntimeState,
};

use super::jsonl::collect_jsonl_files;
use super::lifecycle::deletion::{write_delete_obligation, DeleteObligation};
use super::{AgentHistoryPaths, AgentHistoryRetentionPolicy, FileAgentHistoryStore};

fn runtime(
    session_id: AgentSessionId,
    revision: u64,
    next_history_item_id: u64,
) -> PersistedAgentRuntimeState {
    PersistedAgentRuntimeState {
        id: session_id,
        forked_from_session_id: None,
        parent_session_id: None,
        wake_queue: Default::default(),
        origin: None,
        cwd: None,
        branch: None,
        title: None,
        goal: None,
        generation: AgentGeneration::new(0),
        revision: AgentRevision::new(revision),
        config: AgentSessionConfig {
            provider_id: "provider-1".to_string(),
            provider_protocol: hanafubuki_agent_kernel::ModelApiProtocol::Responses,
            model: "fixture".to_string(),
            instructions: "test".to_string(),
            mode: AgentSessionMode::Default,
            reasoning_effort: None,
            context_window_tokens: None,
            input_capabilities: Default::default(),
            auto_compaction_enabled: true,
            auto_compact_threshold_percent: 80,
            compaction_capability: hanafubuki_agent_kernel::ModelCompactionCapability::LocalSummary,
            sampling_retry_policy: hanafubuki_agent_kernel::AgentSamplingRetryPolicy::default(),
            context_window_source: None,
            compaction_strategy: Default::default(),
            scope: Default::default(),
            configured_facts: Default::default(),
            role_instructions: None,
        },
        memory_contribution: hanafubuki_agent_kernel::AgentMemoryContributionState::default(),
        turn_budget_policy: AgentTurnBudgetPolicy::default(),
        lifecycle: AgentSessionLifecycle::Available,
        usage: None,
        context_window: AgentContextWindowSnapshot::initial(),
        compaction: None,
        compression: None,
        compression_update: None,
        turn: None,
        activities: Vec::new(),
        task_plan: None,
        pending_user_input: None,
        next_message_id: 1,
        next_history_item_id,
        model_context_boundary: AgentModelContextBoundary::Full,
        history_base: None,
        created_at: 1_800_000_000_000,
        updated_at: 1_800_000_000_000 + u128::from(revision),
    }
}

fn item(id: u64, revision: u64, text: &str) -> AgentHistoryItem {
    AgentHistoryItem {
        source_session_id: AgentSessionId::new(1),
        item_id: AgentHistoryItemId::new(id),
        item_revision: revision,
        turn_id: None,
        logical_segment: 0,
        logical_order: 0,
        turn_working_millis: None,
        content: ConversationItem::UserMessage {
            text: text.to_string(),
            images: Vec::new(),
        },
    }
}

fn turn_start(id: u64, turn_id: u64) -> AgentHistoryItem {
    AgentHistoryItem {
        source_session_id: AgentSessionId::new(1),
        item_id: AgentHistoryItemId::new(id),
        item_revision: 0,
        turn_id: Some(AgentTurnId::new(turn_id)),
        logical_segment: 0,
        logical_order: 0,
        turn_working_millis: None,
        content: ConversationItem::TurnStart {
            turn_id: AgentTurnId::new(turn_id),
        },
    }
}

fn truncated_tool_output(
    source_session_id: AgentSessionId,
    id: u64,
    revision: u64,
) -> AgentHistoryItem {
    AgentHistoryItem {
        source_session_id,
        item_id: AgentHistoryItemId::new(id),
        item_revision: revision,
        turn_id: Some(AgentTurnId::new(1)),
        logical_segment: 0,
        logical_order: 0,
        turn_working_millis: None,
        content: ConversationItem::ToolOutput {
            call_id: "large-output-call".to_string(),
            origin: Default::default(),
            output: serde_json::from_value(serde_json::json!({
                "content": [{
                    "type": "text",
                    "text": "HEAD\n… <7000 bytes omitted> …\nTAIL\n<hanafubuki-tool-output-truncation>{\"originalBytes\":8192,\"retainedBytes\":256,\"omittedBytes\":7936,\"omittedItems\":1,\"omittedContentTypes\":[\"text\"]}</hanafubuki-tool-output-truncation>"
                }],
                "structuredContent": null,
                "isError": true
            }))
            .unwrap(),
            truncation: Some(AgentToolOutputTruncation {
                original_bytes: 8_192,
                retained_bytes: 256,
                omitted_bytes: 7_936,
                omitted_items: 1,
                omitted_content_types: vec!["text".to_string()],
            }),
            images: Vec::new(),
            tool_search_results: Vec::new(),
            display_metadata: Default::default(),
        },
    }
}

fn open_store() -> (
    tempfile::TempDir,
    AgentHistoryPaths,
    BackgroundWorkerGroup,
    Arc<FileAgentHistoryStore>,
) {
    let temporary = tempfile::tempdir().unwrap();
    let paths = AgentHistoryPaths::under(temporary.path().join("agent"));
    let workers = BackgroundWorkerGroup::default();
    let store = Arc::new(FileAgentHistoryStore::open(paths.clone(), workers.clone()).unwrap());
    (temporary, paths, workers, store)
}

mod attachments;
mod context_retention;
mod kept_dependents;
mod lifecycle;
mod memory;
mod paging;
mod projection;
mod quarantine;
mod queued;
mod recent_messages;
