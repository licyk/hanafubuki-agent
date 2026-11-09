// Derived from OpenAI Codex (https://github.com/openai/codex),
// `codex-rs/worktree/src/git.rs` (an isolated `git`: the inherited
// repository selectors removed, hooks and filesystem monitoring off) at
// commit `57a808a392`.
// Copyright 2025 OpenAI. Licensed under the Apache License, Version 2.0
// (http://www.apache.org/licenses/LICENSE-2.0).
// Modified for hanafubuki-agent: `git` runs as a reserved execution of the
// Agent's `ExecutorManager` rather than a `std::process::Command`, bounded
// by a timeout; the filter drivers are left as the repository configures
// them, so there is no filter discovery; stdout is read apart from stderr,
// which only an error carries.

use std::path::{Path, PathBuf};
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

use crate::WorktreeError;

/// Long enough to check out a large repository.
const GIT_TIMEOUT_MS: u64 = 10 * 60 * 1_000;
const GIT_OUTPUT_CAP: usize = 4 * 1024 * 1024;
const POLL: Duration = Duration::from_millis(20);
const DISABLED_HOOKS_PATH: &str = if cfg!(windows) { "NUL" } else { "/dev/null" };

/// What a wrapper of `git` may have exported to select another repository:
/// the repository is chosen by the working directory alone.
const REPOSITORY_SELECTORS: [&str; 16] = [
    "GIT_ALTERNATE_OBJECT_DIRECTORIES",
    "GIT_CEILING_DIRECTORIES",
    "GIT_CONFIG",
    "GIT_CONFIG_PARAMETERS",
    "GIT_CONFIG_COUNT",
    "GIT_OBJECT_DIRECTORY",
    "GIT_DIR",
    "GIT_WORK_TREE",
    "GIT_IMPLICIT_WORK_TREE",
    "GIT_GRAFT_FILE",
    "GIT_INDEX_FILE",
    "GIT_NO_REPLACE_OBJECTS",
    "GIT_REPLACE_REF_BASE",
    "GIT_PREFIX",
    "GIT_SHALLOW_FILE",
    "GIT_COMMON_DIR",
];

static NEXT_GIT: AtomicU64 = AtomicU64::new(1);

/// Runs `git args` in `cwd` to its end and returns its stdout; a failure
/// is its stderr.
pub(crate) fn git(
    executor: &ExecutorManager,
    cwd: &Path,
    args: &[&str],
) -> Result<Vec<u8>, WorktreeError> {
    let executor_id = format!("worktree-git-{}", NEXT_GIT.fetch_add(1, Ordering::Relaxed));
    let reservation = reserve_executor_id(executor, &executor_id, &ProcessOwnerId::generate())
        .map_err(WorktreeError::Git)?;
    let request = ExecutorStartRequest {
        executor_id: Some(executor_id),
        command: command(args),
        cwd: Some(cwd.to_path_buf()),
        environment_inheritance: EnvironmentInheritance::Inherit,
        env: REPOSITORY_SELECTORS
            .iter()
            .map(|name| (name.to_string(), None))
            .chain([
                ("GIT_TERMINAL_PROMPT".to_string(), Some("0".to_string())),
                ("GIT_LFS_SKIP_SMUDGE".to_string(), Some("1".to_string())),
            ])
            .collect(),
        io_mode: ExecutorIoMode::Pipe,
        stream_stdin: false,
        stream_output: true,
        cols: None,
        rows: None,
        timeout_ms: Some(GIT_TIMEOUT_MS),
        output_bytes_cap: Some(GIT_OUTPUT_CAP),
    };
    let receipt = match start_reserved_executor(executor, request, &reservation) {
        Ok((_, receipt)) => receipt,
        Err(error) => {
            release_executor_reservation(executor, &reservation);
            return Err(WorktreeError::Git(format!("git did not start: {error}")));
        }
    };
    let finished = wait(executor, &receipt);
    let disposed = dispose_reserved_executor(executor, &receipt);
    let (exit_code, stdout, stderr) = finished?;
    disposed.map_err(WorktreeError::Git)?;
    if exit_code != Some(0) {
        let stderr = String::from_utf8_lossy(&stderr);
        let stderr = stderr.trim();
        return Err(WorktreeError::Git(if stderr.is_empty() {
            format!("git {} exited with {exit_code:?}", args.join(" "))
        } else {
            stderr.to_string()
        }));
    }
    Ok(stdout)
}

/// `git`'s stdout as one line of text.
pub(crate) fn git_text(
    executor: &ExecutorManager,
    cwd: &Path,
    args: &[&str],
) -> Result<String, WorktreeError> {
    let stdout = git(executor, cwd, args)?;
    String::from_utf8(stdout)
        .map(|text| text.trim_end_matches(['\r', '\n']).to_string())
        .map_err(|_| WorktreeError::Git(format!("git {} printed no UTF-8", args.join(" "))))
}

/// `git`'s stdout as a path.
pub(crate) fn git_path(
    executor: &ExecutorManager,
    cwd: &Path,
    args: &[&str],
) -> Result<PathBuf, WorktreeError> {
    let path = git_text(executor, cwd, args)?;
    if path.is_empty() {
        return Err(WorktreeError::Git(format!(
            "git {} printed no path",
            args.join(" ")
        )));
    }
    Ok(PathBuf::from(path))
}

fn command(args: &[&str]) -> Vec<String> {
    let mut command = vec![
        "git".to_string(),
        "-c".to_string(),
        format!("core.hooksPath={DISABLED_HOOKS_PATH}"),
        "-c".to_string(),
        "core.fsmonitor=".to_string(),
    ];
    command.extend(args.iter().map(|arg| arg.to_string()));
    command
}

type Finished = (Option<i32>, Vec<u8>, Vec<u8>);

/// Polls the execution until it ends (the executor's timeout ends it at
/// the latest) and splits its output by stream.
fn wait(
    executor: &ExecutorManager,
    receipt: &ReservedExecutorReceipt,
) -> Result<Finished, WorktreeError> {
    loop {
        let output = read_reserved_executor_output(executor, receipt, Some(0), None)
            .map_err(WorktreeError::Git)?;
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
