use std::collections::{HashMap, HashSet};
use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use hanafubuki_agent_foundation::background_worker::BackgroundWorkerGroup;
use hanafubuki_agent_foundation::persistence::discard_sqlite;
use hanafubuki_agent_kernel::{
    AgentCompressionChain, AgentCompressionUpdate, AgentConversationCommit, AgentConversationDelta,
    AgentHistoryAppendAck, AgentHistoryBase, AgentHistoryItem, AgentHistoryMutation,
    AgentHistoryRecord, AgentHistoryRecordPayload, AgentHistorySortDirection, AgentHistoryStore,
    AgentHistoryStoreError, AgentModelContextBoundary, AgentRecoveredSession,
    AgentRecoveryLoadFailure, AgentRevision, AgentSessionId, AgentSessionLifecycle,
    AgentSessionListRequest, AgentSessionPage, AgentSessionSummary, AgentTranscriptPage,
    AgentTranscriptPageRequest, AgentTurnId, AgentTurnPageRequest, AgentTurnPhase,
    ConversationItem, AGENT_HISTORY_FORMAT_VERSION, AGENT_SESSION_PAGE_SIZE_MAX,
    AGENT_TRANSCRIPT_PAGE_SIZE_MAX,
};
use parking_lot::Mutex;
use rusqlite::{
    params, params_from_iter, types::Value, Connection, OptionalExtension, TransactionBehavior,
};
use serde::{Deserialize, Serialize};

use crate::paths::ensure_scoped_path;
use crate::{
    AgentHistoryPaths, AgentHistoryRefusalSink, AgentHistoryRetentionCandidate,
    AgentHistoryRetentionPolicy, AgentHistoryRetentionReport,
};

mod attachments;
mod content_store;
mod exports;
mod filesystem;
mod jsonl;
mod kept_files;
mod lifecycle;
mod memory;
mod projection;
mod query;
mod retention;
mod sweep;
mod writer;

use projection::ProjectionLag;
use writer::WriterHandle;

use filesystem::{publish_file, sync_directory};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AgentMemoryHistoryQuery {
    pub current_session_id: Option<AgentSessionId>,
    pub now_millis: u128,
    pub minimum_idle_millis: u64,
    pub maximum_age_millis: u64,
    pub limit: usize,
    /// Continue after this candidate of an earlier page, in the same
    /// newest-first order.
    pub older_than: Option<AgentMemoryHistoryCandidate>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AgentMemoryHistoryCandidate {
    pub session_id: AgentSessionId,
    pub source_revision: AgentRevision,
    pub updated_at: u128,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AgentMemorySourceState {
    pub session_id: AgentSessionId,
    pub source_revision: AgentRevision,
    pub contribution: hanafubuki_agent_kernel::AgentMemoryContributionState,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AgentMemoryEvidenceRole {
    User,
    Assistant,
    Steering,
    Tool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentMemoryEvidenceItem {
    pub item_id: hanafubuki_agent_kernel::AgentHistoryItemId,
    pub turn_id: Option<AgentTurnId>,
    pub role: AgentMemoryEvidenceRole,
    pub text: String,
    pub tool_name: Option<String>,
    pub tool_arguments: Option<serde_json::Value>,
    pub call_id: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentMemoryEvidence {
    pub session_id: AgentSessionId,
    pub source_revision: AgentRevision,
    pub updated_at: u128,
    pub items: Vec<AgentMemoryEvidenceItem>,
    pub truncated: bool,
}

#[derive(Clone)]
pub struct FileAgentHistoryStore {
    inner: Arc<StoreInner>,
}

struct StoreInner {
    paths: AgentHistoryPaths,
    workers: BackgroundWorkerGroup,
    writers: Mutex<HashMap<AgentSessionId, WriterHandle>>,
    writer_gates: Mutex<HashMap<AgentSessionId, Arc<Mutex<()>>>>,
    database_gate: Mutex<()>,
    /// Sessions whose index projection cannot be served, with why: an
    /// append whose projection failed, or a session the index could not
    /// admit (a rollout `open` could not project or moved aside, a misplaced
    /// file, an item or attachment that does not read, a summary the session
    /// list cannot decode). Their transcript and runtime reads are refused,
    /// recovery lists them as quarantined, and no attachment is swept while
    /// any is listed.
    projection_lag: Mutex<HashMap<AgentSessionId, ProjectionLag>>,
    /// The attachment digests the rollouts moved into `.recovery` name, read
    /// at open; `None` until then, or when one could not be read, and then
    /// no attachment is swept.
    quarantined_references: Mutex<Option<Arc<HashSet<hanafubuki_agent_kernel::AgentAttachmentId>>>>,
    /// Told of a session refused after open.
    refusals: Arc<dyn AgentHistoryRefusalSink>,
    accepting: AtomicBool,
    /// Makes the next append panic on its writer.
    #[cfg(test)]
    panic_next_append: AtomicBool,
}

pub(in crate::store) struct PendingHistoryCommit {
    session_id: AgentSessionId,
    session_revision: AgentRevision,
    session_updated_at: u128,
    payload: AgentHistoryRecordPayload,
}

impl PendingHistoryCommit {
    fn checkpoint(commit: AgentConversationCommit) -> Self {
        Self {
            session_id: commit.runtime.id,
            session_revision: commit.runtime.revision,
            session_updated_at: commit.runtime.updated_at,
            payload: AgentHistoryRecordPayload {
                runtime_checkpoint: Some(commit.runtime),
                history_mutations: commit.history_mutations,
            },
        }
    }

    fn delta(delta: AgentConversationDelta) -> Self {
        Self {
            session_id: delta.session_id,
            session_revision: delta.session_revision,
            session_updated_at: delta.session_updated_at,
            payload: AgentHistoryRecordPayload {
                runtime_checkpoint: None,
                history_mutations: delta.history_mutations,
            },
        }
    }
}

impl FileAgentHistoryStore {
    /// Every session's memory contribution. A session whose runtime does not
    /// decode is left out (it contributes nothing until it can be read);
    /// recovery quarantines it, and its row is not touched.
    pub fn list_memory_source_states(
        &self,
    ) -> Result<Vec<AgentMemorySourceState>, AgentHistoryStoreError> {
        let _database = self.inner.database_gate.lock();
        let connection = self.connection()?;
        let mut statement = connection
            .prepare("SELECT session_id, runtime_json FROM agent_sessions ORDER BY session_id")
            .map_err(database_error)?;
        let rows = statement
            .query_map([], |row| {
                Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(database_error)?;
        let mut states = Vec::new();
        for row in rows {
            let (session_id, runtime) = row.map_err(database_error)?;
            let runtime: hanafubuki_agent_kernel::PersistedAgentRuntimeState =
                match serde_json::from_str(&runtime) {
                    Ok(runtime) => runtime,
                    Err(error) => {
                        log::warn!(
                            "Agent session {session_id} is left out of memory sources: its runtime does not decode: {error}"
                        );
                        continue;
                    }
                };
            states.push(AgentMemorySourceState {
                session_id: runtime.id,
                source_revision: runtime.revision,
                contribution: runtime.memory_contribution,
            });
        }
        Ok(states)
    }

    /// [`Self::open`] with the SQLite index discarded first, so it is
    /// rebuilt from the JSONL rollouts, which stay the durable history: the
    /// user's repair when the index is wrong in a way open does not detect.
    pub fn open_rebuilding_index(
        paths: AgentHistoryPaths,
        workers: BackgroundWorkerGroup,
        refusals: Arc<dyn AgentHistoryRefusalSink>,
    ) -> Result<Self, AgentHistoryStoreError> {
        log::warn!(
            "rebuilding the Agent history index from the rollouts path={}",
            paths.sqlite_database.display()
        );
        discard_sqlite(&paths.sqlite_database).map_err(AgentHistoryStoreError::Io)?;
        Self::open_reporting_refusals(paths, workers, refusals)
    }

    /// [`Self::open_reporting_refusals`] whose later refusals are only
    /// logged.
    pub fn open(
        paths: AgentHistoryPaths,
        workers: BackgroundWorkerGroup,
    ) -> Result<Self, AgentHistoryStoreError> {
        Self::open_reporting_refusals(paths, workers, Arc::new(crate::NoopAgentHistoryRefusalSink))
    }

    /// Opens the store; a session it refuses once open (the session list
    /// cannot decode its row) is told to `refusals`. What open itself
    /// refuses goes to recovery (`load_recovery_candidates`).
    pub fn open_reporting_refusals(
        paths: AgentHistoryPaths,
        workers: BackgroundWorkerGroup,
        refusals: Arc<dyn AgentHistoryRefusalSink>,
    ) -> Result<Self, AgentHistoryStoreError> {
        let database_existed = paths.sqlite_database.exists();
        fs::create_dir_all(&paths.active_sessions).map_err(io_error)?;
        fs::create_dir_all(&paths.archived_sessions).map_err(io_error)?;
        fs::create_dir_all(&paths.trash).map_err(io_error)?;
        fs::create_dir_all(&paths.attachments).map_err(io_error)?;
        let store = Self {
            inner: Arc::new(StoreInner {
                paths,
                workers,
                writers: Mutex::new(HashMap::new()),
                writer_gates: Mutex::new(HashMap::new()),
                database_gate: Mutex::new(()),
                projection_lag: Mutex::new(HashMap::new()),
                quarantined_references: Mutex::new(None),
                refusals,
                accepting: AtomicBool::new(true),
                #[cfg(test)]
                panic_next_append: AtomicBool::new(false),
            }),
        };
        store.cleanup_attachment_staging()?;
        store.recover_delete_obligations()?;
        let initialize = || -> Result<(), AgentHistoryStoreError> {
            store.initialize_database()?;
            if database_existed {
                store.catch_up_pending_repairs()?;
            } else {
                store.catch_up_discovered_sessions()?;
            }
            store.reconcile_session_locations()?;
            store.validate_committed_attachments()
        };
        if let Err(error) = initialize() {
            if !database_existed {
                return Err(error);
            }
            log::warn!(
                "discarding incompatible Agent history index path={} error={error}",
                store.inner.paths.sqlite_database.display()
            );
            discard_sqlite(&store.inner.paths.sqlite_database)
                .map_err(AgentHistoryStoreError::Io)?;
            // The rebuild judges every session again.
            store.inner.projection_lag.lock().clear();
            store.initialize_database()?;
            store.catch_up_discovered_sessions()?;
            store.reconcile_session_locations()?;
            store.validate_committed_attachments()?;
        }
        store.record_quarantined_references();
        store.cleanup_all_unreferenced_attachments()?;
        Ok(store)
    }

    pub fn paths(&self) -> &AgentHistoryPaths {
        &self.inner.paths
    }

    pub fn stop_accepting(&self) {
        self.inner.accepting.store(false, Ordering::Release);
    }

    pub fn list_memory_candidates(
        &self,
        query: AgentMemoryHistoryQuery,
    ) -> Result<Vec<AgentMemoryHistoryCandidate>, AgentHistoryStoreError> {
        self.list_memory_candidates_inner(query)
    }

    pub fn load_memory_evidence(
        &self,
        candidate: AgentMemoryHistoryCandidate,
        maximum_items: usize,
        maximum_chars: usize,
    ) -> Result<AgentMemoryEvidence, AgentHistoryStoreError> {
        self.load_memory_evidence_inner(candidate, maximum_items, maximum_chars)
    }
}

impl AgentHistoryStore for FileAgentHistoryStore {
    fn allocate_session_id(&self) -> Result<AgentSessionId, AgentHistoryStoreError> {
        self.allocate_session_id_inner()
    }

    fn create_session(
        &self,
        commit: AgentConversationCommit,
    ) -> Result<(), AgentHistoryStoreError> {
        self.create_session_inner(commit)
    }

    fn append_commit(&self, commit: AgentConversationCommit) -> Result<(), AgentHistoryStoreError> {
        self.append_commit_inner(commit)
    }

    fn append_delta(&self, delta: AgentConversationDelta) -> Result<(), AgentHistoryStoreError> {
        self.append_delta_inner(delta)
    }

    fn enqueue_commit(&self, commit: AgentConversationCommit, ack: AgentHistoryAppendAck) {
        self.enqueue_append(
            commit.runtime.id,
            PendingHistoryCommit::checkpoint(commit),
            ack,
        );
    }

    fn enqueue_delta(&self, delta: AgentConversationDelta, ack: AgentHistoryAppendAck) {
        self.enqueue_append(delta.session_id, PendingHistoryCommit::delta(delta), ack);
    }

    fn create_fork(
        &self,
        source_session_id: AgentSessionId,
        before_turn_id: AgentTurnId,
        commit: AgentConversationCommit,
    ) -> Result<AgentHistoryBase, AgentHistoryStoreError> {
        self.create_fork_inner(source_session_id, before_turn_id, commit)
    }

    fn create_fork_at_head(
        &self,
        source_session_id: AgentSessionId,
        source_revision: AgentRevision,
        commit: AgentConversationCommit,
    ) -> Result<AgentHistoryBase, AgentHistoryStoreError> {
        self.create_fork_at_head_inner(source_session_id, source_revision, commit)
    }

    fn stage_image_attachment(
        &self,
        session_id: AgentSessionId,
        image: hanafubuki_agent_kernel::PreparedAgentImage,
    ) -> Result<hanafubuki_agent_kernel::AgentImageAttachmentRef, AgentHistoryStoreError> {
        self.stage_image_attachment_inner(session_id, image)
    }

    fn load_image_attachment(
        &self,
        attachment: &hanafubuki_agent_kernel::AgentImageAttachmentRef,
    ) -> Result<Vec<u8>, AgentHistoryStoreError> {
        self.load_image_attachment_inner(attachment)
    }

    fn enqueue_image_attachment_load(
        &self,
        session_id: AgentSessionId,
        attachments: Vec<hanafubuki_agent_kernel::AgentImageAttachmentRef>,
        ack: hanafubuki_agent_kernel::AgentImageAttachmentLoadAck,
    ) {
        self.enqueue_attachment_load(session_id, attachments, ack);
    }

    fn cleanup_unreferenced_image_attachments(
        &self,
        session_id: AgentSessionId,
    ) -> Result<(), AgentHistoryStoreError> {
        self.cleanup_unreferenced_attachments_for(session_id)
    }

    fn load_visible_image_attachment(
        &self,
        session_id: AgentSessionId,
        attachment: &hanafubuki_agent_kernel::AgentImageAttachmentRef,
    ) -> Result<Vec<u8>, AgentHistoryStoreError> {
        self.load_visible_image_attachment_inner(session_id, attachment)
    }

    fn write_session_export(
        &self,
        session_id: AgentSessionId,
        file_name: &str,
        contents: &str,
    ) -> Result<String, AgentHistoryStoreError> {
        self.write_session_export_inner(session_id, file_name, contents)
    }

    fn write_compression_store(
        &self,
        session_id: AgentSessionId,
        store: &hanafubuki_agent_kernel::AgentContentStore,
    ) -> Result<(), AgentHistoryStoreError> {
        self.write_compression_store_inner(session_id, store)
    }

    fn load_compression_store(
        &self,
        session_id: AgentSessionId,
    ) -> Result<Option<hanafubuki_agent_kernel::AgentContentStore>, AgentHistoryStoreError> {
        self.load_compression_store_inner(session_id)
    }

    fn flush_session(&self, session_id: AgentSessionId) -> Result<(), AgentHistoryStoreError> {
        self.flush_session_inner(session_id)
    }

    fn shutdown_session(&self, session_id: AgentSessionId) -> Result<(), AgentHistoryStoreError> {
        self.shutdown_session_inner(session_id)
    }

    fn discard_live_session(&self, session_id: AgentSessionId) {
        self.discard_live_session_inner(session_id);
    }

    fn load_recovery_candidates(
        &self,
    ) -> Result<Vec<Result<AgentRecoveredSession, AgentRecoveryLoadFailure>>, AgentHistoryStoreError>
    {
        self.load_recovery_candidates_inner()
    }

    fn load_runtime_state(
        &self,
        session_id: AgentSessionId,
    ) -> Result<hanafubuki_agent_kernel::PersistedAgentRuntimeState, AgentHistoryStoreError> {
        self.with_compression(self.load_runtime_state_inner(session_id)?)
    }

    fn load_latest_model_context(
        &self,
        session_id: AgentSessionId,
    ) -> Result<Vec<AgentHistoryItem>, AgentHistoryStoreError> {
        self.load_latest_model_context_inner(session_id)
    }

    fn list_sessions_page(
        &self,
        request: AgentSessionListRequest,
    ) -> Result<AgentSessionPage, AgentHistoryStoreError> {
        self.list_sessions_page_inner(request)
    }

    fn recent_user_messages(
        &self,
        limit: usize,
        before: Option<u128>,
    ) -> Result<Vec<String>, AgentHistoryStoreError> {
        self.recent_user_messages_inner(limit, before)
    }

    fn list_items_page(
        &self,
        request: AgentTranscriptPageRequest,
    ) -> Result<AgentTranscriptPage, AgentHistoryStoreError> {
        self.list_items_page_inner(request)
    }

    fn list_turn_items_page(
        &self,
        request: AgentTurnPageRequest,
    ) -> Result<AgentTranscriptPage, AgentHistoryStoreError> {
        self.list_turn_items_page_inner(request)
    }

    fn archive_session(
        &self,
        session_id: AgentSessionId,
        expected_revision: AgentRevision,
    ) -> Result<(), AgentHistoryStoreError> {
        self.archive_session_inner(session_id, expected_revision)
    }

    fn restore_session(
        &self,
        session_id: AgentSessionId,
        expected_revision: AgentRevision,
    ) -> Result<(), AgentHistoryStoreError> {
        self.restore_session_inner(session_id, expected_revision)
    }

    fn check_sessions_deletable(
        &self,
        session_ids: &[AgentSessionId],
    ) -> Result<(), AgentHistoryStoreError> {
        self.check_sessions_deletable_inner(session_ids)
    }

    fn delete_sessions_confirmed(
        &self,
        session_ids: &[AgentSessionId],
    ) -> Result<(), AgentHistoryStoreError> {
        self.delete_sessions_confirmed_inner(session_ids)
    }

    fn repair_session_projection(
        &self,
        session_id: AgentSessionId,
    ) -> Result<(), AgentHistoryStoreError> {
        self.repair_session_projection_inner(session_id)
    }

    fn shutdown(&self, deadline: std::time::Instant) -> Vec<AgentHistoryStoreError> {
        self.shutdown_inner(deadline)
    }
}

fn lifecycle_name(lifecycle: AgentSessionLifecycle) -> &'static str {
    match lifecycle {
        AgentSessionLifecycle::Available => "available",
        AgentSessionLifecycle::Archived => "archived",
    }
}

fn parse_lifecycle(value: &str) -> Result<AgentSessionLifecycle, AgentHistoryStoreError> {
    match value {
        "available" => Ok(AgentSessionLifecycle::Available),
        "archived" => Ok(AgentSessionLifecycle::Archived),
        other => Err(AgentHistoryStoreError::CorruptHistory(format!(
            "unknown session lifecycle {other}"
        ))),
    }
}

fn turn_phase_name(phase: AgentTurnPhase) -> &'static str {
    match phase {
        AgentTurnPhase::Sampling => "sampling",
        AgentTurnPhase::RunningTools => "running_tools",
        AgentTurnPhase::WaitingForUserInput => "waiting_for_user_input",
        AgentTurnPhase::Completed => "completed",
        AgentTurnPhase::Cancelled => "cancelled",
        AgentTurnPhase::Failed => "failed",
        AgentTurnPhase::Interrupted => "interrupted",
    }
}

fn conversation_item_type(item: &ConversationItem) -> &'static str {
    match item {
        ConversationItem::CanonicalInitialContext { .. } => "canonical_initial_context",
        ConversationItem::TurnStart { .. } => "turn_start",
        ConversationItem::WorldStateUpdate { .. } => "world_state_update",
        ConversationItem::SkillContext { .. } => "skill_context",
        ConversationItem::UserMessage { .. } => "user_message",
        ConversationItem::AssistantMessage { .. } => "assistant_message",
        ConversationItem::Reasoning { .. } => "reasoning",
        ConversationItem::SteeringMessage { .. } => "steering_message",
        ConversationItem::ToolCall { .. } => "tool_call",
        ConversationItem::ToolOutput { .. } => "tool_output",
        ConversationItem::ToolSelection { .. } => "tool_selection",
        ConversationItem::PermissionReview { .. } => "permission_review",
        ConversationItem::ContinuationContext { .. } => "continuation_context",
        ConversationItem::CompactionSummary { .. } => "compaction_summary",
        ConversationItem::ProviderCompaction { .. } => "provider_compaction",
        ConversationItem::UserShellCommand { .. } => "user_shell_command",
    }
}

fn now_millis() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

fn to_i64(value: u64, field: &str) -> Result<i64, AgentHistoryStoreError> {
    i64::try_from(value).map_err(|_| {
        AgentHistoryStoreError::CorruptHistory(format!("{field} exceeds SQLite integer range"))
    })
}

fn from_i64(value: i64, field: &str) -> Result<u64, AgentHistoryStoreError> {
    u64::try_from(value)
        .map_err(|_| AgentHistoryStoreError::CorruptHistory(format!("{field} is negative")))
}

fn io_error(error: std::io::Error) -> AgentHistoryStoreError {
    AgentHistoryStoreError::Io(error.to_string())
}

fn database_error(error: rusqlite::Error) -> AgentHistoryStoreError {
    AgentHistoryStoreError::ProjectionUnavailable(error.to_string())
}

fn json_error(error: serde_json::Error) -> AgentHistoryStoreError {
    AgentHistoryStoreError::CorruptHistory(error.to_string())
}

#[cfg(test)]
mod tests;
