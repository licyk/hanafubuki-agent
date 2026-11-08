//! Retrying a spawn whose executable was busy for a moment.
//!
//! On Linux, `exec` fails with `ETXTBSY` ("Text file busy") while any process
//! holds the file open for writing (macOS runs such a file). The application writes executables it
//! then runs (the Micromamba binary it publishes, an extracted Python runtime,
//! a launcher update) and closes them before spawning, but a sibling thread
//! that forks while one is open copies the writable descriptor into its child,
//! which keeps it until it execs and the descriptor closes (`O_CLOEXEC`).
//! Renaming a finished temporary file into place does not help: the inherited
//! descriptor names the same inode. The window ends by itself within a few
//! milliseconds, so every Executor spawn retries `ETXTBSY` with a short,
//! bounded backoff and returns any other error, or the last `ETXTBSY`, as
//! before. Windows has no such error, and there nothing is retried.

use std::io;
use std::thread;
use std::time::Duration;

/// Waits before each retry: doubling from 1 ms, 511 ms in all.
const BUSY_RETRY_DELAYS: [Duration; 9] = [
    Duration::from_millis(1),
    Duration::from_millis(2),
    Duration::from_millis(4),
    Duration::from_millis(8),
    Duration::from_millis(16),
    Duration::from_millis(32),
    Duration::from_millis(64),
    Duration::from_millis(128),
    Duration::from_millis(256),
];

/// Runs `spawn`, retrying it after each delay of the schedule while `busy`
/// says its error is `ETXTBSY`.
pub(super) fn spawn_retrying_busy_executable<T, E>(
    executor_id: &str,
    mut spawn: impl FnMut() -> Result<T, E>,
    busy: impl Fn(&E) -> bool,
) -> Result<T, E> {
    retry_busy(executor_id, &BUSY_RETRY_DELAYS, &mut spawn, &busy)
}

fn retry_busy<T, E>(
    executor_id: &str,
    delays: &[Duration],
    spawn: &mut impl FnMut() -> Result<T, E>,
    busy: &impl Fn(&E) -> bool,
) -> Result<T, E> {
    let mut delays = delays.iter();
    loop {
        match spawn() {
            Err(error) if busy(&error) => match delays.next() {
                Some(delay) => {
                    log::warn!(
                        "executor spawn found its executable busy; retrying id={executor_id} delay_ms={}",
                        delay.as_millis()
                    );
                    thread::sleep(*delay);
                }
                None => return Err(error),
            },
            result => return result,
        }
    }
}

/// Whether a spawn failed because its executable was open for writing.
pub(super) fn is_executable_busy(error: &io::Error) -> bool {
    #[cfg(unix)]
    {
        error.raw_os_error() == Some(libc::ETXTBSY)
    }
    #[cfg(not(unix))]
    {
        let _ = error;
        false
    }
}

#[cfg(test)]
mod tests;
