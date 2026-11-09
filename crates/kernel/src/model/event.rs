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

/// When a tool lifecycle hook ran.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentHookEvent {
    PreToolUse,
    PostToolUse,
}

/// How the kernel ended one tool lifecycle hook call.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentHookOutcome {
    Passed,
    /// The call was denied and did not run.
    Denied,
    /// Context was added after the step's tool outputs.
    AddedContext,
    /// The hook failed and had no say.
    Failed,
    /// The hook ran past the turn's tool idle timeout and had no say.
    TimedOut,
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
    /// The session's goal changed; `None` when it was cleared.
    SessionGoalUpdated {
        goal: Option<super::AgentGoal>,
    },
    TurnStarted,
    SamplingStarted {
        step: u32,
    },
    ReasoningUpdated {
        reasoning: AgentReasoningSnapshot,
    },
    /// The model started thinking: a reasoning item of `sampling_step`
    /// began. Its text follows as `ReasoningDelta`s.
    ReasoningStarted {
        reasoning_id: AgentReasoningId,
        sampling_step: u32,
    },
    /// More of the reasoning item's text, as Markdown: the provider's summary
    /// of its reasoning, or its raw reasoning text. A new summary part
    /// arrives with the blank line that separates it.
    ReasoningDelta {
        reasoning_id: AgentReasoningId,
        delta: String,
    },
    /// The reasoning item ended after `duration_millis`. The step's commit
    /// then carries it as `ConversationItem::Reasoning`.
    ReasoningCompleted {
        reasoning_id: AgentReasoningId,
        duration_millis: u64,
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
    /// A tool lifecycle hook started for the call `call_id`.
    HookStarted {
        provider_id: String,
        hook_id: String,
        call_id: String,
        event: AgentHookEvent,
    },
    HookCompleted {
        provider_id: String,
        hook_id: String,
        call_id: String,
        event: AgentHookEvent,
        outcome: AgentHookOutcome,
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
