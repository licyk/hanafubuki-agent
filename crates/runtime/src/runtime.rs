//! Every thread of a [`ContainedRuntime`], workers and blocking pool alike,
//! is marked contained, so a panic in work it runs unwinds to the task's join
//! handle or to the guard around it instead of aborting the process
//! (`hanafubuki_agent_foundation::panic`). What spawns from that work (a kardia
//! effect, `tokio::spawn`, `spawn_blocking`, `Handle::current()`) lands on
//! the same runtime, so it is contained too.
//!
//! Each Agent instance has its own (threads [`AGENT_RUNTIME_THREAD_NAME`]):
//! the Agent slot builds one per start, the Agent's shutdown stages run on
//! it, and shutdown stops it last, the hard stop that drops whatever that
//! instance still runs (a plugin `apply` that never returned, a disposal
//! waiting on it) without touching the host or a later instance.

use std::fmt;
use std::future::Future;
use std::num::NonZero;
use std::sync::Arc;
use std::time::{Duration, Instant};

use hanafubuki_agent_foundation::blocking::{
    BlockingFuture, BlockingOperation, BlockingOperationPort,
};
use hanafubuki_agent_foundation::panic::{
    mark_thread_contained, panic_message, unmark_thread_contained,
};
use parking_lot::Mutex;
use tokio::runtime::{Builder, Handle, Runtime};
use tokio::task::{JoinError, JoinHandle};

/// The name of every Agent runtime thread, worker or blocking.
pub const AGENT_RUNTIME_THREAD_NAME: &str = "agent-rt";
const MAX_WORKER_THREADS: usize = 4;
/// A tool call parks a blocking thread for as long as its domain call runs,
/// so the pool is sized for several long calls beside the interface's reads.
const MAX_BLOCKING_THREADS: usize = 128;

/// Why work sent to a contained runtime produced no value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RuntimeTaskFailure {
    /// The work panicked; the panic's bounded message.
    Panicked(String),
    /// The runtime is stopped, or stopped while the work ran.
    Stopped,
}

impl fmt::Display for RuntimeTaskFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Panicked(message) => write!(formatter, "panicked: {message}"),
            Self::Stopped => formatter.write_str("the runtime is stopped"),
        }
    }
}

impl From<JoinError> for RuntimeTaskFailure {
    fn from(error: JoinError) -> Self {
        if error.is_panic() {
            Self::Panicked(panic_message(error.into_panic().as_ref()))
        } else {
            Self::Stopped
        }
    }
}

#[derive(Clone)]
pub struct ContainedRuntime {
    inner: Arc<ContainedRuntimeInner>,
}

struct ContainedRuntimeInner {
    /// The name of its threads, which its errors name too.
    thread_name: &'static str,
    handle: Handle,
    /// Taken by `shutdown`; a runtime is never dropped elsewhere, since
    /// dropping one inside an async context panics.
    runtime: Mutex<Option<Runtime>>,
}

/// Marks every thread `builder` starts contained, and clears the mark when
/// the thread leaves the runtime.
pub fn contained_threads(builder: &mut Builder) -> &mut Builder {
    builder
        .on_thread_start(mark_thread_contained)
        .on_thread_stop(unmark_thread_contained)
}

impl ContainedRuntime {
    /// Starts a runtime: a few worker threads (at most
    /// `MAX_WORKER_THREADS`) and a bounded blocking pool, all named
    /// `thread_name` and marked contained.
    pub fn start(thread_name: &'static str) -> Result<Self, String> {
        let workers = std::thread::available_parallelism()
            .map(NonZero::get)
            .unwrap_or(2)
            .clamp(2, MAX_WORKER_THREADS);
        let runtime = contained_threads(&mut Builder::new_multi_thread())
            .worker_threads(workers)
            .max_blocking_threads(MAX_BLOCKING_THREADS)
            .thread_name(thread_name)
            .enable_all()
            .build()
            .map_err(|error| format!("could not start the {thread_name} runtime: {error}"))?;
        Ok(Self {
            inner: Arc::new(ContainedRuntimeInner {
                thread_name,
                handle: runtime.handle().clone(),
                runtime: Mutex::new(Some(runtime)),
            }),
        })
    }

    pub fn handle(&self) -> &Handle {
        &self.inner.handle
    }

    /// Runs `future` as a task of the runtime and waits for it.
    pub async fn run<F>(&self, future: F) -> Result<F::Output, RuntimeTaskFailure>
    where
        F: Future + Send + 'static,
        F::Output: Send + 'static,
    {
        Ok(self.inner.handle.spawn(future).await?)
    }

    /// Runs `operation` on the runtime's blocking pool and waits for it.
    pub async fn run_blocking<F, R>(&self, operation: F) -> Result<R, RuntimeTaskFailure>
    where
        F: FnOnce() -> R + Send + 'static,
        R: Send + 'static,
    {
        Ok(self.inner.handle.spawn_blocking(operation).await?)
    }

    /// Runs `future` as a task of the runtime and blocks the calling thread
    /// until it ends. For composition and shutdown, which run outside any
    /// async context; the future itself never runs on the caller.
    pub fn block_on<F>(&self, future: F) -> Result<F::Output, RuntimeTaskFailure>
    where
        F: Future + Send + 'static,
        F::Output: Send + 'static,
    {
        let task: JoinHandle<F::Output> = self.inner.handle.spawn(future);
        Ok(self.inner.handle.block_on(task)?)
    }

    /// Like [`Self::block_on`], but the calling thread waits only until
    /// `deadline`: `None` when the work had not ended by then (it keeps
    /// running until the runtime stops). The wait does not depend on the
    /// runtime making progress, so a runtime whose workers are all blocked
    /// cannot hold the caller. For the shutdown stages.
    pub fn block_on_until<F>(
        &self,
        future: F,
        deadline: Instant,
    ) -> Option<Result<F::Output, RuntimeTaskFailure>>
    where
        F: Future + Send + 'static,
        F::Output: Send + 'static,
    {
        let (sender, outcome) = std::sync::mpsc::sync_channel(1);
        let task: JoinHandle<F::Output> = self.inner.handle.spawn(future);
        self.inner.handle.spawn(async move {
            let _ = sender.send(task.await.map_err(RuntimeTaskFailure::from));
        });
        match outcome.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
            Ok(result) => Some(result),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => None,
            // The runtime stopped before the waiting task ran.
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                Some(Err(RuntimeTaskFailure::Stopped))
            }
        }
    }

    /// A blocking port onto this runtime's pool, whose threads are
    /// contained: the Agent's blocking work runs there, never on the
    /// application's pool.
    pub fn blocking_port(&self) -> Arc<dyn BlockingOperationPort> {
        Arc::new(ContainedBlockingOperationPort {
            handle: self.inner.handle.clone(),
        })
    }

    /// Stops the runtime: running tasks are dropped and the blocking pool is
    /// given `timeout` to finish. Work sent afterwards fails as
    /// [`RuntimeTaskFailure::Stopped`]. Called from an async context, where a
    /// bounded wait would panic, it stops in the background instead.
    /// Returns an error when the runtime did not stop within `timeout`.
    pub fn shutdown(&self, timeout: Duration) -> Result<(), String> {
        let Some(runtime) = self.inner.runtime.lock().take() else {
            return Ok(());
        };
        if Handle::try_current().is_ok() {
            runtime.shutdown_background();
            return Err(format!(
                "the {} runtime was stopped from an async context, in the background",
                self.inner.thread_name
            ));
        }
        let started = Instant::now();
        runtime.shutdown_timeout(timeout);
        if started.elapsed() >= timeout {
            Err(format!(
                "{} blocking work was still running after {} ms",
                self.inner.thread_name,
                timeout.as_millis()
            ))
        } else {
            Ok(())
        }
    }
}

struct ContainedBlockingOperationPort {
    handle: Handle,
}

impl BlockingOperationPort for ContainedBlockingOperationPort {
    fn execute<'a>(
        &'a self,
        label: &'static str,
        operation: BlockingOperation,
    ) -> BlockingFuture<'a> {
        let task = self.handle.spawn_blocking(operation);
        Box::pin(async move {
            task.await
                .map_err(|error| format!("{label} failed: {}", RuntimeTaskFailure::from(error)))
        })
    }
}

#[cfg(test)]
mod tests;
