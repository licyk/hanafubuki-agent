use std::collections::BTreeMap;
use std::fmt;
use std::path::PathBuf;

use hanafubuki_agent_approval::{ApprovalOrigin, SandboxEscalation};
use hanafubuki_agent_foundation::terminal_text::TextStyles;
use hanafubuki_agent_process::executor::{ExecutorIoMode, ExecutorStatus, OutputStream};
use hanafubuki_agent_process::terminal_size::{TerminalResize, TerminalSize};
use hanafubuki_agent_sandbox::{SandboxError, SandboxSettings};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct AgentExecutionId(u64);

impl AgentExecutionId {
    pub fn from_untrusted(value: u64) -> Result<Self, AgentExecutionError> {
        if value == 0 {
            return Err(AgentExecutionError::InvalidRequest(
                "execution id must be non-zero".to_string(),
            ));
        }
        Ok(Self(value))
    }

    pub const fn value(self) -> u64 {
        self.0
    }

    pub(crate) const fn new(value: u64) -> Self {
        Self(value)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct AgentExecutionGeneration(u64);

impl AgentExecutionGeneration {
    pub fn from_untrusted(value: u64) -> Result<Self, AgentExecutionError> {
        if value == 0 {
            return Err(AgentExecutionError::InvalidRequest(
                "execution generation must be non-zero".to_string(),
            ));
        }
        Ok(Self(value))
    }

    pub const fn value(self) -> u64 {
        self.0
    }

    pub(crate) const fn initial() -> Self {
        Self(1)
    }
}

impl fmt::Display for AgentExecutionId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

/// Exact Agent conversation scope allowed to address an execution.
///
/// The numeric value is wrapped here rather than compared with an executor id.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct AgentExecutionScope {
    session_id: u64,
}

impl AgentExecutionScope {
    pub fn for_agent_session(session_id: u64) -> Result<Self, AgentExecutionError> {
        if session_id == 0 {
            return Err(AgentExecutionError::InvalidRequest(
                "Agent execution session scope must be non-zero".to_string(),
            ));
        }
        Ok(Self { session_id })
    }

    pub fn session_id(&self) -> u64 {
        self.session_id
    }
}

/// Generic argv command request; the harness of the crate's contract tests.
#[cfg(any(test, feature = "test-support"))]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StartCommandRequest {
    pub argv: Vec<String>,
    pub cwd: Option<PathBuf>,
    pub environment: BTreeMap<String, String>,
    pub timeout_ms: Option<u64>,
    pub output_bytes_cap: Option<usize>,
    pub origin: ApprovalOrigin,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlatformShell {
    Posix,
    PowerShell,
    Cmd,
}

/// Codex-compatible command request after model arguments have crossed the
/// typed application boundary.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExecCommandRequest {
    pub cmd: String,
    pub shell: Option<PlatformShell>,
    pub login: bool,
    pub tty: bool,
    pub yield_time_ms: u64,
    pub max_output_bytes: Option<usize>,
    /// How long the command may run from its start before it is stopped
    /// (`TimedOut`), in milliseconds; at least 1, since the executor takes 0
    /// as no limit. Part of what its approval approves.
    pub timeout_ms: u64,
    /// Where the command runs: absolute, or relative to the service's
    /// working directory; `None` is that directory.
    pub workdir: Option<PathBuf>,
    pub environment: BTreeMap<String, String>,
    pub origin: ApprovalOrigin,
    /// The sandbox the command runs in, over the service's workspace roots.
    pub sandbox: SandboxSettings,
    /// Set when the model asked to run the command outside `sandbox`: its
    /// approval says so, and once approved it runs unconfined.
    pub sandbox_escalation: Option<SandboxEscalation>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WriteStdinRequest {
    pub session_id: AgentExecutionId,
    pub chars: String,
    pub yield_time_ms: u64,
    pub max_output_bytes: Option<usize>,
}

/// Result of one Unified Exec turn, as the model and the shell both receive it.
///
/// Self-describing like [`hanafubuki_agent_workspace_fs::WorkspaceFsResult`]: the `type` tag
/// is what lets a consumer recognize a command result among the structured tool
/// outputs it may be handed, instead of guessing from which fields happen to be
/// present. The application shell relies on it to render the command's output
/// as terminal text rather than as a JSON blob.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename = "unified_exec", rename_all = "camelCase")]
pub struct UnifiedExecResult {
    pub output: String,
    pub exit_code: Option<i32>,
    pub session_id: Option<u64>,
    pub wall_time_ms: u64,
    pub truncated: bool,
    /// The styles `output` was written in (SGR colours and attributes), for
    /// a display; `None` when it has none. Never serialized: the JSON is
    /// what the model reads.
    #[serde(skip)]
    pub styles: Option<TextStyles>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentExecutionKind {
    Command,
    UnifiedExec,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentExecutionStatus {
    Starting,
    Running,
    Exited,
    Failed,
    Cancelled,
    TimedOut,
}

impl AgentExecutionStatus {
    pub fn is_terminal(self) -> bool {
        !matches!(self, Self::Starting | Self::Running)
    }
}

impl From<ExecutorStatus> for AgentExecutionStatus {
    fn from(value: ExecutorStatus) -> Self {
        match value {
            ExecutorStatus::Starting => Self::Starting,
            ExecutorStatus::Running => Self::Running,
            ExecutorStatus::Exited => Self::Exited,
            ExecutorStatus::Failed => Self::Failed,
            ExecutorStatus::Cancelled => Self::Cancelled,
            ExecutorStatus::TimedOut => Self::TimedOut,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentExecutionSnapshot {
    pub execution_id: AgentExecutionId,
    pub generation: AgentExecutionGeneration,
    pub kind: AgentExecutionKind,
    pub status: AgentExecutionStatus,
    pub exit_code: Option<i32>,
    pub output_offset: u64,
    pub created_at_ms: u128,
    pub updated_at_ms: u128,
    pub cleanup_required: bool,
    pub interactive: bool,
    pub last_error: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentBackgroundTerminalSnapshot {
    pub execution_id: AgentExecutionId,
    pub generation: AgentExecutionGeneration,
    pub command: String,
    pub cwd: String,
    pub status: AgentExecutionStatus,
    pub exit_code: Option<i32>,
    pub created_at_ms: u128,
    pub updated_at_ms: u128,
    pub cleanup_required: bool,
    pub recent_output: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentBackgroundTerminalStopFailure {
    pub execution_id: AgentExecutionId,
    pub generation: AgentExecutionGeneration,
    pub message: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentBackgroundTerminalStopOutcome {
    pub disposed: Vec<ConfirmedAgentExecutionDisposal>,
    pub retained: Vec<AgentBackgroundTerminalStopFailure>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentExecutionOutputStream {
    Stdout,
    Stderr,
    Merged,
}

impl From<OutputStream> for AgentExecutionOutputStream {
    fn from(value: OutputStream) -> Self {
        match value {
            OutputStream::Stdout => Self::Stdout,
            OutputStream::Stderr => Self::Stderr,
            OutputStream::Merged => Self::Merged,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentExecutionOutputChunk {
    pub stream: AgentExecutionOutputStream,
    pub text: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentExecutionOutput {
    pub execution_id: AgentExecutionId,
    pub status: AgentExecutionStatus,
    pub exit_code: Option<i32>,
    pub chunks: Vec<AgentExecutionOutputChunk>,
    pub start_offset: u64,
    pub next_offset: u64,
    pub truncated: bool,
    /// The PTY's size at `start_offset`; `None` for pipes.
    #[serde(default)]
    pub terminal_size: Option<TerminalSize>,
    /// The PTY's size changes inside the output, in offset order.
    #[serde(default)]
    pub resizes: Vec<TerminalResize>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentExecutionWaitOutcome {
    pub snapshot: AgentExecutionSnapshot,
    pub completed: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfirmedAgentExecutionDisposal {
    pub execution_id: AgentExecutionId,
    pub generation: AgentExecutionGeneration,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InteractiveResizeAccepted {
    pub execution_id: AgentExecutionId,
    pub generation: AgentExecutionGeneration,
    pub cols: u16,
    pub rows: u16,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AgentExecutionError {
    InvalidRequest(String),
    Resolution(String),
    ApprovalRequired,
    ApprovalMismatch,
    NotFound,
    CrossScope,
    StaleGeneration,
    NotInteractive,
    Cancelled,
    Process(String),
    /// The command asked for a sandbox this machine cannot give it, and
    /// was not run.
    Sandbox(SandboxError),
    CleanupRetained {
        execution_id: AgentExecutionId,
        message: String,
    },
}

impl fmt::Display for AgentExecutionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidRequest(message) => write!(formatter, "invalid Agent command: {message}"),
            Self::Resolution(message) => write!(formatter, "resolve Agent command: {message}"),
            Self::ApprovalRequired => {
                formatter.write_str("Agent command requires an exact consumed approval")
            }
            Self::ApprovalMismatch => {
                formatter.write_str("Agent command approval does not match the resolved plan")
            }
            Self::NotFound => formatter.write_str("Agent execution not found"),
            Self::CrossScope => formatter.write_str("Agent execution belongs to another session"),
            Self::StaleGeneration => formatter.write_str("Agent execution generation is stale"),
            Self::NotInteractive => formatter.write_str("Agent execution is not interactive"),
            Self::Cancelled => formatter.write_str("Agent execution request cancelled"),
            Self::Process(message) => write!(formatter, "Agent execution process error: {message}"),
            Self::Sandbox(error) => error.fmt(formatter),
            Self::CleanupRetained {
                execution_id,
                message,
            } => write!(
                formatter,
                "Agent execution {execution_id} cleanup remains retryable: {message}"
            ),
        }
    }
}

impl std::error::Error for AgentExecutionError {}

pub(crate) struct ResolvedCommandPlan {
    pub command: Vec<String>,
    pub cwd: PathBuf,
    pub environment: BTreeMap<String, String>,
    pub timeout_ms: u64,
    pub output_bytes_cap: usize,
    pub io_mode: ExecutorIoMode,
    pub cols: Option<u16>,
    pub rows: Option<u16>,
    /// A PTY session that accepts stdin writes and resizes.
    pub interactive: bool,
}
