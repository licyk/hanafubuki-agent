use serde::{Deserialize, Serialize};

use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentUsageSource {
    ProviderReported,
    Estimated,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentModelUsage {
    pub last_response_input_tokens: u64,
    pub last_response_output_tokens: u64,
    pub cumulative_input_tokens: u64,
    pub cumulative_output_tokens: u64,
    pub context_window_tokens: Option<u64>,
    pub context_consumed_tokens: Option<u64>,
    pub source: AgentUsageSource,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentTurnPhase {
    Sampling,
    RunningTools,
    WaitingForUserInput,
    Completed,
    Cancelled,
    Failed,
    Interrupted,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentTurnTerminalReason {
    UserInterrupted,
    ModelRequestRetriesExhausted,
    ModelStreamRetriesExhausted,
    ModelError,
    ModelProtocol,
    BudgetExceeded,
    /// The turn ran to `AgentHardLimits::turn_deadline` and was stopped.
    DeadlineExceeded,
    /// The provider's context window was exceeded again after the step's
    /// one compaction.
    ContextWindowExceeded,
    ApplicationRestarted,
    InternalFailure,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentSamplingRetryPolicy {
    pub maximum_retries: u32,
    pub initial_backoff_millis: u64,
    pub maximum_backoff_millis: u64,
}

impl AgentSamplingRetryPolicy {
    pub fn delay_for_retry(self, current_retry: u32) -> std::time::Duration {
        let shift = current_retry.saturating_sub(1).min(31);
        std::time::Duration::from_millis(
            self.initial_backoff_millis
                .saturating_mul(u64::from(1u32 << shift))
                .min(self.maximum_backoff_millis),
        )
    }

    pub fn validate(self) -> Result<Self, AgentError> {
        if self.maximum_retries > 10
            || self.initial_backoff_millis == 0
            || self.maximum_backoff_millis < self.initial_backoff_millis
            || self.maximum_backoff_millis > 60_000
        {
            return Err(AgentError::InvalidInput(
                "invalid Agent response stream retry policy".to_string(),
            ));
        }
        Ok(self)
    }
}

impl Default for AgentSamplingRetryPolicy {
    fn default() -> Self {
        Self {
            maximum_retries: 2,
            initial_backoff_millis: 100,
            maximum_backoff_millis: 2_000,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentModelRetryStage {
    ResponseStream,
    ImageInputFallback,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentSessionLifecycle {
    Available,
    Archived,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentCompactionTrigger {
    Manual,
    Automatic,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentCompactionPhase {
    StandaloneTurn,
    PreTurn,
    MidTurn,
    /// Run on a snapshot of the completed turns while the session went on,
    /// and installed at a later checkpoint before the work added since.
    Background,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentCompactionReason {
    UserRequested,
    ContextLimit,
    NewContextRequested,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentCompactionImplementation {
    LocalSummary,
    ResponsesCompact,
    ResponsesCompactV2,
}

impl From<hanafubuki_agent_model_api::ModelCompactionCapability> for AgentCompactionImplementation {
    fn from(value: hanafubuki_agent_model_api::ModelCompactionCapability) -> Self {
        match value {
            hanafubuki_agent_model_api::ModelCompactionCapability::LocalSummary => {
                Self::LocalSummary
            }
            hanafubuki_agent_model_api::ModelCompactionCapability::ResponsesCompact => {
                Self::ResponsesCompact
            }
            hanafubuki_agent_model_api::ModelCompactionCapability::ResponsesCompactV2 => {
                Self::ResponsesCompactV2
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentCompactionStatus {
    Reserved,
    Running,
    ReadyToInstall,
    Completed,
    Failed,
    Cancelled,
}

impl AgentCompactionStatus {
    pub const fn is_terminal(self) -> bool {
        matches!(self, Self::Completed | Self::Failed | Self::Cancelled)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentContextWindowSnapshot {
    pub window_number: u64,
    pub first_window_id: AgentContextWindowId,
    pub previous_window_id: Option<AgentContextWindowId>,
    pub window_id: AgentContextWindowId,
    pub prefill_input_tokens: Option<u64>,
    pub new_context_requested: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentContextTokenStatus {
    pub active_context_tokens: u64,
    pub auto_compact_threshold_percent: Option<u8>,
    pub proactive_compaction_token_limit: Option<u64>,
    pub full_context_window_limit: Option<u64>,
    pub base_window_tokens_remaining: Option<u64>,
    pub token_limit_reached: bool,
}

/// The largest output assumed of a model whose own is unknown, as Hoshimi
/// assumes (`context_window.py`).
const FALLBACK_MAX_OUTPUT_TOKENS: u64 = 32_768;

impl AgentContextTokenStatus {
    /// The status of `active_context_tokens` in a window of
    /// `full_context_window_limit` (none: unknown), for a model whose
    /// largest output is `max_output_tokens` (none: unknown).
    pub fn calculate(
        active_context_tokens: u64,
        full_context_window_limit: Option<u64>,
        max_output_tokens: Option<u64>,
        auto_compaction_enabled: bool,
        auto_compact_threshold_percent: u8,
    ) -> Self {
        let proactive_compaction_token_limit = auto_compaction_enabled
            .then_some(full_context_window_limit)
            .flatten()
            .map(|window| {
                proactive_limit(window, max_output_tokens, auto_compact_threshold_percent)
            });
        let base_window_tokens_remaining = [
            proactive_compaction_token_limit
                .map(|limit| limit.saturating_sub(active_context_tokens)),
            full_context_window_limit.map(|limit| limit.saturating_sub(active_context_tokens)),
        ]
        .into_iter()
        .flatten()
        .min();
        let token_limit_reached = auto_compaction_enabled
            && (proactive_compaction_token_limit
                .is_some_and(|limit| active_context_tokens >= limit)
                || full_context_window_limit.is_some_and(|limit| active_context_tokens >= limit));
        Self {
            active_context_tokens,
            auto_compact_threshold_percent: auto_compaction_enabled
                .then_some(auto_compact_threshold_percent),
            proactive_compaction_token_limit,
            full_context_window_limit,
            base_window_tokens_remaining,
            token_limit_reached,
        }
    }
}

/// Where a session compacts by itself: at `percent` of `window`, or sooner
/// when what is left could not hold a reply (Hoshimi's `budget`). The room
/// kept for the reply is the model's largest output, but no more than a
/// tenth of the window (a reply seldom fills a large ceiling), no less than
/// 4096 tokens, and never more than half the window.
fn proactive_limit(window: u64, max_output_tokens: Option<u64>, percent: u8) -> u64 {
    let share = u64::try_from((u128::from(window) * u128::from(percent)) / 100).unwrap_or(u64::MAX);
    let headroom = max_output_tokens
        .unwrap_or(FALLBACK_MAX_OUTPUT_TOKENS)
        .min((window / 10).max(4_096))
        .min(window / 2);
    share.min(window - headroom)
}

impl AgentContextWindowSnapshot {
    pub fn initial() -> Self {
        let window_id = AgentContextWindowId::new();
        Self {
            window_number: 0,
            first_window_id: window_id,
            previous_window_id: None,
            window_id,
            prefill_input_tokens: None,
            new_context_requested: false,
        }
    }

    pub fn next(self) -> Result<Self, AgentError> {
        Ok(Self {
            window_number: self
                .window_number
                .checked_add(1)
                .ok_or(AgentError::UsageOverflow {
                    field: "Agent context window number",
                })?,
            first_window_id: self.first_window_id,
            previous_window_id: Some(self.window_id),
            window_id: AgentContextWindowId::new(),
            prefill_input_tokens: None,
            new_context_requested: false,
        })
    }

    pub fn validate(self) -> Result<Self, AgentError> {
        if !self.first_window_id.is_v7() || !self.window_id.is_v7() {
            return Err(AgentError::Persistence(
                "Agent context window identifiers must be UUIDv7".to_string(),
            ));
        }
        match (self.window_number, self.previous_window_id) {
            (0, None) if self.first_window_id == self.window_id => Ok(self),
            (0, _) => Err(AgentError::Persistence(
                "the first Agent context window has an invalid chain".to_string(),
            )),
            (_, Some(previous)) if previous.is_v7() && self.window_id != previous => Ok(self),
            _ => Err(AgentError::Persistence(
                "an Agent context window is missing its UUIDv7 predecessor".to_string(),
            )),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentCompactionUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub source: AgentUsageSource,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentCompactionSnapshot {
    pub id: AgentCompactionId,
    pub generation: AgentCompactionGeneration,
    pub trigger: AgentCompactionTrigger,
    pub phase: AgentCompactionPhase,
    pub reason: AgentCompactionReason,
    pub implementation: AgentCompactionImplementation,
    pub status: AgentCompactionStatus,
    pub source_revision: AgentRevision,
    pub source_window_id: AgentContextWindowId,
    pub target_window: AgentContextWindowSnapshot,
    pub turn_generation: Option<AgentGeneration>,
    pub summary: Option<String>,
    pub usage: Option<AgentCompactionUsage>,
    pub last_error: Option<String>,
    pub created_at: u64,
    pub updated_at: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentCompactionOutcome {
    pub session_id: AgentSessionId,
    pub revision: AgentRevision,
    pub compaction: AgentCompactionSnapshot,
}

impl AgentTurnPhase {
    pub const fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Completed | Self::Cancelled | Self::Failed | Self::Interrupted
        )
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentTurnSnapshot {
    pub id: AgentTurnId,
    pub generation: AgentGeneration,
    pub phase: AgentTurnPhase,
    pub sampling_step: u32,
    pub reasoning: AgentReasoningSnapshot,
    pub final_answer_message_id: Option<AgentMessageId>,
    pub last_error: Option<String>,
    pub terminal_reason: Option<AgentTurnTerminalReason>,
    pub created_at: u128,
    pub updated_at: u128,
    pub recovery: Option<crate::AgentTurnRecovery>,
    pub pending_steering_count: usize,
    pub last_consumed_steering_sequence: Option<AgentSteeringSequence>,
    pub model_session_generation: AgentModelSessionGeneration,
    pub permission_mode: hanafubuki_agent_approval::AgentPermissionMode,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentTurnCancellationOutcome {
    pub session_id: AgentSessionId,
    pub turn_id: AgentTurnId,
    pub generation: AgentGeneration,
    pub phase: AgentTurnPhase,
    pub reason: AgentTurnTerminalReason,
    pub revision: AgentRevision,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionRiskLevel {
    Low,
    Medium,
    High,
    Critical,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionUserAuthorization {
    Explicit,
    Implicit,
    Absent,
    Conflicting,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionAssessmentOutcome {
    Allow,
    Deny,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PermissionAssessment {
    pub risk_level: PermissionRiskLevel,
    pub user_authorization: PermissionUserAuthorization,
    pub outcome: PermissionAssessmentOutcome,
    pub rationale: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionReviewStatus {
    Reviewing,
    Approved,
    Denied,
    TimedOut,
    Failed,
    Aborted,
}

/// Returned after a turn has been durably admitted and its managed async
/// worker has been scheduled. Completion remains authoritative in the session
/// snapshot and event stream.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentTurnAccepted {
    pub session_id: AgentSessionId,
    pub turn_id: AgentTurnId,
    pub generation: AgentGeneration,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentSteeringInput {
    pub sequence: AgentSteeringSequence,
    pub text: String,
    #[serde(default)]
    pub images: Vec<super::AgentImageAttachmentRef>,
    /// Steering recorded before the host context had a kind carried it as
    /// `applicationContext`, which reads back as no context.
    pub host_context: Option<AgentHostContext>,
    pub accepted_at: u128,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentSteeringSubmission {
    pub text: String,
    pub images: Vec<super::AgentImageAttachmentRef>,
    pub host_context: Option<AgentHostContext>,
}

impl From<String> for AgentSteeringSubmission {
    fn from(text: String) -> Self {
        Self {
            text,
            images: Vec::new(),
            host_context: None,
        }
    }
}

impl From<&str> for AgentSteeringSubmission {
    fn from(text: &str) -> Self {
        Self::from(text.to_string())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentSteeringAccepted {
    pub session_id: AgentSessionId,
    pub turn_id: AgentTurnId,
    pub generation: AgentGeneration,
    pub sequence: AgentSteeringSequence,
    pub revision: AgentRevision,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentTurnOutcome {
    pub session_id: AgentSessionId,
    pub turn_id: AgentTurnId,
    pub generation: AgentGeneration,
    pub final_answer_message_id: AgentMessageId,
    pub final_answer: String,
}
