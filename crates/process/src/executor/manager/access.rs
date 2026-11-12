//! Executor lookup, output reads, stdin, and resize, by id or by process receipt.

use std::process::ExitStatus;
use std::sync::Arc;
use std::time::{Duration, Instant};

use super::{ExecutorManager, ReservedExecutorReceipt};
use crate::executor::model::{ExecutorInfo, ExecutorOutput};
use crate::executor::session::ExecutorSession;
use crate::ownership::{ProcessOwnerId, ProcessReceipt};
use crate::terminal_size::TerminalSize;

fn get_reserved_session(
    manager: &ExecutorManager,
    executor_id: &str,
    reservation_id: &ProcessOwnerId,
) -> Result<Arc<ExecutorSession>, String> {
    let registry = manager.registry.lock();
    if !registry
        .ownership
        .authorize_identity(executor_id, reservation_id)
    {
        return Err(format!("reserved executor receipt is stale: {executor_id}"));
    }
    registry
        .sessions
        .get(executor_id)
        .cloned()
        .ok_or_else(|| format!("executor not found: {executor_id}"))
}

pub fn reserved_executor_exists(manager: &ExecutorManager, receipt: &ProcessReceipt) -> bool {
    get_reserved_session(manager, receipt.process_id(), receipt.owner_id()).is_ok()
}

/// Waits up to `timeout` for the reserved executor's process to end, and
/// returns how it ended; `None` while it still runs. A process whose end
/// was observed without an exit status (a failed wait, a PTY) is an error.
pub fn wait_reserved_executor(
    manager: &ExecutorManager,
    receipt: &ProcessReceipt,
    timeout: Duration,
) -> Result<Option<ExitStatus>, String> {
    let session = get_reserved_session(manager, receipt.process_id(), receipt.owner_id())?;
    if !session.wait_for_completion(timeout) {
        return Ok(None);
    }
    let inner = session.inner.lock();
    match inner.exit_status {
        Some(status) => Ok(Some(status)),
        None => Err(inner
            .error
            .clone()
            .unwrap_or_else(|| "the process ended without an exit status".to_string())),
    }
}

pub fn get_reserved_executor_info(
    manager: &ExecutorManager,
    receipt: &ProcessReceipt,
) -> Result<ExecutorInfo, String> {
    Ok(get_reserved_session(manager, receipt.process_id(), receipt.owner_id())?.info())
}

pub fn read_reserved_executor_output(
    manager: &ExecutorManager,
    receipt: &ProcessReceipt,
    from_offset: Option<u64>,
    max_bytes: Option<usize>,
) -> Result<ExecutorOutput, String> {
    get_reserved_session(manager, receipt.process_id(), receipt.owner_id())?;
    read_executor_output(manager, receipt.process_id(), from_offset, max_bytes)
}

pub fn get_executor_info(
    manager: &ExecutorManager,
    executor_id: &str,
) -> Result<ExecutorInfo, String> {
    Ok(manager.get(executor_id)?.info())
}

pub fn read_executor_output(
    manager: &ExecutorManager,
    executor_id: &str,
    from_offset: Option<u64>,
    max_bytes: Option<usize>,
) -> Result<ExecutorOutput, String> {
    let session = manager.get(executor_id)?;
    let (status, exit_code) = {
        let inner = session.inner.lock();
        (inner.status, inner.exit_code)
    };
    let mut output = session.output.lock();
    output.release_held(Instant::now());
    let (chunks, start_offset, next_offset, truncated) =
        output.read_from(from_offset.unwrap_or(0), max_bytes);
    let (terminal_size, resizes) = output.terminal_sizes(start_offset, next_offset);
    drop(output);
    Ok(ExecutorOutput {
        executor_id: executor_id.to_string(),
        status,
        exit_code,
        chunks,
        start_offset,
        next_offset,
        truncated,
        terminal_size,
        resizes,
    })
}

pub fn write_executor_stdin(
    manager: &ExecutorManager,
    executor_id: &str,
    bytes: Vec<u8>,
    close_stdin: bool,
) -> Result<(), String> {
    let session = generic_mutation_session(manager, executor_id)?;
    session.backend()?.write_stdin(bytes, close_stdin)
}

pub fn resize_executor_pty(
    manager: &ExecutorManager,
    executor_id: &str,
    cols: u16,
    rows: u16,
) -> Result<(), String> {
    generic_mutation_session(manager, executor_id)?.resize(TerminalSize::new(cols, rows))
}

fn generic_mutation_session(
    manager: &ExecutorManager,
    executor_id: &str,
) -> Result<Arc<ExecutorSession>, String> {
    let registry = manager.registry.lock();
    if registry.ownership.contains_process(executor_id) {
        return Err(format!(
            "executor {executor_id} is domain-owned; a process receipt is required"
        ));
    }
    registry
        .sessions
        .get(executor_id)
        .cloned()
        .ok_or_else(|| format!("executor not found: {executor_id}"))
}

pub fn write_reserved_executor_stdin(
    manager: &ExecutorManager,
    receipt: &ReservedExecutorReceipt,
    bytes: Vec<u8>,
    close_stdin: bool,
) -> Result<(), String> {
    let session = get_reserved_session(manager, receipt.process_id(), receipt.owner_id())?;
    session.backend()?.write_stdin(bytes, close_stdin)
}

pub fn interrupt_reserved_executor(
    manager: &ExecutorManager,
    receipt: &ReservedExecutorReceipt,
) -> Result<(), String> {
    let session = get_reserved_session(manager, receipt.process_id(), receipt.owner_id())?;
    session.backend()?.interrupt()
}

pub fn resize_reserved_executor_pty(
    manager: &ExecutorManager,
    receipt: &ReservedExecutorReceipt,
    cols: u16,
    rows: u16,
) -> Result<(), String> {
    get_reserved_session(manager, receipt.process_id(), receipt.owner_id())?
        .resize(TerminalSize::new(cols, rows))
}
