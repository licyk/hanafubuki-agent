use hanafubuki_agent_approval::{
    ApprovalAction, ApprovalRequest, ApprovalRequestId, ApprovalResolution,
};
use serde::{Deserialize, Serialize};

use super::*;

/// How the kernel ended one continuation hook call.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentContinuationHookOutcome {
    Passed,
    Continued,
    Stopped,
    IgnoredFailure,
    Failed,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum AgentEventKind {
    SessionTitleUpdated {
        title: AgentSessionTitle,
    },
    TurnStarted,
    SamplingStarted {
        step: u32,
    },
    ReasoningUpdated {
        reasoning: AgentReasoningSnapshot,
    },
    /// The model's latest summary of what it is doing, bounded and stripped
    /// of Markdown markers. Retained for the rest of the turn until replaced.
    ReasoningHeadlineUpdated {
        headline: String,
    },
    ResponseStarted {
        response_id: String,
    },
    AssistantMessageStarted {
        message_id: AgentMessageId,
        phase: Option<AgentMessagePhase>,
        sampling_step: u32,
    },
    AssistantTextDelta {
        message_id: AgentMessageId,
        delta: String,
    },
    ProposedPlanStarted {
        message_id: AgentMessageId,
        sampling_step: u32,
    },
    ProposedPlanDelta {
        message_id: AgentMessageId,
        delta: String,
    },
    ProposedPlanCompleted {
        message_id: AgentMessageId,
    },
    AssistantMessageCompleted {
        message_id: AgentMessageId,
        phase: Option<AgentMessagePhase>,
    },
    HistoryCommitted {
        start_index: usize,
        item_count: usize,
        usage: Option<AgentModelUsage>,
    },
    ToolStarted {
        call_id: String,
        name: String,
        presentation: AgentToolPresentation,
    },
    ToolExecutionFinished {
        call_id: String,
        is_error: bool,
    },
    ExecutionProgress {
        update: AgentToolProgressUpdate,
    },
    ApprovalRequested {
        request: ApprovalRequest,
    },
    ApprovalResolved {
        request_id: ApprovalRequestId,
        resolution: ApprovalResolution,
    },
    PermissionReviewStarted {
        review_id: String,
        call_id: String,
        action: ApprovalAction,
        started_at: u64,
    },
    PermissionReviewCompleted {
        review_id: String,
        call_id: String,
        action_summary: String,
        status: PermissionReviewStatus,
        assessment: Option<PermissionAssessment>,
        started_at: u64,
        completed_at: u64,
    },
    TaskPlanUpdated {
        plan: AgentTaskPlanSnapshot,
    },
    UserInputRequested {
        request: AgentUserInputRequest,
    },
    UserInputResolved {
        resolution: AgentUserInputResolution,
    },
    ToolCompleted {
        call_id: String,
        is_error: bool,
    },
    ContinuationHookStarted {
        provider_id: String,
        hook_id: String,
    },
    ContinuationHookCompleted {
        provider_id: String,
        hook_id: String,
        outcome: AgentContinuationHookOutcome,
    },
    SteeringAccepted {
        sequence: AgentSteeringSequence,
    },
    SteeringConsumed {
        first_sequence: AgentSteeringSequence,
        through_sequence: AgentSteeringSequence,
        item_count: usize,
    },
    ModelRetryScheduled {
        stage: AgentModelRetryStage,
        current_retry: u32,
        maximum_retries: u32,
        delay_millis: u64,
        reason: String,
    },
    CompactionUpdated {
        compaction: AgentCompactionSnapshot,
    },
    TurnCompleted {
        final_answer_message_id: AgentMessageId,
    },
    TurnCancelled {
        reason: AgentTurnTerminalReason,
    },
    TurnFailed {
        reason: AgentTurnTerminalReason,
        message: String,
    },
}

impl AgentEventKind {
    pub const fn is_terminal(&self) -> bool {
        matches!(
            self,
            Self::TurnCompleted { .. } | Self::TurnCancelled { .. } | Self::TurnFailed { .. }
        )
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentEvent {
    pub session_id: AgentSessionId,
    pub turn_id: Option<AgentTurnId>,
    pub generation: AgentGeneration,
    pub sequence: AgentEventSequence,
    pub activity: Option<AgentActivitySnapshot>,
    pub kind: AgentEventKind,
}

/// Per-turn streaming projection sink. Implementations are adapters only:
/// committed history and the event journal remain authoritative even when a
/// presentation sink drops or rejects delivery.
///
/// `emit` runs on the thread that committed the event: a turn's task, a
/// session's progress consumer, or the history store's writer for the
/// terminal events of a dropped turn. It must not block or wait for the
/// kernel or the history store.
pub trait AgentEventSink: Send + Sync {
    fn emit(&self, event: AgentEvent);
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentEventBatch {
    pub events: Vec<AgentEvent>,
    pub oldest_available_sequence: AgentEventSequence,
    pub next_sequence: AgentEventSequence,
    pub truncated: bool,
}
