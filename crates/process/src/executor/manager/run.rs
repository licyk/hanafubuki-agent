//! Blocking one-shot runs for callers that only need output and an exit code.

use std::thread;
use std::time::{Duration, Instant};

use super::{
    dispose_executor, read_executor_output, start_executor, terminate_executor, ExecutorManager,
};
use crate::executor::model::{ExecutorOutput, ExecutorStartRequest, ExecutorStatus};

/// The final outcome of a blocking one-shot run.
pub struct RunOutcome {
    /// Terminal status of the run.
    pub status: ExecutorStatus,
    pub exit_code: Option<i32>,
    /// All output (all streams) decoded lossily, in arrival order.
    pub output: String,
}

const RUN_POLL: Duration = Duration::from_millis(25);

/// Start an execution and block until it finishes (or `wait` elapses), returning
/// the final status, exit code, and combined output, then dispose the session.
/// Ordinary Rust helper for backend callers that only need argv + output + exit
/// code (e.g. a `python --version` check); it never touches Tauri.
pub fn run_and_wait(
    manager: &ExecutorManager,
    request: ExecutorStartRequest,
    wait: Duration,
) -> Result<RunOutcome, String> {
    let info = start_executor(manager, request)?;
    let id = info.executor_id;
    let deadline = Instant::now() + wait;

    let outcome = loop {
        let out = read_executor_output(manager, &id, Some(0), None)?;
        if out.status.is_terminal() {
            break outcome_from(&out);
        }
        if Instant::now() >= deadline {
            if let Err(error) = terminate_executor(manager, &id) {
                log::warn!("terminate timed-out executor {id} failed: {error}");
            }
            let out = read_executor_output(manager, &id, Some(0), None)?;
            break outcome_from(&out);
        }
        thread::sleep(RUN_POLL);
    };

    if let Err(error) = dispose_executor(manager, &id) {
        log::warn!("dispose executor {id} after run_and_wait failed: {error}");
    }
    Ok(outcome)
}

fn outcome_from(out: &ExecutorOutput) -> RunOutcome {
    let mut bytes = Vec::new();
    for chunk in &out.chunks {
        bytes.extend_from_slice(&chunk.data);
    }
    RunOutcome {
        status: out.status,
        exit_code: out.exit_code,
        output: String::from_utf8_lossy(&bytes).into_owned(),
    }
}
