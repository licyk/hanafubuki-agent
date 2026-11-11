use std::fmt;

use super::AgentUsageSource;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AgentError {
    SessionNotFound,
    StaleGeneration,
    StaleRevision,
    SessionArchived,
    SessionNotArchived,
    TurnAlreadyRunning,
    TurnNotRunning,
    TurnNotEditable,
    SteeringPending,
    ApprovalNotFound,
    StaleApproval,
    UserInputNotFound,
    StaleUserInput,
    Approval(String),
    InvalidInput(String),
    SkillDisambiguation {
        name: String,
        candidates: Vec<hanafubuki_agent_skills::SkillId>,
    },
    LimitExceeded {
        resource: &'static str,
        limit: usize,
    },
    Model(String),
    ModelRequestRetriesExhausted {
        attempts: u32,
        message: String,
    },
    ModelStreamInterrupted(String),
    ModelStreamRetriesExhausted {
        retries: u32,
        message: String,
    },
    UnsupportedImageInput {
        status: Option<u16>,
        code: Option<String>,
        message: String,
        request_id: Option<String>,
    },
    /// The Provider refused the request's reasoning parameter.
    UnsupportedReasoning {
        status: Option<u16>,
        code: Option<String>,
        message: String,
        request_id: Option<String>,
    },
    ProviderContextWindowExceeded {
        status: Option<u16>,
        code: Option<String>,
        message: String,
        request_id: Option<String>,
    },
    /// The provider refused the step's request as too large for its context
    /// window again after the step's one compaction.
    ContextWindowExceededAfterCompaction {
        message: String,
    },
    Protocol(String),
    UsageOverflow {
        field: &'static str,
    },
    ContextBudgetExceeded {
        consumed: u64,
        limit: u64,
        source: AgentUsageSource,
    },
    CompactionDidNotReduceContext {
        before: u64,
        after: u64,
    },
    CompactionAlreadyRunning,
    CompactionNotRunning,
    StaleCompaction,
    CompactionUnavailable,
    Persistence(String),
    /// The kernel itself failed (a turn's task panicked); the message is for
    /// the user and the log.
    Internal(String),
    Cancelled,
    /// The turn reached its deadline and was cancelled.
    TurnDeadlineExceeded {
        deadline: std::time::Duration,
    },
    CancellationNotConfirmed,
    SessionMutationNotConfirmed,
    ShuttingDown,
    /// The plugin row a facade feature call needs (`feature`, its name) is
    /// not running, or its service panicked during the call; `reason` is
    /// the whole message for the user.
    FeatureUnavailable {
        feature: String,
        reason: String,
    },
}

impl fmt::Display for AgentError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SessionNotFound => formatter.write_str("agent session not found"),
            Self::StaleGeneration => formatter.write_str("stale agent turn generation"),
            Self::StaleRevision => formatter.write_str("stale agent session revision"),
            Self::SessionArchived => formatter.write_str("agent session is archived"),
            Self::SessionNotArchived => formatter.write_str("agent session is not archived"),
            Self::TurnAlreadyRunning => formatter.write_str("agent turn is already running"),
            Self::TurnNotRunning => formatter.write_str("agent turn is not running"),
            Self::TurnNotEditable => formatter.write_str(
                "agent turn is not editable because it is unavailable, active, or compacted",
            ),
            Self::SteeringPending => formatter.write_str("agent turn has pending steering input"),
            Self::ApprovalNotFound => formatter.write_str("agent approval request not found"),
            Self::StaleApproval => formatter.write_str("stale agent approval request"),
            Self::UserInputNotFound => formatter.write_str("agent user input request not found"),
            Self::StaleUserInput => formatter.write_str("stale agent user input request"),
            Self::Approval(message) => write!(formatter, "agent approval failed: {message}"),
            Self::InvalidInput(message) => write!(formatter, "invalid agent input: {message}"),
            Self::SkillDisambiguation { name, candidates } => write!(
                formatter,
                "Skill reference ${name} is ambiguous: {}",
                serde_json::to_string(candidates).unwrap_or_else(|_| "[]".to_string())
            ),
            Self::LimitExceeded { resource, limit } => {
                write!(formatter, "agent {resource} limit exceeded ({limit})")
            }
            Self::Model(message) => write!(formatter, "agent model failed: {message}"),
            Self::ModelRequestRetriesExhausted { attempts, message } => write!(
                formatter,
                "agent model request retries exhausted after {attempts} attempt(s): {message}"
            ),
            Self::ModelStreamInterrupted(message) => {
                write!(
                    formatter,
                    "agent model response stream interrupted: {message}"
                )
            }
            Self::ModelStreamRetriesExhausted { retries, message } => write!(
                formatter,
                "agent model response stream retries exhausted after {retries} retry(s): {message}"
            ),
            Self::UnsupportedImageInput {
                status,
                code,
                message,
                request_id,
            } => write!(
                formatter,
                "agent model endpoint does not support image input status={status:?} code={code:?} request_id={request_id:?}: {message}"
            ),
            Self::UnsupportedReasoning {
                status,
                code,
                message,
                request_id,
            } => write!(
                formatter,
                "agent model endpoint does not support the reasoning parameter status={status:?} code={code:?} request_id={request_id:?}: {message}"
            ),
            Self::ProviderContextWindowExceeded {
                status,
                code,
                message,
                request_id,
            } => write!(
                formatter,
                "agent model context window exceeded status={status:?} code={code:?} request_id={request_id:?}: {message}"
            ),
            Self::ContextWindowExceededAfterCompaction { message } => write!(
                formatter,
                "the conversation is still too long for the model's context window after it was compacted; start a new conversation or attach less context ({message})"
            ),
            Self::Protocol(message) => write!(formatter, "agent model protocol failed: {message}"),
            Self::UsageOverflow { field } => {
                write!(formatter, "agent model usage overflowed {field}")
            }
            Self::ContextBudgetExceeded {
                consumed,
                limit,
                source,
            } => write!(
                formatter,
                "agent context budget exceeded: {consumed} of {limit} tokens ({source:?})"
            ),
            Self::CompactionDidNotReduceContext { before, after } => write!(
                formatter,
                "agent compaction did not reduce context: replacement uses {after} tokens versus {before} before compaction"
            ),
            Self::CompactionAlreadyRunning => {
                formatter.write_str("agent compaction is already running")
            }
            Self::CompactionNotRunning => formatter.write_str("agent compaction is not running"),
            Self::StaleCompaction => formatter.write_str("stale agent compaction identity"),
            Self::CompactionUnavailable => formatter.write_str(
                "agent conversation does not contain a safe prefix that can be compacted",
            ),
            Self::Persistence(message) => write!(formatter, "agent persistence failed: {message}"),
            Self::Internal(message) => write!(formatter, "agent internal error: {message}"),
            Self::Cancelled => formatter.write_str("agent turn cancelled"),
            Self::TurnDeadlineExceeded { deadline }
                if deadline.as_secs() >= 60 && deadline.as_secs() % 60 == 0 =>
            {
                write!(
                formatter,
                "agent turn stopped at its time limit of {} minutes; send a message to continue",
                deadline.as_secs() / 60
            )
            }
            Self::TurnDeadlineExceeded { deadline } => write!(
                formatter,
                "agent turn stopped at its time limit of {} s; send a message to continue",
                deadline.as_secs_f64()
            ),
            Self::CancellationNotConfirmed => {
                formatter.write_str("agent turn cancellation was not confirmed")
            }
            Self::SessionMutationNotConfirmed => {
                formatter.write_str("agent session has a non-terminal turn")
            }
            Self::ShuttingDown => formatter.write_str("agent service is shutting down"),
            Self::FeatureUnavailable { reason, .. } => formatter.write_str(reason),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentShutdownDiagnostic {
    pub message: String,
}

impl std::error::Error for AgentError {}
