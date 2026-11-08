//! Live progress a running tool reports to the activity monitor before its
//! output is committed.

use serde::{Deserialize, Serialize};

use crate::AgentActivityAction;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentExecutionProgressStatus {
    Starting,
    Running,
    Exited,
    Failed,
    Cancelled,
    TimedOut,
}

impl AgentExecutionProgressStatus {
    pub const fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Exited | Self::Failed | Self::Cancelled | Self::TimedOut
        )
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum AgentToolProgressUpdate {
    ExecutionStarted {
        execution_id: u64,
        generation: u64,
        action: AgentActivityAction,
        status: AgentExecutionProgressStatus,
    },
    ExecutionOutput {
        execution_id: u64,
        generation: u64,
        text: String,
        next_offset: u64,
        truncated: bool,
    },
    ExecutionStatus {
        execution_id: u64,
        generation: u64,
        status: AgentExecutionProgressStatus,
        exit_code: Option<i32>,
        cleanup_required: bool,
    },
    ExecutionDisposed {
        execution_id: u64,
        generation: u64,
    },
    ExecutionCleanupFailed {
        execution_id: u64,
        generation: u64,
    },
}

/// Receives a running tool's progress, from the tool's call or from a
/// thread that outlives it. Implementations must not block: the kernel's
/// sink only queues the update on the session.
pub trait AgentToolProgressSink: Send + Sync {
    fn report(&self, update: AgentToolProgressUpdate);
}

#[derive(Default)]
pub struct NoopAgentToolProgressSink;

impl AgentToolProgressSink for NoopAgentToolProgressSink {
    fn report(&self, _update: AgentToolProgressUpdate) {}
}
