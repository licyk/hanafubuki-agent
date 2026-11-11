//! A guest's state as the guest host tracks it, and as its status reports
//! it.

use std::sync::Arc;
use std::time::Instant;

use hanafubuki_agent_contracts::ContextSectionKind;
use hanafubuki_agent_host::{RestartPolicy, RestartWindow};
use kardia_hmr::{ExitCause, GuestExit, GuestHandle};
use serde::Serialize;

/// Where a guest is in its lifecycle.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum GuestState {
    /// Not running: its package is off, or the guest host is.
    Stopped,
    /// Not started: its runtime does not run here, or its permissions are
    /// not granted.
    Blocked,
    Starting,
    Running,
    /// It failed while running, and restarts after the restart policy's
    /// delay.
    Restarting,
    /// It stays failed until it is retried: its start failed, or it failed
    /// more often than the restart policy allows.
    Failed,
}

/// A guest's status: its state, why it is not running, and how its last
/// process ended.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GuestStatus {
    pub state: GuestState,
    /// Why it is blocked or failed. It can quote the guest's own words.
    pub error: Option<String>,
    /// How its last process ended, once one has.
    pub last_exit: Option<GuestExitReport>,
    /// The end of what its process wrote to stderr (at most 16 KiB): the
    /// running process's, else the last one's.
    pub stderr_tail: String,
    /// Its automatic restarts within the restart policy's window.
    pub restarts: usize,
    /// The running process's id.
    pub process_id: Option<u32>,
    /// What of its declared contributions was left out when it last
    /// started, and what the Agent's calls into it went without (a context
    /// section it did not give in time, say).
    pub issues: Vec<String>,
}

/// How a guest's process ended.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GuestExitReport {
    pub cause: GuestExitCause,
    /// Why the Agent killed it: a call past its deadline, a broken
    /// protocol. It can quote the guest's own words.
    pub reason: Option<String>,
    /// The exit status as the platform reports it (`exit status: 1`,
    /// `signal: 9 (SIGKILL)`); none when it could not be observed.
    pub status: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum GuestExitCause {
    /// It exited on its own.
    Exited,
    /// The Agent killed it.
    Killed,
    /// Its fiber stopped it.
    Disposed,
}

impl GuestExitReport {
    fn of(exit: &GuestExit) -> Self {
        let (cause, reason) = match &exit.cause {
            ExitCause::Exited => (GuestExitCause::Exited, None),
            ExitCause::Killed(reason) => (GuestExitCause::Killed, Some(reason.clone())),
            ExitCause::Disposed => (GuestExitCause::Disposed, None),
            _ => (GuestExitCause::Exited, None),
        };
        Self {
            cause,
            reason,
            status: exit.status.map(|status| status.to_string()),
        }
    }
}

/// What the guest host keeps of one guest.
pub(crate) struct GuestRecord {
    pub(crate) state: GuestState,
    pub(crate) error: Option<String>,
    /// The process of its current (or last) start.
    pub(crate) handle: Option<GuestHandle>,
    pub(crate) last_exit: Option<Arc<GuestExit>>,
    /// Whether its current start became active: a failure after that is a
    /// fault the restart loop answers, one before it a failed start.
    pub(crate) started: bool,
    pub(crate) window: RestartWindow,
    /// What its last start left out of its contributions.
    pub(crate) issues: Vec<String>,
    /// The kind of its prompt fragments while it runs and has a prompt.
    pub(crate) prompt: Option<ContextSectionKind>,
    /// Changed by every decision about the guest (a new package, a retry, a
    /// scheduled restart), so a restart scheduled before it does not run.
    pub(crate) epoch: u64,
    /// The revision of its linked directory its entry is named after.
    pub(crate) revision: u64,
}

impl GuestRecord {
    pub(crate) fn new(state: GuestState) -> Self {
        Self {
            state,
            error: None,
            handle: None,
            last_exit: None,
            started: false,
            window: RestartWindow::default(),
            issues: Vec::new(),
            prompt: None,
            epoch: 0,
            revision: 0,
        }
    }

    pub(crate) fn status(&self, policy: &RestartPolicy) -> GuestStatus {
        let running = matches!(self.state, GuestState::Starting | GuestState::Running);
        let live = self
            .handle
            .as_ref()
            .filter(|handle| running && handle.exit().is_none());
        let tail = match (live, &self.last_exit) {
            (Some(handle), _) => handle.stderr_tail(),
            (None, Some(exit)) => exit.stderr_tail.clone(),
            (None, None) => Vec::new(),
        };
        GuestStatus {
            state: self.state,
            error: self.error.clone(),
            last_exit: self.last_exit.as_deref().map(GuestExitReport::of),
            stderr_tail: String::from_utf8_lossy(&tail).into_owned(),
            restarts: self.window.count(policy, Instant::now()),
            process_id: live.and_then(GuestHandle::id),
            issues: self.issues.clone(),
        }
    }
}
