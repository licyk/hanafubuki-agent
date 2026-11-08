//! What the slot reports: where the Agent is, why it is not running, and
//! the error a caller without a running Agent returns.

use std::fmt;

use hanafubuki_agent_kernel::AgentQuarantinedSession;
use serde::Serialize;

/// Why the Agent is off.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum AgentDisabledReason {
    /// The `agent` module is turned off (`disabledModules`, at start-up), or
    /// was stopped since: its stop turns the Agent off in order.
    Setting,
    /// The last starts never finished (`AgentStartMarker`): the Agent stays
    /// off until the user restarts it or turns its module off and on.
    CrashLoop,
}

/// The start-up stage an Agent failure happened in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum AgentStartStage {
    /// Building the service: settings, workspace, stores, the plugin host.
    Construct,
    /// Loading the durable sessions.
    Recover,
    /// Starting the plugins and the MCP servers.
    Start,
}

impl fmt::Display for AgentStartStage {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Construct => "construction",
            Self::Recover => "session recovery",
            Self::Start => "start-up",
        })
    }
}

/// How one Agent instance starts.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AgentStartOptions {
    /// Every plugin outside the core is off for this instance; the saved
    /// `agentDisabledPlugins` is not changed.
    pub safe_mode: bool,
    /// The history index is discarded and rebuilt from the rollouts before
    /// the sessions are recovered. Applies to this start only.
    pub rebuild_history_index: bool,
}

/// Where the Agent is, as the interface and diagnostics see it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(
    tag = "state",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum AgentAvailability {
    Disabled {
        reason: AgentDisabledReason,
    },
    /// Being built, or about to be once the application is presented.
    Starting,
    Running,
    /// Kernel code panicked, so the Agent was stopped: its in-memory state
    /// is not trusted, and a restart rebuilds it from disk.
    Faulted {
        /// What panicked, and its message.
        reason: String,
        /// When, in milliseconds since the Unix epoch.
        at: u64,
        /// Automatic restarts already made in the restart window.
        restarts_in_window: u32,
        /// When the automatic restart runs (milliseconds since the Unix
        /// epoch); `None` once the window's restarts are used up, and the
        /// Agent stays stopped until the user restarts it.
        next_restart_at: Option<u64>,
    },
    /// Stopping, to restart or to turn off.
    Stopping,
    Failed {
        stage: AgentStartStage,
        reason: String,
    },
    /// The application is shutting down.
    Stopped,
}

impl AgentAvailability {
    pub(super) fn why(&self) -> String {
        match self {
            Self::Disabled {
                reason: AgentDisabledReason::Setting,
            } => "it is turned off in Settings".to_string(),
            Self::Disabled {
                reason: AgentDisabledReason::CrashLoop,
            } => "it was turned off because its last starts did not finish; restart it, or turn \
                  it off and on in Settings"
                .to_string(),
            Self::Starting => "it is still starting".to_string(),
            Self::Running => "it is running".to_string(),
            Self::Faulted {
                reason,
                next_restart_at: Some(_),
                ..
            } => format!("it stopped after an internal error and restarts shortly: {reason}"),
            Self::Faulted { reason, .. } => format!(
                "it stopped after repeated internal errors and stays stopped until it is \
                 restarted: {reason}"
            ),
            Self::Stopping => "it is stopping".to_string(),
            Self::Failed { stage, reason } => format!("its {stage} failed: {reason}"),
            Self::Stopped => "the application is shutting down".to_string(),
        }
    }
}

/// The slot's state with what recovery found, for `agent_status` and the
/// diagnostic bundle.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentStatus {
    #[serde(flatten)]
    pub availability: AgentAvailability,
    /// How many starts this run has made; the number of the running (or
    /// last) one.
    pub generation: u64,
    /// The Agent starting or running is in safe mode (core plugins only).
    pub safe_mode: bool,
    /// The running Agent has a plugin that failed, whose teardown is stuck,
    /// or that runs with a warning (part of it left out of turns); derived
    /// from the plugin status, which says which.
    pub degraded: bool,
    /// The sessions recovery skipped, and those the history refused since
    /// (a session list row that does not decode); their files are kept.
    pub quarantined_sessions: Vec<AgentQuarantinedSession>,
}

/// The Agent is not running; `Display` is the message every caller returns.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentUnavailable {
    pub availability: AgentAvailability,
}

impl fmt::Display for AgentUnavailable {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "the Agent is unavailable: {}",
            self.availability.why()
        )
    }
}

impl From<AgentUnavailable> for String {
    fn from(unavailable: AgentUnavailable) -> Self {
        unavailable.to_string()
    }
}
