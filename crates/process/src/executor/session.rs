//! Per-execution session state and the control-backend abstraction.

use std::process::ExitStatus;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use parking_lot::{Condvar, Mutex};

use super::model::{ExecutorInfo, ExecutorIoMode, ExecutorStatus};
use super::output::{pipe_fallback, OutputBuffer};
use crate::process_groups::{ProcessGroupRecord, ProcessOwnerGroup};
use crate::terminal_size::TerminalSize;

/// Control interface a backend (pipe or PTY) exposes for a running execution.
pub(crate) trait ExecutorBackend: Send + Sync {
    /// Queue input for the process. It never blocks on the process reading it,
    /// so a full queue is reported instead of stalling the caller.
    fn write_stdin(&self, bytes: Vec<u8>, close: bool) -> Result<(), String>;
    fn resize(&self, cols: u16, rows: u16) -> Result<(), String>;
    /// Interrupt the process using the backend's native interactive-control
    /// semantics without marking the execution as explicitly terminated.
    fn interrupt(&self) -> Result<(), String>;
    /// Request termination of the process tree and report control failures.
    fn terminate(&self) -> Result<(), String>;
    /// The process group the child leads (Unix: it was started in its own
    /// session), if any.
    fn process_group_leader(&self) -> Option<u32> {
        None
    }
}

pub(crate) struct SessionInner {
    pub status: ExecutorStatus,
    pub exit_code: Option<i32>,
    /// How the process ended, as the pipe backend observed it (a signal
    /// too, which `exit_code` cannot carry).
    pub exit_status: Option<ExitStatus>,
    pub error: Option<String>,
    pub ended_at_ms: Option<u128>,
}

/// Its locks do not poison: the Agent's tools read and resize a session from
/// threads where a panic unwinds, and each critical section leaves the
/// session consistent at any panic point (reads, a status step, a resize
/// recorded only after the PTY accepted it). The reader and waiter threads
/// that write output and the final status still abort on a panic.
pub struct ExecutorSession {
    pub executor_id: String,
    pub io_mode: ExecutorIoMode,
    pub command: Vec<String>,
    pub output: Arc<Mutex<OutputBuffer>>,
    pub(crate) inner: Mutex<SessionInner>,
    pub(crate) terminate_requested: AtomicBool,
    pub(crate) timed_out: AtomicBool,
    pub(crate) finished: AtomicBool,
    /// Cloned out for every call, so no control operation (a blocking
    /// terminate, a resize) holds this lock while it works.
    backend: Mutex<Option<Arc<dyn ExecutorBackend>>>,
    completion: Condvar,
    /// The owner group the session was started in.
    pub(crate) owner_group: Option<ProcessOwnerGroup>,
    /// The process group recorded for it, removed when the process ends.
    recorded_group: Mutex<Option<(ProcessGroupRecord, u32)>>,
}

impl ExecutorSession {
    pub(crate) fn new(
        executor_id: String,
        io_mode: ExecutorIoMode,
        command: Vec<String>,
        cap: usize,
        terminal_size: Option<TerminalSize>,
        owner_group: Option<ProcessOwnerGroup>,
    ) -> Self {
        Self {
            executor_id,
            io_mode,
            command,
            output: Arc::new(Mutex::new(OutputBuffer::new(
                cap,
                terminal_size,
                match io_mode {
                    ExecutorIoMode::Pipe => pipe_fallback(),
                    ExecutorIoMode::Pty => None,
                },
            ))),
            inner: Mutex::new(SessionInner {
                status: ExecutorStatus::Starting,
                exit_code: None,
                exit_status: None,
                error: None,
                ended_at_ms: None,
            }),
            terminate_requested: AtomicBool::new(false),
            timed_out: AtomicBool::new(false),
            finished: AtomicBool::new(false),
            backend: Mutex::new(None),
            completion: Condvar::new(),
            owner_group,
            recorded_group: Mutex::new(None),
        }
    }

    /// Records the process group `leader` leads in the owner group's record,
    /// if it has one. A process that already ended is forgotten again at
    /// once, so the record never keeps a group whose end it missed.
    pub(crate) fn record_process_group(&self, leader: u32) {
        let Some(record) = self
            .owner_group
            .as_ref()
            .and_then(ProcessOwnerGroup::record)
        else {
            return;
        };
        if !record.add(leader) {
            return;
        }
        let mut recorded = self.recorded_group.lock();
        if self.finished.load(Ordering::Acquire) {
            drop(recorded);
            record.remove(leader);
        } else {
            *recorded = Some((record.clone(), leader));
        }
    }

    pub fn info(&self) -> ExecutorInfo {
        let inner = self.inner.lock();
        ExecutorInfo {
            executor_id: self.executor_id.clone(),
            status: inner.status,
            io_mode: self.io_mode,
            command: self.command.clone(),
            exit_code: inner.exit_code,
            error: inner.error.clone(),
            output_offset: {
                let mut output = self.output.lock();
                output.release_held(Instant::now());
                output.next_offset()
            },
        }
    }

    /// Store the control backend once the process is spawned. If a termination
    /// (manual or timeout) was already requested before the backend existed, it
    /// is honored immediately so nothing can outlive a very small `timeout_ms`.
    ///
    /// `terminate` sets its flag before taking the backend lock and this checks
    /// the flag after storing under it, so one of the two always terminates.
    pub(crate) fn set_backend(&self, backend: Box<dyn ExecutorBackend>) {
        let backend: Arc<dyn ExecutorBackend> = Arc::from(backend);
        *self.backend.lock() = Some(backend.clone());
        if self.terminate_requested.load(Ordering::Acquire)
            || self.timed_out.load(Ordering::Acquire)
        {
            if let Err(error) = backend.terminate() {
                log::error!(
                    "executor early termination failed id={}: {error}",
                    self.executor_id
                );
            }
        }
    }

    pub(crate) fn backend(&self) -> Result<Arc<dyn ExecutorBackend>, String> {
        self.backend
            .lock()
            .clone()
            .ok_or_else(|| "executor has no active backend".to_string())
    }

    /// Drop the control backend after termination was confirmed, closing the
    /// PTY or pipes it owns so reader threads reach end of file instead of
    /// keeping the session alive. On Windows the final `ClosePseudoConsole`
    /// can wait for the console host, so the drop runs on its own thread.
    pub(crate) fn release_backend(&self) {
        let Some(backend) = self.backend.lock().take() else {
            return;
        };
        #[cfg(windows)]
        {
            let spawned = hanafubuki_agent_foundation::background_worker::spawn_contained_thread(
                hanafubuki_agent_foundation::module_faults::FaultOwner::Core,
                format!("exec-release-{}", self.executor_id),
                move || drop(backend),
            );
            if let Err(error) = spawned {
                log::warn!(
                    "executor backend release thread failed id={}: {error}",
                    self.executor_id
                );
            }
        }
        #[cfg(not(windows))]
        drop(backend);
    }

    /// Resize the PTY and record the change in its output, or do nothing when
    /// it already has `size`: a no-op resize would still make ConPTY repaint.
    ///
    /// The output lock is held across the resize so the reader cannot push a
    /// byte produced at the new size ahead of the record. Bytes the program
    /// wrote before the resize but the reader has not yet drained still land
    /// after it; no PTY reports where its pipe was at the moment of a resize.
    pub(crate) fn resize(&self, size: TerminalSize) -> Result<(), String> {
        let backend = self.backend()?;
        let mut output = self.output.lock();
        if output.terminal_size() == Some(size) {
            return Ok(());
        }
        backend.resize(size.cols, size.rows)?;
        output.record_resize(size);
        Ok(())
    }

    pub(crate) fn set_running(&self) {
        let mut inner = self.inner.lock();
        if inner.status == ExecutorStatus::Starting {
            inner.status = ExecutorStatus::Running;
        }
    }

    /// Record the terminal outcome (called once by the waiter thread). The final
    /// status reflects timeout / cancellation / spawn error / normal exit.
    pub(crate) fn finalize(&self, exit_code: Option<i32>, error: Option<String>) {
        self.finalize_with(exit_code, error, None);
    }

    /// [`Self::finalize`] for a process whose exit status was observed.
    pub(crate) fn finalize_exited(&self, status: ExitStatus) {
        self.finalize_with(status.code(), None, Some(status));
    }

    fn finalize_with(
        &self,
        exit_code: Option<i32>,
        error: Option<String>,
        exit_status: Option<ExitStatus>,
    ) {
        let mut inner = self.inner.lock();
        if inner.status.is_terminal() {
            return;
        }
        let status = if self.timed_out.load(Ordering::Acquire) {
            ExecutorStatus::TimedOut
        } else if self.terminate_requested.load(Ordering::Acquire) {
            ExecutorStatus::Cancelled
        } else if error.is_some() {
            ExecutorStatus::Failed
        } else {
            ExecutorStatus::Exited
        };
        inner.status = status;
        inner.exit_code = exit_code;
        inner.exit_status = exit_status;
        inner.error = error;
        inner.ended_at_ms = Some(now_ms());
        self.finished.store(true, Ordering::Release);
        drop(inner);
        self.completion.notify_all();
        let recorded = self.recorded_group.lock().take();
        if let Some((record, leader)) = recorded {
            record.remove(leader);
        }
    }

    pub(crate) fn terminate(&self) -> Result<(), String> {
        let started = Instant::now();
        log::info!(
            "agent_diag stage=executor.session_terminate.begin executor_id={}",
            self.executor_id
        );
        self.terminate_requested.store(true, Ordering::Release);
        let lock_started = Instant::now();
        log::info!(
            "agent_diag stage=executor.session_terminate.backend_lock.begin executor_id={}",
            self.executor_id
        );
        let backend = self.backend.lock().clone();
        log::info!(
            "agent_diag stage=executor.session_terminate.backend_lock.end executor_id={} elapsed_ms={}",
            self.executor_id,
            lock_started.elapsed().as_millis()
        );
        if let Some(backend) = backend {
            backend.terminate()?;
        }
        log::info!(
            "agent_diag stage=executor.session_terminate.end executor_id={} outcome=ok elapsed_ms={}",
            self.executor_id,
            started.elapsed().as_millis()
        );
        Ok(())
    }

    pub(crate) fn is_finished(&self) -> bool {
        self.finished.load(Ordering::Acquire)
    }

    pub(crate) fn wait_for_completion(&self, timeout: Duration) -> bool {
        let mut inner = self.inner.lock();
        if inner.status.is_terminal() {
            return true;
        }
        let _ = self.completion.wait_while_for(
            &mut inner,
            |state| !state.status.is_terminal(),
            timeout,
        );
        inner.status.is_terminal()
    }
}

pub(crate) fn now_ms() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0)
}
