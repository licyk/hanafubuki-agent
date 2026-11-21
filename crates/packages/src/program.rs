//! A program a package operation runs: one execution of the Agent's
//! `ExecutorManager`, bounded by a timeout and an output cap, its two
//! streams kept apart. `git` and `npm` share it, so a package's fetch
//! reaches the environment and proxy the Agent's processes own rather than
//! the Agent's own.

use std::collections::{BTreeMap, HashMap};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;
use std::time::Duration;

use hanafubuki_agent_process::executor::manager::{
    dispose_reserved_executor, read_reserved_executor_output, release_executor_reservation,
    reserve_executor_id, start_reserved_executor, ReservedExecutorReceipt,
};
use hanafubuki_agent_process::executor::{
    EnvironmentInheritance, ExecutorIoMode, ExecutorManager, ExecutorStartRequest, OutputStream,
};
use hanafubuki_agent_process::ownership::ProcessOwnerId;

const POLL: Duration = Duration::from_millis(20);

static NEXT_PROGRAM: AtomicU64 = AtomicU64::new(1);

/// One program to run to its end.
pub(crate) struct ProgramRequest<'a> {
    /// How a failure names it, e.g. `git`.
    pub(crate) name: &'a str,
    /// The program and its arguments.
    pub(crate) command: &'a [String],
    pub(crate) cwd: &'a Path,
    /// `Some(value)` sets a variable and `None` unsets it; the rest of the
    /// environment is inherited.
    pub(crate) env: &'a BTreeMap<String, Option<String>>,
    pub(crate) timeout_ms: u64,
    pub(crate) output_cap: usize,
}

/// Its exit code, its standard output and its standard error.
pub(crate) type Finished = (Option<i32>, Vec<u8>, Vec<u8>);

/// Runs `request` to its end; an error is the program not starting or the
/// execution failing.
pub(crate) fn run(
    executor: &ExecutorManager,
    request: ProgramRequest<'_>,
) -> Result<Finished, String> {
    let executor_id = format!(
        "packages-{}-{}",
        request.name,
        NEXT_PROGRAM.fetch_add(1, Ordering::Relaxed)
    );
    let reservation = reserve_executor_id(executor, &executor_id, &ProcessOwnerId::generate())?;
    let start = ExecutorStartRequest {
        executor_id: Some(executor_id),
        command: request.command.to_vec(),
        cwd: Some(request.cwd.to_path_buf()),
        environment_inheritance: EnvironmentInheritance::Inherit,
        env: request
            .env
            .iter()
            .map(|(name, value)| (name.clone(), value.clone()))
            .collect::<HashMap<_, _>>(),
        io_mode: ExecutorIoMode::Pipe,
        stream_stdin: false,
        stream_output: true,
        cols: None,
        rows: None,
        timeout_ms: Some(request.timeout_ms),
        output_bytes_cap: Some(request.output_cap),
    };
    let receipt = match start_reserved_executor(executor, start, &reservation) {
        Ok((_, receipt)) => receipt,
        Err(error) => {
            release_executor_reservation(executor, &reservation);
            return Err(format!("{} did not start: {error}", request.name));
        }
    };
    let finished = wait(executor, &receipt);
    let disposed = dispose_reserved_executor(executor, &receipt);
    let finished = finished?;
    disposed?;
    Ok(finished)
}

/// Polls the execution until it ends (the executor's timeout ends it at
/// the latest) and splits its output by stream.
fn wait(executor: &ExecutorManager, receipt: &ReservedExecutorReceipt) -> Result<Finished, String> {
    loop {
        let output = read_reserved_executor_output(executor, receipt, Some(0), None)?;
        if output.status.is_terminal() {
            let (mut stdout, mut stderr) = (Vec::new(), Vec::new());
            for chunk in output.chunks {
                match chunk.stream {
                    OutputStream::Stdout => stdout.extend(chunk.data),
                    OutputStream::Stderr | OutputStream::Merged => stderr.extend(chunk.data),
                }
            }
            return Ok((output.exit_code, stdout, stderr));
        }
        thread::sleep(POLL);
    }
}
