//! Shared backend helpers.

use std::io::Write;
use std::sync::atomic::Ordering;
use std::sync::mpsc::{self, SyncSender, TrySendError};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use hanafubuki_agent_foundation::background_worker::spawn_contained_thread;
use hanafubuki_agent_foundation::module_faults::FaultOwner;
use parking_lot::Mutex;

use super::session::ExecutorSession;

const TIMEOUT_POLL: Duration = Duration::from_millis(50);
/// Writes queued for a process that has not read them yet (Codex parity).
const STDIN_QUEUE_CAPACITY: usize = 128;

/// A process's input, written by a dedicated thread.
///
/// A write into a PTY or pipe blocks once the process stops reading. Queueing
/// keeps that wait on the writer thread, so a write never holds a session
/// lock or stalls the caller, and terminating the process is never stuck
/// behind it: once the process is gone the pending write fails and the
/// thread ends.
pub(crate) struct StdinWriter {
    sender: Mutex<Option<SyncSender<Vec<u8>>>>,
}

impl StdinWriter {
    pub(crate) fn spawn(
        executor_id: &str,
        mut writer: Box<dyn Write + Send>,
    ) -> Result<Self, String> {
        let (sender, receiver) = mpsc::sync_channel::<Vec<u8>>(STDIN_QUEUE_CAPACITY);
        let id = executor_id.to_string();
        spawn_contained_thread(
            FaultOwner::Core,
            format!("exec-stdin-{executor_id}"),
            move || {
                // Ends once every sender is gone and the queue is drained;
                // dropping the writer then closes the process's input.
                for bytes in receiver {
                    if let Err(error) = writer.write_all(&bytes).and_then(|()| writer.flush()) {
                        log::debug!("executor stdin writer ended id={id}: {error}");
                        return;
                    }
                }
            },
        )
        .map_err(|error| format!("spawn stdin writer: {error}"))?;
        Ok(Self {
            sender: Mutex::new(Some(sender)),
        })
    }

    /// Queue `bytes`, then close the input when `close` is set; queued writes
    /// are still delivered before the process sees end of input.
    pub(crate) fn write(&self, bytes: Vec<u8>, close: bool) -> Result<(), String> {
        let mut sender = self.sender.lock();
        let Some(queue) = sender.as_ref() else {
            return Err("stdin is closed".to_string());
        };
        if !bytes.is_empty() {
            match queue.try_send(bytes) {
                Ok(()) => {}
                Err(TrySendError::Full(_)) => {
                    return Err(
                        "stdin is not accepting input: the process has not read earlier input"
                            .to_string(),
                    )
                }
                Err(TrySendError::Disconnected(_)) => {
                    *sender = None;
                    return Err("stdin is closed".to_string());
                }
            }
        }
        if close {
            *sender = None;
        }
        Ok(())
    }
}

/// Spawn a timer that terminates the execution after `timeout_ms` (when > 0),
/// flagging it as timed-out so the final status is `TimedOut`.
pub(crate) fn spawn_timeout(session: Arc<ExecutorSession>, timeout_ms: Option<u64>) {
    let ms = match timeout_ms {
        Some(ms) if ms > 0 => ms,
        _ => return,
    };
    spawn_contained_thread(
        FaultOwner::Core,
        format!("exec-timeout-{}", session.executor_id),
        move || {
            let deadline = Instant::now() + Duration::from_millis(ms);
            while Instant::now() < deadline {
                if session.is_finished() {
                    return;
                }
                thread::sleep(TIMEOUT_POLL);
            }
            if !session.is_finished() {
                // Order matters: `timed_out` is checked before `terminate_requested`
                // in `finalize`, so the status ends up `TimedOut`.
                session.timed_out.store(true, Ordering::Release);
                if let Err(error) = session.terminate() {
                    log::debug!(
                        "executor timeout thread could not terminate {}: {error}",
                        session.executor_id
                    );
                }
            }
        },
    )
    .expect("spawn executor timeout thread");
}
