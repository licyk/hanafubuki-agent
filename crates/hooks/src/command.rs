// Derived from OpenAI Codex (https://github.com/openai/codex),
// `codex-rs/hooks/src/engine/command_runner.rs` (a hook's command through
// the user's shell, the event on its stdin, bounded by its timeout) at
// commit `57a808a392`.
// Copyright 2025 OpenAI. Licensed under the Apache License, Version 2.0
// (http://www.apache.org/licenses/LICENSE-2.0).
// Modified for hanafubuki-agent: the command runs as a reserved execution
// of the Agent's `ExecutorManager` rather than a `tokio::process::Command`,
// and the executor's own timeout ends it; a cancelled run is terminated and
// disposed of before it returns.

use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use hanafubuki_agent_foundation::cancellation::CancellationToken;
use hanafubuki_agent_process::executor::manager::{
    dispose_reserved_executor, read_reserved_executor_output, release_executor_reservation,
    reserve_executor_id, start_reserved_executor, terminate_reserved_executor,
    write_reserved_executor_stdin, ReservedExecutorReceipt,
};
use hanafubuki_agent_process::executor::{
    EnvironmentInheritance, ExecutorIoMode, ExecutorManager, ExecutorStartRequest, ExecutorStatus,
    OutputStream,
};
use hanafubuki_agent_process::ownership::ProcessOwnerId;

/// What a hook may print; the rest is cut.
const OUTPUT_CAP: usize = 1024 * 1024;
const POLL: Duration = Duration::from_millis(20);

static NEXT_HOOK: AtomicU64 = AtomicU64::new(1);

/// How a hook's command ended.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct CommandRun {
    pub(crate) exit_code: Option<i32>,
    pub(crate) stdout: String,
    pub(crate) stderr: String,
    /// Why it has no result: it did not start, timed out or was cancelled.
    pub(crate) error: Option<String>,
}

/// Runs `command` in `cwd` through the user's shell with `input` on its
/// stdin, to its end, its timeout, or `cancellation`.
pub(crate) fn run_command(
    executor: &ExecutorManager,
    command: &str,
    timeout: Duration,
    input: &str,
    cwd: &Path,
    cancellation: &CancellationToken,
) -> CommandRun {
    let executor_id = format!("hook-{}", NEXT_HOOK.fetch_add(1, Ordering::Relaxed));
    let reservation = match reserve_executor_id(executor, &executor_id, &ProcessOwnerId::generate())
    {
        Ok(reservation) => reservation,
        Err(error) => return failed(format!("the hook did not start: {error}")),
    };
    let request = ExecutorStartRequest {
        executor_id: Some(executor_id),
        command: shell_command(command),
        cwd: Some(cwd.to_path_buf()),
        environment_inheritance: EnvironmentInheritance::Inherit,
        env: Default::default(),
        io_mode: ExecutorIoMode::Pipe,
        stream_stdin: true,
        stream_output: true,
        cols: None,
        rows: None,
        timeout_ms: Some(u64::try_from(timeout.as_millis()).unwrap_or(u64::MAX)),
        output_bytes_cap: Some(OUTPUT_CAP),
    };
    let receipt = match start_reserved_executor(executor, request, &reservation) {
        Ok((_, receipt)) => receipt,
        Err(error) => {
            release_executor_reservation(executor, &reservation);
            return failed(format!("the hook did not start: {error}"));
        }
    };
    // A hook that does not read its stdin closes the pipe first; that is
    // not its failure.
    if let Err(error) =
        write_reserved_executor_stdin(executor, &receipt, input.as_bytes().to_vec(), true)
    {
        log::debug!("hook stdin was not written: {error}");
    }
    let run = wait(executor, &receipt, timeout, cancellation);
    if let Err(error) = dispose_reserved_executor(executor, &receipt) {
        log::warn!("a hook's process was not disposed of: {error}");
    }
    run
}

/// Polls the execution until it ends (the executor's timeout ends it at
/// the latest), terminating it when `cancellation` fires.
fn wait(
    executor: &ExecutorManager,
    receipt: &ReservedExecutorReceipt,
    timeout: Duration,
    cancellation: &CancellationToken,
) -> CommandRun {
    let mut cancelled = false;
    loop {
        let output = match read_reserved_executor_output(executor, receipt, Some(0), None) {
            Ok(output) => output,
            Err(error) => return failed(error),
        };
        if output.status.is_terminal() {
            let (mut stdout, mut stderr) = (Vec::new(), Vec::new());
            for chunk in output.chunks {
                match chunk.stream {
                    OutputStream::Stdout => stdout.extend(chunk.data),
                    OutputStream::Stderr | OutputStream::Merged => stderr.extend(chunk.data),
                }
            }
            let error = match output.status {
                _ if cancelled => Some("the hook was cancelled".to_string()),
                ExecutorStatus::TimedOut => {
                    Some(format!("the hook timed out after {} s", timeout.as_secs()))
                }
                ExecutorStatus::Failed if output.exit_code.is_none() => {
                    Some("the hook failed to run".to_string())
                }
                ExecutorStatus::Cancelled => Some("the hook was cancelled".to_string()),
                _ => None,
            };
            return CommandRun {
                exit_code: output.exit_code,
                stdout: String::from_utf8_lossy(&stdout).into_owned(),
                stderr: String::from_utf8_lossy(&stderr).into_owned(),
                error,
            };
        }
        if !cancelled && cancellation.is_cancelled() {
            cancelled = true;
            if let Err(error) = terminate_reserved_executor(executor, receipt) {
                log::warn!("a cancelled hook was not terminated: {error}");
            }
        }
        if cancelled {
            std::thread::sleep(POLL);
        } else {
            cancellation.wait_timeout(POLL);
        }
    }
}

/// The user's shell, as Codex runs a hook: `$SHELL -lc` (`/bin/sh`
/// without one), or `%COMSPEC% /C` on Windows.
fn shell_command(command: &str) -> Vec<String> {
    if cfg!(windows) {
        let shell = std::env::var("COMSPEC").unwrap_or_else(|_| "cmd.exe".to_string());
        vec![shell, "/C".to_string(), command.to_string()]
    } else {
        let shell = std::env::var("SHELL")
            .ok()
            .filter(|shell| !shell.is_empty())
            .unwrap_or_else(|| "/bin/sh".to_string());
        vec![shell, "-lc".to_string(), command.to_string()]
    }
}

fn failed(error: String) -> CommandRun {
    CommandRun {
        error: Some(error),
        ..CommandRun::default()
    }
}
