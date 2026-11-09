use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use hanafubuki_agent_approval::AuthorizedAction;
use hanafubuki_agent_foundation::cancellation::CancellationToken;
use hanafubuki_agent_foundation::terminal_text::{complete_prefix_len, render_plain_text};
use hanafubuki_agent_process::executor::manager::{
    dispose_reserved_executor, finalize_executor_reservation_handle, get_reserved_executor_info,
    interrupt_reserved_executor, read_reserved_executor_output, reserve_executor_id,
    resize_reserved_executor_pty, start_reserved_executor, terminate_reserved_executor,
    write_reserved_executor_stdin,
};
use hanafubuki_agent_process::executor::{
    EnvironmentInheritance, ExecutorInfo, ExecutorIoMode, ExecutorManager, ExecutorOutput,
    ExecutorStartRequest,
};
use hanafubuki_agent_process::ownership::{
    OwnedReleaseStatus, ProcessOwnerId, ProcessReceipt, ProcessReservation,
};
use hanafubuki_agent_sandbox::{Sandbox, SandboxPolicy};
use parking_lot::{Condvar, Mutex, RwLock};

use crate::model::{
    AgentBackgroundTerminalSnapshot, AgentBackgroundTerminalStopFailure,
    AgentBackgroundTerminalStopOutcome, AgentExecutionError, AgentExecutionGeneration,
    AgentExecutionId, AgentExecutionKind, AgentExecutionOutput, AgentExecutionOutputChunk,
    AgentExecutionScope, AgentExecutionSnapshot, AgentExecutionWaitOutcome,
    ConfirmedAgentExecutionDisposal, ExecCommandRequest, InteractiveResizeAccepted,
    ResolvedCommandPlan, UnifiedExecResult, WriteStdinRequest,
};
use crate::resolver::{MAX_OUTPUT_READ_BYTES, MAX_WAIT_MS};
use crate::shell::prepare_exec_command;

const WAIT_POLL: Duration = Duration::from_millis(20);
const BACKGROUND_OUTPUT_BYTES: usize = 8 * 1024;
const BACKGROUND_OUTPUT_LINES: usize = 3;
const BACKGROUND_OUTPUT_LINE_BYTES: usize = 512;
const MAX_COMMAND_DISPLAY_BYTES: usize = 4 * 1024;
const MAX_CWD_DISPLAY_BYTES: usize = 4 * 1024;
const MAX_EXECUTION_DIAGNOSTIC_BYTES: usize = 1024;
/// Longest a finished execution's disposal waits for its observer to report
/// the final output, so a stuck reporter cannot hold cleanup.
const OBSERVER_DRAIN_TIMEOUT: Duration = Duration::from_secs(2);

mod background;
mod interactive;
mod lifecycle;
mod observer;
mod output;
mod process_port;
mod registry;
mod shutdown;
mod start;
mod unified;

use output::{bound_utf8, recent_output_lines};
use process_port::executor_request;
use registry::{now_millis, ExecutionRegistry};

pub use process_port::{AgentExecutionProcessPort, ExecutorAgentExecutionPort};

#[derive(Clone)]
pub struct AgentExecutionService {
    inner: Arc<AgentExecutionServiceInner>,
}

struct AgentExecutionServiceInner {
    process: Arc<dyn AgentExecutionProcessPort>,
    registry: Mutex<ExecutionRegistry>,
    /// Signalled with `registry` whenever an execution's observer is released.
    observer_released: Condvar,
    /// The workspace roots, which the application sets, and sets again when
    /// the workspace changes. The first (the primary root) is where a
    /// command without an absolute `workdir` runs; all are writable in a
    /// workspace-write sandbox.
    workspace_roots: RwLock<Vec<PathBuf>>,
    /// What confines a command that asks for a sandbox.
    sandbox: Sandbox,
}

impl AgentExecutionService {
    pub fn ensure_generation(
        &self,
        scope: &AgentExecutionScope,
        execution_id: AgentExecutionId,
        generation: AgentExecutionGeneration,
    ) -> Result<(), AgentExecutionError> {
        let registry = self.inner.registry.lock();
        let record = registry
            .records
            .get(&execution_id)
            .ok_or(AgentExecutionError::NotFound)?;
        if &record.scope != scope {
            return Err(AgentExecutionError::CrossScope);
        }
        if record.generation != generation {
            return Err(AgentExecutionError::StaleGeneration);
        }
        Ok(())
    }
    pub fn new(executor: ExecutorManager) -> Self {
        Self::with_process_port(Arc::new(ExecutorAgentExecutionPort::new(executor)))
    }

    /// The service over `process`, with this machine's sandbox
    /// ([`Sandbox::detect`]).
    pub fn with_process_port(process: Arc<dyn AgentExecutionProcessPort>) -> Self {
        Self::with_sandbox(process, Sandbox::detect())
    }

    pub fn with_sandbox(process: Arc<dyn AgentExecutionProcessPort>, sandbox: Sandbox) -> Self {
        Self {
            inner: Arc::new(AgentExecutionServiceInner {
                process,
                registry: Mutex::new(ExecutionRegistry {
                    next_execution_id: 1,
                    records: HashMap::new(),
                }),
                observer_released: Condvar::new(),
                workspace_roots: RwLock::new(Vec::new()),
                sandbox,
            }),
        }
    }

    pub fn sandbox(&self) -> &Sandbox {
        &self.inner.sandbox
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn prepare(
        &self,
        request: &crate::StartCommandRequest,
    ) -> Result<crate::PreparedCommand, AgentExecutionError> {
        crate::prepare_command(request)
    }

    pub fn prepare_exec_command_action(
        &self,
        request: &ExecCommandRequest,
    ) -> Result<hanafubuki_agent_approval::ApprovalAction, AgentExecutionError> {
        let prepared = prepare_exec_command(request, self.working_directory().as_deref())?;
        // A command its sandbox cannot run is refused before anyone is
        // asked to approve it.
        self.sandboxed(prepared.plan.command.clone(), request)?;
        Ok(prepared.action().clone())
    }

    /// Sets the workspace roots, the primary first: the directory a command
    /// without an absolute `workdir` runs in (see
    /// [`ExecCommandRequest::workdir`]), and what a workspace-write sandbox
    /// lets commands write.
    pub fn set_workspace_roots(&self, roots: Vec<PathBuf>) {
        *self.inner.workspace_roots.write() = roots;
    }

    pub fn workspace_roots(&self) -> Vec<PathBuf> {
        self.inner.workspace_roots.read().clone()
    }

    pub fn working_directory(&self) -> Option<PathBuf> {
        self.inner.workspace_roots.read().first().cloned()
    }

    /// `command` as `request`'s sandbox confines it over the workspace
    /// roots; unconfined when the request asked to leave the sandbox (its
    /// approval said so).
    fn sandboxed(
        &self,
        command: Vec<String>,
        request: &ExecCommandRequest,
    ) -> Result<Vec<String>, AgentExecutionError> {
        let policy = match request.sandbox_escalation {
            Some(_) => SandboxPolicy::FullAccess,
            None => request.sandbox.policy(&self.workspace_roots()),
        };
        self.inner
            .sandbox
            .with_windows_sandbox(request.sandbox.windows_sandbox)
            .wrap(&policy, command)
            .map_err(AgentExecutionError::Sandbox)
    }
}
