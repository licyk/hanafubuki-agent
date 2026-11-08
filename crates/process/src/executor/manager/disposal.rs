//! Termination and confirmed disposal: metadata is released only after exit is observed.

use std::fmt;
use std::sync::Arc;
use std::time::{Duration, Instant};

use super::{ExecutorManager, ReservedExecutorReceipt};
use crate::executor::session::ExecutorSession;
use crate::process_groups::ProcessOwnerGroup;

pub(crate) const CONFIRMED_DISPOSAL_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ConfirmedExecutorDisposalOutcome {
    Disposed,
    AlreadyAbsent,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ConfirmedExecutorDisposalError {
    Termination {
        executor_id: String,
        error: String,
    },
    Timeout {
        executor_id: String,
        timeout: Duration,
    },
}

impl fmt::Display for ConfirmedExecutorDisposalError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Termination { executor_id, error } => {
                write!(formatter, "terminate executor {executor_id}: {error}")
            }
            Self::Timeout {
                executor_id,
                timeout,
            } => write!(
                formatter,
                "timed out after {} ms confirming executor {executor_id} termination",
                timeout.as_millis()
            ),
        }
    }
}

/// Request termination of the execution and its process tree. Idempotent; output
/// captured so far is preserved.
pub fn terminate_executor(manager: &ExecutorManager, executor_id: &str) -> Result<(), String> {
    if manager
        .registry
        .lock()
        .ownership
        .contains_process(executor_id)
    {
        return Err(format!(
            "executor {executor_id} is domain-owned; a process receipt is required"
        ));
    }
    manager.get(executor_id)?.terminate()
}

/// Dispose only after termination is confirmed. On failure the session and its
/// owner remain registered so cleanup can be retried.
pub fn dispose_executor(manager: &ExecutorManager, executor_id: &str) -> Result<(), String> {
    if manager
        .registry
        .lock()
        .ownership
        .contains_process(executor_id)
    {
        return Err(format!(
            "executor {executor_id} is domain-owned; a process receipt is required"
        ));
    }
    dispose_executor_confirmed(manager, executor_id)
        .map(|_| ())
        .map_err(|error| error.to_string())
}

/// Dispose executor metadata only after its waiter has observed process exit.
/// The registry is never held while termination is requested or awaited.
pub(crate) fn dispose_executor_confirmed(
    manager: &ExecutorManager,
    executor_id: &str,
) -> Result<ConfirmedExecutorDisposalOutcome, ConfirmedExecutorDisposalError> {
    dispose_executor_confirmed_with_timeout(manager, executor_id, CONFIRMED_DISPOSAL_TIMEOUT)
}

pub(crate) fn dispose_executor_confirmed_with_timeout(
    manager: &ExecutorManager,
    executor_id: &str,
    timeout: Duration,
) -> Result<ConfirmedExecutorDisposalOutcome, ConfirmedExecutorDisposalError> {
    let session = {
        let registry = manager.registry.lock();
        registry.sessions.get(executor_id).cloned()
    };
    let Some(session) = session else {
        return Ok(ConfirmedExecutorDisposalOutcome::AlreadyAbsent);
    };

    if !session.is_finished() {
        session
            .terminate()
            .map_err(|error| ConfirmedExecutorDisposalError::Termination {
                executor_id: executor_id.to_string(),
                error,
            })?;
        if !session.wait_for_completion(timeout) {
            return Err(ConfirmedExecutorDisposalError::Timeout {
                executor_id: executor_id.to_string(),
                timeout,
            });
        }
    }

    let mut registry = manager.registry.lock();
    if registry
        .sessions
        .get(executor_id)
        .is_some_and(|registered| Arc::ptr_eq(registered, &session))
    {
        registry.sessions.remove(executor_id);
        registry.ownership.release_confirmed_process(executor_id);
    }
    drop(registry);
    session.release_backend();
    Ok(ConfirmedExecutorDisposalOutcome::Disposed)
}

/// Terminates, confirms and disposes every session started in `group`
/// (`ExecutorManager::owned_by`): the sweep its holder runs once the owners
/// it handed the group to have stopped. Each session is disposed through the
/// confirmed path, its ownership released only after its process is gone;
/// one that does not stop within `timeout` stays registered and is returned
/// with why.
pub fn terminate_owner_group(
    manager: &ExecutorManager,
    group: &ProcessOwnerGroup,
    timeout: Duration,
) -> Vec<String> {
    let owned = manager
        .registry
        .lock()
        .sessions
        .iter()
        .filter(|(_, session)| {
            session
                .owner_group
                .as_ref()
                .is_some_and(|owner| owner.same(group))
        })
        .map(|(executor_id, _)| executor_id.clone())
        .collect::<Vec<_>>();
    owned
        .into_iter()
        .filter_map(|executor_id| {
            dispose_executor_confirmed_with_timeout(manager, &executor_id, timeout)
                .err()
                .map(|error| error.to_string())
        })
        .collect()
}

pub fn terminate_reserved_executor(
    manager: &ExecutorManager,
    receipt: &ReservedExecutorReceipt,
) -> Result<(), String> {
    let session = {
        let registry = manager.registry.lock();
        if !registry.ownership.authorize(receipt) {
            return Err(format!(
                "reserved executor receipt is stale: {}",
                receipt.process_id()
            ));
        }
        registry
            .sessions
            .get(receipt.process_id())
            .cloned()
            .ok_or_else(|| format!("executor not found: {}", receipt.process_id()))?
    };
    session.terminate()
}

pub fn dispose_reserved_executor(
    manager: &ExecutorManager,
    receipt: &ReservedExecutorReceipt,
) -> Result<(), String> {
    let session = confirmed_reserved_session(manager, receipt)?;
    let mut registry = manager.registry.lock();
    if !registry.ownership.authorize(receipt) {
        return Err(format!(
            "reserved executor receipt became stale: {}",
            receipt.process_id()
        ));
    }
    if registry
        .sessions
        .get(receipt.process_id())
        .is_some_and(|registered| Arc::ptr_eq(registered, &session))
    {
        registry.sessions.remove(receipt.process_id());
        registry.ownership.release_owner(receipt);
    }
    drop(registry);
    session.release_backend();
    Ok(())
}

pub fn confirm_reserved_executor_termination(
    manager: &ExecutorManager,
    receipt: &ReservedExecutorReceipt,
) -> Result<(), String> {
    confirmed_reserved_session(manager, receipt).map(|_| ())
}

fn confirmed_reserved_session(
    manager: &ExecutorManager,
    receipt: &ReservedExecutorReceipt,
) -> Result<Arc<ExecutorSession>, String> {
    let confirmation_started = Instant::now();
    log::info!(
        "agent_diag stage=executor.confirmed_reserved_session.begin executor_id={}",
        receipt.process_id()
    );
    let session = {
        let registry = manager.registry.lock();
        if !registry.ownership.authorize(receipt) {
            return Err(format!(
                "reserved executor receipt is stale: {}",
                receipt.process_id()
            ));
        }
        registry
            .sessions
            .get(receipt.process_id())
            .cloned()
            .ok_or_else(|| format!("executor not found: {}", receipt.process_id()))?
    };
    if !session.is_finished() {
        let terminate_started = Instant::now();
        log::info!(
            "agent_diag stage=executor.confirmed_reserved_session.terminate.begin executor_id={}",
            receipt.process_id()
        );
        session.terminate()?;
        log::info!(
            "agent_diag stage=executor.confirmed_reserved_session.terminate.end executor_id={} outcome=ok elapsed_ms={}",
            receipt.process_id(),
            terminate_started.elapsed().as_millis()
        );
        let wait_started = Instant::now();
        log::info!(
            "agent_diag stage=executor.confirmed_reserved_session.wait.begin executor_id={}",
            receipt.process_id()
        );
        if !session.wait_for_completion(CONFIRMED_DISPOSAL_TIMEOUT) {
            log::info!(
                "agent_diag stage=executor.confirmed_reserved_session.wait.end executor_id={} outcome=timeout elapsed_ms={}",
                receipt.process_id(),
                wait_started.elapsed().as_millis()
            );
            return Err(format!(
                "timed out after {} ms confirming executor {} termination",
                CONFIRMED_DISPOSAL_TIMEOUT.as_millis(),
                receipt.process_id()
            ));
        }
        log::info!(
            "agent_diag stage=executor.confirmed_reserved_session.wait.end executor_id={} outcome=terminal elapsed_ms={}",
            receipt.process_id(),
            wait_started.elapsed().as_millis()
        );
    }
    log::info!(
        "agent_diag stage=executor.confirmed_reserved_session.end executor_id={} outcome=ok elapsed_ms={}",
        receipt.process_id(),
        confirmation_started.elapsed().as_millis()
    );
    Ok(session)
}
