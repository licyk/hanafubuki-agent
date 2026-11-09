//! Tauri-free Agent domain boundary.
//!
//! The crate owns Agent session/turn state, committed conversation history,
//! model sampling, typed tool dispatch, cancellation, archive/restore and
//! confirmed permanent deletion.
//! It deliberately knows nothing about application services or Tauri.

pub mod activity;
mod admission;
mod approved_action;
pub mod budget;
mod context;
pub mod conversation;
mod core_tools;
mod kernel_faults;
pub mod limits;
pub mod loop_control;
mod memory;
pub mod model;
mod model_catalog;
pub mod parallel_tool_runtime;
pub mod permission;
pub mod plugin;
pub mod service;
pub mod tool_catalog;
mod tool_output;
pub mod tool_runtime;

#[cfg(any(test, feature = "test-support"))]
pub mod test_support;

pub use admission::collect_turn_extensions;
pub use budget::{
    AgentBudgetError, AgentTurnBudgetLedger, AgentTurnBudgetPolicy,
    ABSOLUTE_MAX_CONSECUTIVE_STALLED_STEPS, ABSOLUTE_MAX_CUMULATIVE_MODEL_INPUT_TOKENS,
    ABSOLUTE_MAX_CUMULATIVE_MODEL_OUTPUT_TOKENS, ABSOLUTE_MAX_HOOK_FRAGMENT_BYTES,
    ABSOLUTE_MAX_IDLE_TIMEOUT_SECONDS, ABSOLUTE_MAX_IN_FLIGHT_TOOLS,
    ABSOLUTE_MAX_PENDING_INPUT_ITEMS, ABSOLUTE_MAX_TOOL_INVOCATIONS,
    ABSOLUTE_MAX_TOOL_OUTPUT_BYTES,
};
pub use context::collect_world_state;
pub use conversation::{
    AgentCompactionCheckpoint, AgentConversationCommit, AgentConversationDelta,
    AgentHistoryAppendAck, AgentHistoryBase, AgentHistoryFollowUp, AgentHistoryItem,
    AgentHistoryItemId, AgentHistoryMutation, AgentHistoryRecord, AgentHistoryRecordPayload,
    AgentHistorySortDirection, AgentHistoryStore, AgentHistoryStoreError, AgentImageAttachmentLoad,
    AgentImageAttachmentLoadAck, AgentModelContextBoundary, AgentQuarantinedSession,
    AgentRecoveredSession, AgentRecoveryLoadFailure, AgentRecoveryReport, AgentSessionListRequest,
    AgentSessionOpenResult, AgentSessionPage, AgentSessionParentFilter, AgentTranscriptPage,
    AgentTranscriptPageRequest, AgentTurnPageRequest, AgentTurnRecovery,
    EphemeralAgentHistoryStore, PersistedAgentRuntimeState, PersistedAgentTurn,
    AGENT_HISTORY_FORMAT_VERSION, AGENT_SESSION_PAGE_SIZE_MAX, AGENT_TRANSCRIPT_PAGE_SIZE_MAX,
};
pub use core_tools::AgentCoreTool;
pub use hanafubuki_agent_approval::{
    AgentPermissionMode, ApprovalAction, ApprovalActionDigest, ApprovalActionKind,
    ApprovalDecision, ApprovalDecisionSource, ApprovalOrigin, ApprovalRequest, ApprovalRequestId,
    ApprovalResolution, ApprovalRiskClass, ApprovalUserDecision, AuthorizedAction,
    PermissionRequirement, SandboxEscalation,
};
pub use hanafubuki_agent_model_api::{
    flattened_tool_call_name, ModelApiProtocol, ModelCatalogEntry, ModelCompactionCapability,
    ModelCredential, ModelEvent, ModelHeader, ModelImageInputCapability, ModelImageInputPolicy,
    ModelInputCapabilities, ModelInputCapabilityPolicy, ModelInputItem, ModelJsonSchemaOutput,
    ModelOutputFormat, ModelOutputItem, ModelReasoningConfig, ModelReasoningEffort, ModelRequest,
    ModelRetryPolicy, ModelSession, ModelStructuredOutputCapability, ModelToolChoice,
    ModelToolSpec,
};
pub use hanafubuki_agent_skills::{SkillId, SkillScope};
pub use hanafubuki_agent_tool_contracts::{
    ToolCatalogRevision as AgentToolCatalogRevision, ToolExecutionClass as AgentToolExecutionClass,
    ToolExposure, ToolGeneration as AgentToolGeneration, ToolId as AgentToolId, ToolOutputContent,
    ToolProviderCapabilities, ToolProviderId as AgentToolProviderId, ToolSpec,
};
pub use kernel_faults::{AgentKernelFaultSink, NoopAgentKernelFaultSink};
pub use limits::{AgentHardLimits, FIRST_BATCH_HARD_LIMITS};
pub use loop_control::{
    sampling_progress_digest, AgentFollowUpReason, AgentNoProgressError, AgentNoProgressState,
    AgentProgressDigest, SamplingRequestResult,
};
pub use model::{
    AgentActivityAction, AgentActivityId, AgentActivityKind, AgentActivityOutputPreview,
    AgentActivityPhase, AgentActivityPresentation, AgentActivitySnapshot, AgentAttachmentId,
    AgentBackgroundCompactionState, AgentBackgroundCompactionThreshold, AgentChildSessionBase,
    AgentChildSessionRequest, AgentCompactionGeneration, AgentCompactionId,
    AgentCompactionImplementation, AgentCompactionOutcome, AgentCompactionPhase,
    AgentCompactionReason, AgentCompactionSnapshot, AgentCompactionStatus, AgentCompactionStrategy,
    AgentCompactionTrigger, AgentCompactionUsage, AgentCompressionChain, AgentCompressionDelta,
    AgentCompressionState, AgentCompressionUpdate, AgentContentStore, AgentContextTokenStatus,
    AgentContextWindowId, AgentContextWindowSnapshot, AgentContinuationHookGeneration,
    AgentContinuationHookId, AgentContinuationHookOutcome, AgentContinuationHookProviderId,
    AgentError, AgentEvent, AgentEventBatch, AgentEventKind, AgentEventSequence, AgentEventSink,
    AgentExecutionActivityRef, AgentExecutionProgressStatus, AgentGeneration, AgentGoal,
    AgentGoalChange, AgentGoalOrigin, AgentGoalStatus, AgentHierarchicalCompression,
    AgentHookEvent, AgentHookOutcome, AgentHostContext, AgentImageAttachmentRef, AgentImageDetail,
    AgentMemoryCitationSnapshot, AgentMemoryContributionMode, AgentMemoryContributionState,
    AgentMessageId, AgentMessagePhase, AgentModelRetryStage, AgentModelSessionGeneration,
    AgentModelUsage, AgentPlanBlock, AgentPlanImplementationRequest, AgentReasoningCheckpoint,
    AgentReasoningId, AgentReasoningPhase, AgentReasoningSnapshot, AgentRevision,
    AgentSamplingRetryPolicy, AgentSessionConfig, AgentSessionForkOutcome, AgentSessionForkRequest,
    AgentSessionHeadForkOutcome, AgentSessionHeadForkRequest, AgentSessionId,
    AgentSessionLifecycle, AgentSessionMode, AgentSessionModelConfigurationUpdate,
    AgentSessionOrigin, AgentSessionPersistence, AgentSessionRuntimeSnapshot, AgentSessionSummary,
    AgentSessionTitle, AgentSessionTitleRenameRequest, AgentSessionTitleSource,
    AgentSessionWorkspace, AgentShutdownDiagnostic, AgentSkillRef, AgentSteeringAccepted,
    AgentSteeringInput, AgentSteeringSequence, AgentSteeringSubmission, AgentTaskPlanSnapshot,
    AgentTaskPlanStep, AgentTaskPlanStepStatus, AgentToolOutputTruncation, AgentToolPresentation,
    AgentToolProgressUpdate, AgentTurnAccepted, AgentTurnCancellationOutcome, AgentTurnId,
    AgentTurnInput, AgentTurnOrigin, AgentTurnOutcome, AgentTurnPhase, AgentTurnSnapshot,
    AgentTurnTerminalReason, AgentUsageSource, AgentUserInputAccepted, AgentUserInputAnswer,
    AgentUserInputOption, AgentUserInputQuestion, AgentUserInputRequest, AgentUserInputRequestId,
    AgentUserInputResolution, AgentWorldStateSection, AgentWorldStateSnapshot,
    AgentWorldStateUpdate, ConfirmedAgentSessionDeletion, ContextSectionKind, ConversationItem,
    PermissionAssessment, PermissionAssessmentOutcome, PermissionReviewStatus, PermissionRiskLevel,
    PermissionUserAuthorization, PreparedAgentImage, PreparedAgentSessionDeletion,
    AGENT_SESSION_TITLE_MAX_CHARS, MAXIMUM_AGENT_PLUGIN_WORLD_STATE_SECTIONS,
    MAXIMUM_AGENT_WORLD_STATE_SECTIONS, MAXIMUM_AGENT_WORLD_STATE_SECTION_BYTES,
    MAXIMUM_AGENT_WORLD_STATE_TOTAL_BYTES, MAXIMUM_HOST_CONTEXT_BYTES,
    MAXIMUM_USER_IMAGES_PER_MESSAGE,
};
pub use parallel_tool_runtime::{
    AgentToolExecutionGate, AgentToolExecutionGateError, AgentToolExecutionPermit, CompletionSet,
    ToolCallRuntime,
};
pub use permission::{
    build_permission_review_evidence, review_permission, PermissionReviewError,
    PermissionReviewEvidence, PermissionReviewEvidenceEntry, PermissionReviewEvidenceRole,
    PermissionReviewOmission, PERMISSION_REVIEW_TIMEOUT,
};
pub use plugin::{kernel_plugin, AgentKernelRegistries};
pub use service::AgentService;
pub use tool_catalog::{
    validate_dynamic_tool_arguments, AgentMemoryContextEffect, AgentPlannedTool,
    AgentPlannedToolBinding, AgentToolCatalogContext, AgentToolCatalogError,
    AgentToolHandlerBinding, AgentToolPlan, AgentToolSearchIndexCache, AgentToolSearchInfo,
    AgentToolSearchMatch, AgentToolSelection, AgentToolTrust, DEFAULT_AGENT_TOOL_SEARCH_RESULTS,
    MAX_AGENT_DIRECT_TOOL_ENTRIES, MAX_AGENT_DIRECT_TOOL_SCHEMA_BYTES,
    MAX_AGENT_TOOL_CATALOG_ENTRIES, MAX_AGENT_TOOL_CATALOG_SCHEMA_BYTES,
    MAX_AGENT_TOOL_SEARCH_RESULTS,
};
pub use tool_runtime::{
    ActivatedSkillContext, AgentToolError, AgentToolInvocationContext, AgentToolOutput,
    AgentToolProgressSink, NoopAgentToolProgressSink,
};
