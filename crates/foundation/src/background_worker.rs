use std::panic::{catch_unwind, resume_unwind, AssertUnwindSafe};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use parking_lot::Mutex;

use crate::cancellation::CancellationToken;
use crate::module_faults::{report_module_fault, FaultOwner};
use crate::panic::{mark_thread_contained, panic_message};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkerShutdownDiagnostic {
    pub worker: String,
    pub message: String,
}

struct Worker {
    name: String,
    cancellation: CancellationToken,
    handle: Option<JoinHandle<()>>,
    exit: BackgroundWorker,
}

/// How one member of a [`BackgroundWorkerGroup`] is doing. A member learns
/// through it that its worker thread has ended, and whether by a panic.
#[derive(Clone, Default)]
pub struct BackgroundWorker {
    exit: Arc<OnceLock<WorkerExit>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum WorkerExit {
    Returned,
    Panicked(String),
}

impl BackgroundWorker {
    /// Whether the worker's body has ended, by returning or by a panic.
    pub fn has_exited(&self) -> bool {
        self.exit.get().is_some()
    }

    /// The message of the panic that ended the worker, if one did.
    pub fn panic_message(&self) -> Option<String> {
        match self.exit.get() {
            Some(WorkerExit::Panicked(message)) => Some(message.clone()),
            Some(WorkerExit::Returned) | None => None,
        }
    }
}

/// Told the name of a member whose body panicked and the panic's message,
/// on the member's own thread as it ends; it only hands the report on.
pub type WorkerPanicHandler = Arc<dyn Fn(&str, &str) + Send + Sync>;

/// Owns named background workers and provides one bounded shutdown contract.
struct WorkerGroupInner {
    accepting: AtomicBool,
    /// Whether every member thread is marked contained.
    contained: bool,
    /// Told of every member that panicked.
    panic_handler: Option<WorkerPanicHandler>,
    workers: Mutex<Vec<Worker>>,
}

impl WorkerGroupInner {
    fn new(contained: bool, panic_handler: Option<WorkerPanicHandler>) -> Self {
        Self {
            accepting: AtomicBool::new(true),
            contained,
            panic_handler,
            workers: Mutex::new(Vec::new()),
        }
    }
}

impl Default for WorkerGroupInner {
    fn default() -> Self {
        Self::new(false, None)
    }
}

impl Drop for WorkerGroupInner {
    fn drop(&mut self) {
        for worker in self.workers.get_mut().iter() {
            worker.cancellation.cancel();
        }
    }
}

#[derive(Clone, Default)]
pub struct BackgroundWorkerGroup {
    inner: Arc<WorkerGroupInner>,
}

impl BackgroundWorkerGroup {
    /// A group whose member threads are marked contained
    /// ([`crate::panic::mark_thread_contained`]): a panic in a member unwinds
    /// to the group instead of aborting the process. The application's
    /// groups are contained; a default group's panic aborts the process once
    /// the application's panic hook is installed.
    pub fn contained() -> Self {
        Self {
            inner: Arc::new(WorkerGroupInner::new(true, None)),
        }
    }

    /// [`Self::contained`], telling `handler` of every member that panicked
    /// (after the panic is logged): the owner of the group learns that a
    /// worker it relies on is gone. A non-Agent group's handler is
    /// [`crate::module_faults::module_fault_handler`].
    pub fn contained_with_panic_handler(handler: WorkerPanicHandler) -> Self {
        Self {
            inner: Arc::new(WorkerGroupInner::new(true, Some(handler))),
        }
    }

    /// Starts `run` on a named thread the group owns. Fails when the group is
    /// shutting down or the operating system cannot create the thread.
    ///
    /// A panic in `run` that unwinds (in a [`Self::contained`] group, or
    /// anywhere without the application's panic hook) ends only that worker:
    /// it is logged at error with the worker's name and shutdown lists it as
    /// a diagnostic.
    pub fn spawn(
        &self,
        name: impl Into<String>,
        run: impl FnOnce(CancellationToken) + Send + 'static,
    ) -> Result<(), String> {
        self.spawn_with_cancellation(name, CancellationToken::default(), run)
            .map(|_| ())
    }

    /// `spawn`, returning the member's [`BackgroundWorker`], through which
    /// it learns that the worker ended and whether by a panic.
    pub fn spawn_observed(
        &self,
        name: impl Into<String>,
        run: impl FnOnce(CancellationToken) + Send + 'static,
    ) -> Result<BackgroundWorker, String> {
        self.start(name.into(), CancellationToken::default(), run)
    }

    /// `spawn_observed`, with a cancellation token the caller also holds:
    /// the caller can stop the member on its own and learn when it ended,
    /// and the group's shutdown still cancels and joins it.
    pub fn spawn_with_cancellation(
        &self,
        name: impl Into<String>,
        cancellation: CancellationToken,
        run: impl FnOnce(CancellationToken) + Send + 'static,
    ) -> Result<BackgroundWorker, String> {
        self.start(name.into(), cancellation, run)
    }

    fn start(
        &self,
        name: String,
        cancellation: CancellationToken,
        run: impl FnOnce(CancellationToken) + Send + 'static,
    ) -> Result<BackgroundWorker, String> {
        if !self.inner.accepting.load(Ordering::Acquire) {
            return Err(format!("background worker group is shutting down: {name}"));
        }
        let mut workers = self.inner.workers.lock();
        if !self.inner.accepting.load(Ordering::Acquire) {
            return Err(format!("background worker group is shutting down: {name}"));
        }
        let worker_token = cancellation.clone();
        let thread_name = name.clone();
        let exit = BackgroundWorker::default();
        let observed = exit.clone();
        let contained = self.inner.contained;
        let panic_handler = self.inner.panic_handler.clone();
        let handle = thread::Builder::new()
            .name(thread_name.clone())
            .spawn(move || {
                if contained {
                    mark_thread_contained();
                }
                // Not `catch_panic`: that would contain every member's panic,
                // and only a contained group's members are written for it.
                let ended = match catch_unwind(AssertUnwindSafe(|| run(worker_token))) {
                    Ok(()) => WorkerExit::Returned,
                    Err(payload) => {
                        let message = panic_message(payload.as_ref());
                        log::error!("background worker {thread_name} panicked: {message}");
                        if let Some(handler) = &panic_handler {
                            // A panicking handler must not take the exit
                            // record with it.
                            let _ = crate::panic::catch_panic(|| handler(&thread_name, &message));
                        }
                        WorkerExit::Panicked(message)
                    }
                };
                let _ = observed.exit.set(ended);
            })
            .map_err(|error| format!("spawn background worker {name}: {error}"))?;
        workers.push(Worker {
            name,
            cancellation,
            handle: Some(handle),
            exit: exit.clone(),
        });
        Ok(exit)
    }

    pub fn shutdown(&self, deadline: Instant) -> Vec<WorkerShutdownDiagnostic> {
        self.inner.accepting.store(false, Ordering::Release);
        let mut workers = self.inner.workers.lock();
        for worker in workers.iter() {
            worker.cancellation.cancel();
        }
        let mut diagnostics = Vec::new();
        for worker in workers.iter_mut() {
            let Some(handle) = worker.handle.take() else {
                continue;
            };
            while !handle.is_finished() && Instant::now() < deadline {
                thread::sleep(Duration::from_millis(5));
            }
            if handle.is_finished() {
                let joined = handle.join();
                if let Some(message) = worker.exit.panic_message() {
                    diagnostics.push(WorkerShutdownDiagnostic {
                        worker: worker.name.clone(),
                        message: format!("worker panicked: {message}"),
                    });
                } else if joined.is_err() {
                    diagnostics.push(WorkerShutdownDiagnostic {
                        worker: worker.name.clone(),
                        message: "worker panicked during shutdown".to_string(),
                    });
                }
            } else {
                diagnostics.push(WorkerShutdownDiagnostic {
                    worker: worker.name.clone(),
                    message: "shutdown deadline elapsed; worker remains detached".to_string(),
                });
            }
        }
        diagnostics
    }
}

/// Starts `run` on a thread named `name` that is marked contained, outside
/// any group: for a thread whose owner joins or detaches it itself (one per
/// process, request or operation). A panic in `run` is reported to the
/// module fault sink as a fault of `owner`, the thread named as its
/// component, then ends the thread as it did before: `join` returns it as
/// an error. Fails when the operating system cannot create the thread.
pub fn spawn_contained_thread<T: Send + 'static>(
    owner: FaultOwner,
    name: impl Into<String>,
    run: impl FnOnce() -> T + Send + 'static,
) -> std::io::Result<JoinHandle<T>> {
    let name = name.into();
    let component = name.clone();
    thread::Builder::new().name(name).spawn(move || {
        mark_thread_contained();
        match catch_unwind(AssertUnwindSafe(run)) {
            Ok(value) => value,
            Err(payload) => {
                report_module_fault(owner, &component, &panic_message(payload.as_ref()));
                // Not a new panic: the hook is not run again.
                resume_unwind(payload)
            }
        }
    })
}

#[cfg(test)]
mod tests;
