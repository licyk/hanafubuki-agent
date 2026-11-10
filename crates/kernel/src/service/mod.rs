use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use hanafubuki_agent_approval::{
    ApprovalAction, ApprovalContext, ApprovalContextScope, ApprovalDecision,
    ApprovalDecisionSource, ApprovalError, ApprovalOutcome, ApprovalRequestId, ApprovalService,
    ApprovalUserDecision, AuthorizedAction,
};
use hanafubuki_agent_foundation::cancellation::CancellationToken;
use hanafubuki_agent_model_api::{
    AnthropicMessagesModelTransport, ChatCompletionsModelTransport, ModelApiProtocol,
    ModelCatalogClient, ModelCatalogEntry, ModelContextRole, ModelCredential, ModelEndpoint,
    ModelError, ModelEvent, ModelImageInputCapability, ModelInputCapabilities, ModelInputItem,
    ModelOutputItem, ModelPort, ModelProvider, ModelRequest, ModelRetryPolicy, ModelSession,
    ModelSessionFactory, ModelToolCall, ResponsesModelTransport,
};
use parking_lot::{Condvar, Mutex};
use serde_json::json;

use crate::activity::AgentActivityTracker;
use crate::budget::{AgentTurnBudgetLedger, AgentTurnBudgetPolicy};
use crate::conversation::{
    AgentConversationCommit, AgentConversationDelta, AgentHistoryBase, AgentHistoryItem,
    AgentHistoryItemId, AgentHistoryMutation, AgentHistorySortDirection, AgentHistoryStore,
    AgentHistoryStoreError, AgentModelContextBoundary, AgentSessionListRequest,
    AgentSessionOpenResult, AgentSessionPage, AgentTranscriptPage, AgentTranscriptPageRequest,
    AgentTurnPageRequest, AgentTurnRecovery, EphemeralAgentHistoryStore,
    PersistedAgentRuntimeState, PersistedAgentTurn,
};
use crate::core_tools::{CoreToolCall, RequestUserInputTool, UpdateTaskPlanTool};
use crate::limits::{AgentHardLimits, FIRST_BATCH_HARD_LIMITS};
use crate::loop_control::{
    sampling_progress_digest, AgentFollowUpReason, AgentNoProgressState, SamplingRequestResult,
};
use crate::model::{
    AgentCompactionGeneration, AgentCompactionId, AgentCompactionImplementation,
    AgentCompactionOutcome, AgentCompactionPhase, AgentCompactionReason, AgentCompactionSnapshot,
    AgentCompactionStatus, AgentCompactionTrigger, AgentCompactionUsage, AgentContextTokenStatus,
    AgentContextWindowSnapshot, AgentError, AgentEvent, AgentEventKind, AgentEventSequence,
    AgentEventSink, AgentGeneration, AgentGoal, AgentGoalChange, AgentMemoryContributionState,
    AgentMessageId, AgentMessagePhase, AgentModelRetryStage, AgentModelSessionGeneration,
    AgentModelUsage, AgentPlanBlock, AgentReasoningCheckpoint, AgentReasoningId,
    AgentReasoningTransitionError, AgentRevision, AgentSamplingRetryPolicy, AgentSessionConfig,
    AgentSessionForkOutcome, AgentSessionForkRequest, AgentSessionHeadForkOutcome,
    AgentSessionHeadForkRequest, AgentSessionId, AgentSessionLifecycle,
    AgentSessionModelConfigurationUpdate, AgentSessionRuntimeSnapshot, AgentSessionSummary,
    AgentSessionTitle, AgentSessionTitleRenameRequest, AgentSessionTitleSource,
    AgentShutdownDiagnostic, AgentSteeringAccepted, AgentSteeringInput, AgentSteeringSequence,
    AgentTaskPlanSnapshot, AgentTurnAccepted, AgentTurnCancellationOutcome, AgentTurnId,
    AgentTurnOutcome, AgentTurnPhase, AgentTurnTerminalReason, AgentUsageSource,
    AgentUserInputAccepted, AgentUserInputAnswer, AgentUserInputRequest, AgentUserInputRequestId,
    AgentUserInputResolution, AgentWorldStateSection, AgentWorldStateSnapshot,
    AgentWorldStateUpdate, ConfirmedAgentSessionDeletion, ContextSectionKind, ConversationItem,
    PermissionAssessment, PermissionAssessmentOutcome, PermissionReviewStatus,
    PreparedAgentSessionDeletion, MAXIMUM_AGENT_WORLD_STATE_SECTIONS,
    MAXIMUM_AGENT_WORLD_STATE_SECTION_BYTES, MAXIMUM_AGENT_WORLD_STATE_TOTAL_BYTES,
};
use crate::permission::{review_permission, PermissionReviewError};
use crate::tool_catalog::{AgentToolCatalogContext, AgentToolPlan, AgentToolSearchIndexCache};
use crate::tool_runtime::{AgentToolError, AgentToolInvocationContext, AgentToolOutput};
use crate::{AgentContinuationHookOutcome, AgentKernelRegistries};
use hanafubuki_agent_contracts::{
    AgentContinuationContext, AgentContinuationHookDecision, AgentContinuationHookError,
    AgentContinuationHookFailurePolicy, AgentContinuationHookRegistration, AgentNotification,
    RegistryEntry, MAXIMUM_CONTINUATION_FRAGMENTS_PER_HOOK, MAXIMUM_CONTINUATION_FRAGMENT_BYTES,
};

mod attachments;

#[derive(Clone)]
pub struct AgentService {
    inner: Arc<AgentServiceInner>,
}

struct AgentServiceInner {
    default_model: Option<Arc<dyn ModelPort>>,
    /// What plugins registered, snapshotted per turn, sampling step or call.
    registries: AgentKernelRegistries,
    tool_search_cache: AgentToolSearchIndexCache,
    tool_spec_cache: crate::tool_catalog::AgentToolSpecCache,
    /// The durable store, and the memory one of the ephemeral sessions.
    history_store: SessionStores,
    limits: AgentHardLimits,
    /// The live sessions, one cell each (see `state::cell`).
    sessions: SessionTable,
    next_turn_id: AtomicU64,
    next_compaction_id: AtomicU64,
    image_capabilities: Mutex<ModelImageCapabilityCache>,
    model_catalog: crate::model_catalog::ModelCatalog,
    approvals: ApprovalService,
    pending_approvals: Mutex<HashMap<ApprovalRequestId, PendingApprovalRoute>>,
    pending_user_inputs: Mutex<HashMap<AgentUserInputRequestId, PendingUserInputRoute>>,
    next_user_input_request_id: AtomicU64,
    managed_turns: Mutex<HashMap<TurnIdentity, Option<tokio::task::AbortHandle>>>,
    /// Signalled when a managed turn task ends; shutdown waits on it.
    managed_turns_changed: Condvar,
    /// The background compactions' tasks (`compaction::background`), by
    /// session; shutdown, archive and delete wait for them.
    managed_background: Mutex<HashSet<(AgentSessionId, AgentCompactionId)>>,
    managed_background_changed: Condvar,
    event_sinks: Mutex<HashMap<AgentSessionId, Arc<dyn AgentEventSink>>>,
    accepting: AtomicBool,
    /// Told of a panic the kernel caught in its own code.
    kernel_faults: Arc<dyn crate::AgentKernelFaultSink>,
}

struct PendingApprovalRoute {
    identity: TurnIdentity,
    context: ApprovalContext,
}

struct PendingUserInputRoute {
    identity: TurnIdentity,
    call_id: String,
    request: AgentUserInputRequest,
    sender: Option<tokio::sync::oneshot::Sender<AgentUserInputResolution>>,
}

fn history_contents<T: std::borrow::Borrow<AgentHistoryItem>>(
    history: &[T],
) -> Vec<ConversationItem> {
    history
        .iter()
        .map(|item| item.borrow().content.clone())
        .collect()
}

fn mutable_history(record: &mut SessionRecord) -> &mut Vec<Arc<AgentHistoryItem>> {
    record.conversation.history_version = record
        .conversation
        .history_version
        .checked_add(1)
        .expect("Agent history version exhausted");
    Arc::make_mut(&mut record.conversation.history)
}

fn replace_history(record: &mut SessionRecord, history: Vec<AgentHistoryItem>) {
    record.conversation.history_version = record
        .conversation
        .history_version
        .checked_add(1)
        .expect("Agent history version exhausted");
    record.conversation.history = Arc::new(history.into_iter().map(Arc::new).collect());
}

#[derive(Clone)]
struct TurnRecord {
    id: AgentTurnId,
    generation: AgentGeneration,
    phase: AgentTurnPhase,
    sampling_step: u32,
    reasoning: AgentReasoningCheckpoint,
    final_answer_message_id: Option<crate::AgentMessageId>,
    last_error: Option<String>,
    terminal_reason: Option<AgentTurnTerminalReason>,
    cancellation: CancellationToken,
    created_at: u128,
    updated_at: u128,
    recovery: Option<AgentTurnRecovery>,
    budget_ledger: AgentTurnBudgetLedger,
    no_progress: AgentNoProgressState,
    pending_steering: VecDeque<AgentSteeringInput>,
    next_steering_sequence: u64,
    last_consumed_steering_sequence: Option<AgentSteeringSequence>,
    model_session: Arc<dyn ModelSession>,
    model_session_generation: AgentModelSessionGeneration,
    permission_mode: hanafubuki_agent_approval::AgentPermissionMode,
    consecutive_auto_review_denials: u8,
    sampling_retry_policy: AgentSamplingRetryPolicy,
    /// The turn's extensions, frozen at admission. They are never included
    /// in a runtime snapshot or canonical history.
    extensions: hanafubuki_agent_contracts::TurnExtensions,
    /// The hierarchical compression failed this turn (a panic in the
    /// algorithm): the turn's requests carry the full history, as under
    /// `standard`, and the next turn tries again.
    compression_fallback: bool,
}

#[derive(Clone)]
struct CompactionRecord {
    snapshot: AgentCompactionSnapshot,
    cancellation: CancellationToken,
    operation_revision: u64,
}

#[derive(Clone, Copy)]
struct CompactionIdentity {
    session_id: AgentSessionId,
    id: AgentCompactionId,
    generation: AgentCompactionGeneration,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct TurnIdentity {
    session_id: AgentSessionId,
    turn_id: AgentTurnId,
    generation: AgentGeneration,
}

struct TurnExecutionGuard {
    service: AgentService,
    identity: TurnIdentity,
    event_sink: Arc<dyn AgentEventSink>,
    armed: bool,
}

struct NoopAgentEventSink;

impl AgentEventSink for NoopAgentEventSink {
    fn emit(&self, _event: AgentEvent) {}
}

struct UnavailableRecoveredModel;

impl ModelPort for UnavailableRecoveredModel {
    fn stream(
        &self,
        _request: ModelRequest,
        _cancellation: CancellationToken,
    ) -> hanafubuki_agent_model_api::ModelFuture<
        '_,
        hanafubuki_agent_model_api::ModelResult<hanafubuki_agent_model_api::ModelStream>,
    > {
        Box::pin(async {
            Err(ModelError::InvalidRequest(
                "the recovered session has no available model transport".to_string(),
            ))
        })
    }
}

struct SamplingOutput {
    result: SamplingRequestResult,
    items: Vec<AgentSamplingItem>,
    tool_calls: Vec<ModelToolCall>,
    provider_end_turn: Option<bool>,
}

#[derive(Clone, Debug)]
enum AgentSamplingItem {
    Assistant {
        message_id: crate::AgentMessageId,
        phase: Option<crate::AgentMessagePhase>,
        text: String,
    },
    ToolCall(ModelToolCall),
}

#[derive(Clone, Debug)]
enum PendingModelItem {
    Assistant {
        message_id: AgentMessageId,
        provider_phase: Option<hanafubuki_agent_model_api::ModelMessagePhase>,
        streamed_text: String,
        filtered_text_bytes: usize,
        plan_output: Option<plan_output::PlanOutputProjection>,
        completed: Option<(
            Option<hanafubuki_agent_model_api::ModelMessagePhase>,
            String,
        )>,
    },
    ToolCall(ModelToolCall),
    Reasoning(sampling::PendingReasoning),
    Ignored {
        item_type: String,
        completed: bool,
    },
}

/// The continuation hooks of one sampling step, in the order they are asked.
type ContinuationHooks = Vec<Arc<RegistryEntry<AgentContinuationHookRegistration>>>;

enum ToolApprovalDisposition {
    Approved(Box<AuthorizedAction>),
    Rejected(String),
}

const PLAN_IMPLEMENTATION_MESSAGE: &str = "Implement the plan.";

enum TurnAdmission {
    Regular,
    ImplementPlan(crate::AgentPlanImplementationRequest),
}

mod compaction;
mod compression;
mod diagnostics;
mod durable;
mod factory;
mod image_capability;
mod interaction;
mod lifecycle;
mod persistence;
mod plan_output;
mod progress;
mod recovery;
pub(crate) mod retry_wait;
mod sampling;
mod session;
mod state;
mod stores;
mod tool_execution;
mod turn;
mod validation;

use diagnostics::{trace_tool_call, trace_tool_result};
use durable::StagedPersistence;
use factory::configured_model;
use image_capability::ModelImageCapabilityCache;
use persistence::*;
use sampling::*;
use state::*;
use stores::SessionStores;
use validation::*;
