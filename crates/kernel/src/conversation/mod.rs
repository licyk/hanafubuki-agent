//! Durable, storage-neutral Agent history contracts.
//!
//! Canonical records contain incremental history mutations and optionally a
//! runtime checkpoint at a recovery boundary. Concrete stores must persist the
//! JSONL record before updating any rebuildable query projection.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::{
    AgentActivitySnapshot, AgentCompactionSnapshot, AgentContextWindowSnapshot, AgentEventSequence,
    AgentGeneration, AgentMemoryContributionState, AgentMessageId, AgentModelUsage,
    AgentNoProgressState, AgentRevision, AgentSessionConfig, AgentSessionId, AgentSessionLifecycle,
    AgentSessionRuntimeSnapshot, AgentSessionSummary, AgentSessionTitle, AgentSteeringInput,
    AgentSteeringSequence, AgentTaskPlanSnapshot, AgentTurnBudgetLedger, AgentTurnBudgetPolicy,
    AgentTurnId, AgentTurnPhase, AgentTurnTerminalReason, AgentUserInputRequest, ConversationItem,
};

pub const AGENT_HISTORY_FORMAT_VERSION: u32 = 5;
pub const AGENT_SESSION_PAGE_SIZE_MAX: usize = 100;
pub const AGENT_TRANSCRIPT_PAGE_SIZE_MAX: usize = 200;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum AgentTurnRecovery {
    InterruptedAtStartup {
        synthetic_tool_outputs: usize,
        recovered_steering_inputs: usize,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PersistedAgentTurn {
    pub id: AgentTurnId,
    pub generation: AgentGeneration,
    pub phase: AgentTurnPhase,
    pub sampling_step: u32,
    pub reasoning: crate::AgentReasoningCheckpoint,
    pub final_answer_message_id: Option<AgentMessageId>,
    pub last_error: Option<String>,
    pub terminal_reason: Option<AgentTurnTerminalReason>,
    pub created_at: u128,
    pub updated_at: u128,
    pub recovery: Option<AgentTurnRecovery>,
    pub budget_ledger: AgentTurnBudgetLedger,
    pub no_progress: AgentNoProgressState,
    pub pending_steering: Vec<AgentSteeringInput>,
    pub next_steering_sequence: u64,
    pub last_consumed_steering_sequence: Option<AgentSteeringSequence>,
    pub model_session_generation: crate::AgentModelSessionGeneration,
    pub permission_mode: hanafubuki_agent_approval::AgentPermissionMode,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct AgentHistoryItemId(u64);

impl AgentHistoryItemId {
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    pub const fn value(self) -> u64 {
        self.0
    }
}

impl fmt::Display for AgentHistoryItemId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentHistoryItem {
    pub source_session_id: AgentSessionId,
    pub item_id: AgentHistoryItemId,
    pub item_revision: u64,
    pub turn_id: Option<AgentTurnId>,
    pub logical_segment: u32,
    pub logical_order: u64,
    /// Milliseconds spent processing this item's turn. Present only on the
    /// final durable display item after that turn reaches a terminal phase.
    pub turn_working_millis: Option<u64>,
    pub content: ConversationItem,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum AgentHistoryMutation {
    AppendItem { item: AgentHistoryItem },
    UpsertItemSnapshot { item: AgentHistoryItem },
    SetModelContextBoundary { boundary: AgentModelContextBoundary },
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
#[allow(
    clippy::large_enum_variant,
    reason = "public persisted state, one per session; a box would change the type its users build"
)]
pub enum AgentModelContextBoundary {
    #[default]
    Full,
    Compacted {
        checkpoint: AgentCompactionCheckpoint,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentCompactionCheckpoint {
    pub checkpoint_item_id: AgentHistoryItemId,
    pub snapshot: AgentCompactionSnapshot,
    pub replacement_history: Vec<AgentHistoryItem>,
    pub window_chain: Vec<AgentContextWindowSnapshot>,
    pub future_items_after_item_id: u64,
    /// SHA-256 of the frozen canonical initial context used for this window.
    pub initial_context_digest: [u8; 32],
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PersistedAgentRuntimeState {
    pub id: AgentSessionId,
    pub forked_from_session_id: Option<AgentSessionId>,
    /// The session this one is a child of. Like `origin`, `cwd` and
    /// `branch`, absent from a record written before it existed, and left
    /// out when none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_session_id: Option<AgentSessionId>,
    #[serde(
        default,
        deserialize_with = "crate::model::tolerant_origin",
        skip_serializing_if = "Option::is_none"
    )]
    pub origin: Option<crate::AgentSessionOrigin>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    #[serde(default)]
    pub title: Option<AgentSessionTitle>,
    pub generation: AgentGeneration,
    pub revision: AgentRevision,
    pub config: AgentSessionConfig,
    #[serde(default)]
    pub memory_contribution: AgentMemoryContributionState,
    pub turn_budget_policy: AgentTurnBudgetPolicy,
    pub lifecycle: AgentSessionLifecycle,
    pub usage: Option<AgentModelUsage>,
    pub context_window: AgentContextWindowSnapshot,
    pub compaction: Option<AgentCompactionSnapshot>,
    pub turn: Option<PersistedAgentTurn>,
    pub activities: Vec<AgentActivitySnapshot>,
    pub task_plan: Option<AgentTaskPlanSnapshot>,
    pub pending_user_input: Option<AgentUserInputRequest>,
    pub next_message_id: u64,
    pub next_history_item_id: u64,
    pub model_context_boundary: AgentModelContextBoundary,
    pub history_base: Option<AgentHistoryBase>,
    pub created_at: u128,
    pub updated_at: u128,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentHistoryBase {
    pub session_id: AgentSessionId,
    pub end_ordinal_exclusive: u64,
    pub end_byte_offset: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentConversationCommit {
    pub runtime: PersistedAgentRuntimeState,
    pub history_mutations: Vec<AgentHistoryMutation>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentConversationDelta {
    pub session_id: AgentSessionId,
    pub session_revision: AgentRevision,
    pub session_updated_at: u128,
    pub history_mutations: Vec<AgentHistoryMutation>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentHistoryRecordPayload {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub runtime_checkpoint: Option<PersistedAgentRuntimeState>,
    pub history_mutations: Vec<AgentHistoryMutation>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentHistoryRecord {
    pub format_version: u32,
    pub session_id: AgentSessionId,
    pub ordinal: u64,
    pub recorded_at: u128,
    pub commit_id: String,
    pub session_revision: AgentRevision,
    pub session_updated_at: u128,
    pub payload: AgentHistoryRecordPayload,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentHistorySortDirection {
    Ascending,
    #[default]
    Descending,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AgentSessionListRequest {
    pub page_size: usize,
    pub cursor: Option<String>,
    pub lifecycle: AgentSessionLifecycle,
    pub direction: AgentHistorySortDirection,
    pub search: Option<String>,
    #[serde(default)]
    pub parent: AgentSessionParentFilter,
}

/// Which sessions a list holds by their parent.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AgentSessionParentFilter {
    /// The sessions without a parent: the list a person picks from.
    #[default]
    TopLevel,
    /// The children of one session.
    ChildrenOf(AgentSessionId),
}

impl Default for AgentSessionListRequest {
    fn default() -> Self {
        Self {
            page_size: 50,
            cursor: None,
            lifecycle: AgentSessionLifecycle::Available,
            direction: AgentHistorySortDirection::Descending,
            search: None,
            parent: AgentSessionParentFilter::TopLevel,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentSessionPage {
    pub sessions: Vec<AgentSessionSummary>,
    pub next_cursor: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AgentTranscriptPageRequest {
    pub session_id: AgentSessionId,
    pub page_size: usize,
    pub cursor: Option<String>,
    pub direction: AgentHistorySortDirection,
    pub through_revision: Option<AgentRevision>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AgentTurnPageRequest {
    pub session_id: AgentSessionId,
    pub turn_id: AgentTurnId,
    pub page_size: usize,
    pub cursor: Option<String>,
    pub direction: AgentHistorySortDirection,
    pub through_revision: Option<AgentRevision>,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentTranscriptPage {
    pub items: Vec<AgentHistoryItem>,
    pub older_cursor: Option<String>,
    pub newer_cursor: Option<String>,
    pub through_revision: AgentRevision,
}

impl Serialize for AgentTranscriptPage {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        #[derive(Serialize)]
        #[serde(rename_all = "camelCase")]
        struct PresentationPage<'a> {
            items: Vec<AgentHistoryItem>,
            older_cursor: &'a Option<String>,
            newer_cursor: &'a Option<String>,
            through_revision: AgentRevision,
        }

        let items = self
            .items
            .iter()
            .cloned()
            .map(|mut item| {
                if let ConversationItem::AssistantMessage { model_text, .. } = &mut item.content {
                    *model_text = None;
                }
                item
            })
            .collect();
        PresentationPage {
            items,
            older_cursor: &self.older_cursor,
            newer_cursor: &self.newer_cursor,
            through_revision: self.through_revision,
        }
        .serialize(serializer)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentSessionOpenResult {
    pub runtime: AgentSessionRuntimeSnapshot,
    pub transcript: AgentTranscriptPage,
    pub through_revision: AgentRevision,
    pub next_event_sequence: AgentEventSequence,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentRecoveredSession {
    pub runtime: PersistedAgentRuntimeState,
    pub model_context: Vec<AgentHistoryItem>,
}

/// A recovery candidate the store could not load. Its files stay as they
/// are; recovery quarantines it and loads the others.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentRecoveryLoadFailure {
    pub session_id: AgentSessionId,
    pub error: AgentHistoryStoreError,
}

/// A session recovery skipped because it failed to load or validate. Nothing
/// of it was rewritten or deleted; until a later start recovers it, opening it
/// fails with its unresolved recovery obligation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentQuarantinedSession {
    pub session_id: AgentSessionId,
    pub reason: String,
}

/// What start-up recovery did: the sessions it restored and the ones it
/// quarantined.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AgentRecoveryReport {
    pub restored: usize,
    pub quarantined: Vec<AgentQuarantinedSession>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AgentHistoryStoreError {
    NotFound {
        session_id: AgentSessionId,
    },
    StaleRevision,
    InvalidRequest(String),
    InvalidCursor(String),
    IncompatibleFormat {
        found: u32,
        expected: u32,
    },
    CorruptHistory(String),
    ReferencedByFork {
        session_id: AgentSessionId,
    },
    /// A session file the history keeps but the index does not hold (a
    /// rollout moved to `.recovery`, one refused at open) forks from the
    /// session or names one of its attachments, so deleting it would take
    /// what that file needs. `dependents` names each file, with its session
    /// when the name says it.
    ReferencedByQuarantinedSession {
        session_id: AgentSessionId,
        dependents: Vec<String>,
    },
    ProjectionUnavailable(String),
    ShuttingDown,
    /// Shutdown's deadline passed before the session's writer stopped.
    WriterDidNotStop {
        session_id: AgentSessionId,
    },
    Io(String),
}

impl fmt::Display for AgentHistoryStoreError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotFound { session_id } => {
                write!(formatter, "Agent session {session_id} not found")
            }
            Self::StaleRevision => formatter.write_str("stale Agent history revision"),
            Self::InvalidRequest(message) => {
                write!(formatter, "invalid Agent history request: {message}")
            }
            Self::InvalidCursor(message) => {
                write!(formatter, "invalid Agent history cursor: {message}")
            }
            Self::IncompatibleFormat { found, expected } => write!(
                formatter,
                "incompatible Agent history format {found}; expected {expected}"
            ),
            Self::CorruptHistory(message) => write!(formatter, "corrupt Agent history: {message}"),
            Self::ReferencedByFork { session_id } => {
                write!(
                    formatter,
                    "Agent session {session_id} is referenced by a fork"
                )
            }
            Self::ReferencedByQuarantinedSession {
                session_id,
                dependents,
            } => write!(
                formatter,
                "Agent session {session_id} cannot be deleted: quarantined session files that \
                 the history keeps still need its history or attachments ({}); restore or \
                 remove them first",
                dependents.join("; ")
            ),
            Self::ProjectionUnavailable(message) => {
                write!(formatter, "Agent history projection unavailable: {message}")
            }
            Self::ShuttingDown => formatter.write_str("Agent history store is shutting down"),
            Self::WriterDidNotStop { session_id } => write!(
                formatter,
                "the Agent history writer of session {session_id} did not stop before the \
                 shutdown deadline; appends it had not acknowledged are not durable"
            ),
            Self::Io(message) => write!(formatter, "Agent history I/O failed: {message}"),
        }
    }
}

impl std::error::Error for AgentHistoryStoreError {}

pub trait AgentHistoryStore: Send + Sync {
    fn allocate_session_id(&self) -> Result<AgentSessionId, AgentHistoryStoreError>;
    fn create_session(&self, commit: AgentConversationCommit)
        -> Result<(), AgentHistoryStoreError>;
    /// Appends `commit`, returning once it is durable or failed.
    fn append_commit(&self, commit: AgentConversationCommit) -> Result<(), AgentHistoryStoreError>;
    /// Appends `delta`, returning once it is durable or failed.
    fn append_delta(&self, delta: AgentConversationDelta) -> Result<(), AgentHistoryStoreError>;
    /// Queues `commit` behind the session's earlier appends and returns
    /// without waiting for it; `ack` reports the outcome, and a follow-up it
    /// hands back is appended next. The caller, which may be an async worker
    /// or a drop guard, only finds or starts the session's writer and
    /// queues; it never waits for the append itself.
    fn enqueue_commit(&self, commit: AgentConversationCommit, ack: AgentHistoryAppendAck);
    /// `enqueue_commit` for a delta.
    fn enqueue_delta(&self, delta: AgentConversationDelta, ack: AgentHistoryAppendAck);
    fn create_fork(
        &self,
        source_session_id: AgentSessionId,
        before_turn_id: AgentTurnId,
        commit: AgentConversationCommit,
    ) -> Result<AgentHistoryBase, AgentHistoryStoreError>;
    fn create_fork_at_head(
        &self,
        source_session_id: AgentSessionId,
        source_revision: AgentRevision,
        commit: AgentConversationCommit,
    ) -> Result<AgentHistoryBase, AgentHistoryStoreError> {
        let _ = (source_session_id, source_revision, commit);
        Err(AgentHistoryStoreError::Io(
            "committed-head forks are not supported by this history store".to_string(),
        ))
    }
    fn stage_image_attachment(
        &self,
        session_id: AgentSessionId,
        image: crate::PreparedAgentImage,
    ) -> Result<crate::AgentImageAttachmentRef, AgentHistoryStoreError> {
        let _ = (session_id, image);
        Err(AgentHistoryStoreError::Io(
            "image attachments are not supported by this history store".to_string(),
        ))
    }
    fn load_image_attachment(
        &self,
        attachment: &crate::AgentImageAttachmentRef,
    ) -> Result<Vec<u8>, AgentHistoryStoreError> {
        let _ = attachment;
        Err(AgentHistoryStoreError::Io(
            "image attachments are not supported by this history store".to_string(),
        ))
    }
    /// Loads `attachments` for a model request of `session_id` and reports
    /// their bytes through `ack`. The caller may be an async worker: a store
    /// that reads files does so on its own worker and returns at once. This
    /// default, for stores held in memory, loads them inline.
    fn enqueue_image_attachment_load(
        &self,
        session_id: AgentSessionId,
        attachments: Vec<crate::AgentImageAttachmentRef>,
        ack: AgentImageAttachmentLoadAck,
    ) {
        let _ = session_id;
        ack.acknowledge(
            attachments
                .iter()
                .map(|attachment| self.load_image_attachment(attachment))
                .collect(),
        );
    }
    fn cleanup_unreferenced_image_attachments(
        &self,
        _session_id: AgentSessionId,
    ) -> Result<(), AgentHistoryStoreError> {
        Ok(())
    }
    fn load_visible_image_attachment(
        &self,
        session_id: AgentSessionId,
        attachment: &crate::AgentImageAttachmentRef,
    ) -> Result<Vec<u8>, AgentHistoryStoreError> {
        if session_id != attachment.source_session_id {
            return Err(AgentHistoryStoreError::NotFound { session_id });
        }
        self.load_image_attachment(attachment)
    }
    fn flush_session(&self, session_id: AgentSessionId) -> Result<(), AgentHistoryStoreError>;
    fn shutdown_session(&self, session_id: AgentSessionId) -> Result<(), AgentHistoryStoreError>;
    fn discard_live_session(&self, session_id: AgentSessionId);
    /// Every session that needs recovery, oldest id first, each loaded on
    /// its own: one that fails to load is its own entry's error, and only a
    /// failure to read the index fails the whole call.
    fn load_recovery_candidates(
        &self,
    ) -> Result<Vec<Result<AgentRecoveredSession, AgentRecoveryLoadFailure>>, AgentHistoryStoreError>;
    fn load_runtime_state(
        &self,
        session_id: AgentSessionId,
    ) -> Result<PersistedAgentRuntimeState, AgentHistoryStoreError>;
    fn load_latest_model_context(
        &self,
        session_id: AgentSessionId,
    ) -> Result<Vec<AgentHistoryItem>, AgentHistoryStoreError>;
    fn list_sessions_page(
        &self,
        request: AgentSessionListRequest,
    ) -> Result<AgentSessionPage, AgentHistoryStoreError>;
    fn list_items_page(
        &self,
        request: AgentTranscriptPageRequest,
    ) -> Result<AgentTranscriptPage, AgentHistoryStoreError>;
    fn list_turn_items_page(
        &self,
        request: AgentTurnPageRequest,
    ) -> Result<AgentTranscriptPage, AgentHistoryStoreError>;
    fn archive_session(
        &self,
        session_id: AgentSessionId,
        expected_revision: AgentRevision,
    ) -> Result<(), AgentHistoryStoreError>;
    fn restore_session(
        &self,
        session_id: AgentSessionId,
        expected_revision: AgentRevision,
    ) -> Result<(), AgentHistoryStoreError>;
    /// Whether `session_ids` can be deleted together: no session outside
    /// them needs their history or attachments. The deletion's prepare step
    /// asks before anything else of the session is let go;
    /// `delete_sessions_confirmed` checks again.
    fn check_sessions_deletable(
        &self,
        session_ids: &[AgentSessionId],
    ) -> Result<(), AgentHistoryStoreError>;
    fn delete_sessions_confirmed(
        &self,
        session_ids: &[AgentSessionId],
    ) -> Result<(), AgentHistoryStoreError>;
    fn repair_session_projection(
        &self,
        session_id: AgentSessionId,
    ) -> Result<(), AgentHistoryStoreError>;
    /// Stops every writer after the commands queued before it, waiting
    /// until `deadline`. A writer that has not stopped by then is reported;
    /// what it had not acknowledged was never durable, so recovery restores
    /// the last acknowledged revision.
    fn shutdown(&self, deadline: std::time::Instant) -> Vec<AgentHistoryStoreError>;
}

mod acknowledgement;
mod ephemeral;

pub use acknowledgement::{
    AgentHistoryAppendAck, AgentHistoryFollowUp, AgentImageAttachmentLoad,
    AgentImageAttachmentLoadAck,
};
pub use ephemeral::EphemeralAgentHistoryStore;
