//! Executor spawn: request normalization, id allocation, and backend start.

use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Instant;

use super::{ExecutorManager, ExecutorReservation, ProcessSupervision, ReservedExecutorReceipt};
use crate::executor::model::{ExecutorInfo, ExecutorStartRequest};
use crate::executor::output::DEFAULT_OUTPUT_CAP;
use crate::executor::session::ExecutorSession;
use crate::executor::{pipe, pty};

/// Start an execution from an argv command in pipe (default) or PTY mode.
pub fn start_executor(
    manager: &ExecutorManager,
    request: ExecutorStartRequest,
) -> Result<ExecutorInfo, String> {
    start_executor_impl(manager, request, None, ProcessSupervision::Tree).map(|(info, _)| info)
}

/// Start a pipe execution that is allowed to outlive the current process.
///
/// The execution remains observable and terminable while this manager is alive,
/// but Windows does not attach it to the Executor's kill-on-close Job Object.
/// This is intended for explicit process handoff such as launching an updater
/// or replacement application immediately before the current process exits.
pub fn start_independent_executor(
    manager: &ExecutorManager,
    request: ExecutorStartRequest,
) -> Result<ExecutorInfo, String> {
    if request.io_mode.uses_pty() {
        return Err("independent executor start supports pipe mode only".to_string());
    }
    start_executor_impl(manager, request, None, ProcessSupervision::Independent)
        .map(|(info, _)| info)
}

pub fn start_reserved_executor(
    manager: &ExecutorManager,
    request: ExecutorStartRequest,
    reservation: &ExecutorReservation,
) -> Result<(ExecutorInfo, ReservedExecutorReceipt), String> {
    let (info, receipt) = start_executor_impl(
        manager,
        request,
        Some(reservation),
        ProcessSupervision::Tree,
    )?;
    Ok((
        info,
        receipt.expect("reserved executor start always returns a receipt"),
    ))
}

fn start_executor_impl(
    manager: &ExecutorManager,
    mut request: ExecutorStartRequest,
    reservation: Option<&ExecutorReservation>,
    supervision: ProcessSupervision,
) -> Result<(ExecutorInfo, Option<ReservedExecutorReceipt>), String> {
    if request.command.is_empty() || request.command[0].trim().is_empty() {
        return Err("executor command must not be empty".to_string());
    }
    apply_child_process_environment(manager, &mut request);

    // Codex parity: PTY mode implies stdin + output streaming.
    request.normalize_io_mode();

    // Convert a relative cwd into a normalized absolute path.
    if let Some(cwd) = request.cwd.take() {
        request.cwd = Some(hanafubuki_agent_foundation::paths::normalize_path(cwd)?);
    }

    let mut executor_id = match request.executor_id.as_deref().map(str::trim) {
        Some(id) if !id.is_empty() => id.to_string(),
        _ => format!("exec-{}", manager.next_id.fetch_add(1, Ordering::Relaxed)),
    };
    if let Some(reservation) = reservation {
        if executor_id != reservation.process_id() {
            return Err(format!(
                "reserved executor id mismatch: expected {}, got {executor_id}",
                reservation.process_id()
            ));
        }
    } else if request.executor_id.is_none() {
        loop {
            let registry = manager.registry.lock();
            let available = !registry.sessions.contains_key(&executor_id)
                && !registry.ownership.contains_identity(&executor_id);
            drop(registry);
            if available {
                break;
            }
            executor_id = format!("exec-{}", manager.next_id.fetch_add(1, Ordering::Relaxed));
        }
    }
    let cap = request
        .output_bytes_cap
        .unwrap_or(DEFAULT_OUTPUT_CAP)
        .max(1);

    let session = Arc::new(ExecutorSession::new(
        executor_id.clone(),
        request.io_mode,
        request.command.clone(),
        cap,
        request.io_mode.uses_pty().then(|| request.terminal_size()),
        manager.owner_group.clone(),
    ));

    {
        let mut registry = manager.registry.lock();
        if registry.sessions.contains_key(&executor_id) {
            return Err(format!("executor already exists: {executor_id}"));
        }
        match reservation {
            Some(reservation) => {
                if !registry.ownership.begin_spawn(reservation) {
                    return Err(format!(
                        "executor reservation is missing, consumed, stale, or already spawning: {executor_id}"
                    ));
                }
            }
            None => {
                if registry.ownership.contains_identity(&executor_id) {
                    return Err(format!("executor id is reserved or owned: {executor_id}"));
                }
                registry
                    .sessions
                    .insert(executor_id.clone(), session.clone());
            }
        }
    }

    let backend_spawn_started = Instant::now();
    log::info!(
        "agent_diag stage=executor.backend_spawn.begin executor_id={executor_id} io_mode={:?}",
        request.io_mode
    );
    let inherited_environment = manager.child_process_environment.inherited_snapshot();
    let spawned = if request.io_mode.uses_pty() {
        pty::spawn(session.clone(), &request, &inherited_environment)
    } else {
        pipe::spawn(
            session.clone(),
            &request,
            supervision,
            &inherited_environment,
        )
    };
    log::info!(
        "agent_diag stage=executor.backend_spawn.end executor_id={executor_id} io_mode={:?} outcome={} elapsed_ms={}",
        request.io_mode,
        if spawned.is_ok() { "ok" } else { "error" },
        backend_spawn_started.elapsed().as_millis()
    );

    match spawned {
        Ok(backend) => {
            // Before the session is published: a crash from here on leaves
            // its process group in the owner's record.
            if let Some(leader) = backend.process_group_leader() {
                session.record_process_group(leader);
            }
            // Register the backend first (honoring any early terminate/timeout),
            // then start the timeout timer so it can never fire before the
            // backend exists.
            session.set_backend(backend);
            session.set_running();
            let receipt = if let Some(reservation) = reservation {
                let mut registry = manager.registry.lock();
                if registry.sessions.contains_key(&executor_id) {
                    drop(registry);
                    let _ = session.terminate();
                    return Err(format!("executor already exists: {executor_id}"));
                }
                let receipt = registry.ownership.commit(reservation).map_err(|error| {
                    let _ = session.terminate();
                    error
                })?;
                registry
                    .sessions
                    .insert(executor_id.clone(), session.clone());
                Some(receipt)
            } else {
                None
            };
            crate::executor::backend_common::spawn_timeout(session.clone(), request.timeout_ms);
            log::info!(
                "executor started id={executor_id} io_mode={:?} argv0={}",
                request.io_mode,
                request.command[0]
            );
            Ok((session.info(), receipt))
        }
        Err(err) => {
            let mut registry = manager.registry.lock();
            if let Some(reservation) = reservation {
                registry.ownership.abort_spawn(reservation);
            }
            if registry
                .sessions
                .get(&executor_id)
                .map(|registered| Arc::ptr_eq(registered, &session))
                .unwrap_or(false)
            {
                registry.sessions.remove(&executor_id);
            }
            Err(format!("spawn failed: {err}"))
        }
    }
}

pub(super) fn apply_child_process_environment(
    manager: &ExecutorManager,
    request: &mut ExecutorStartRequest,
) {
    request
        .env
        .extend(manager.child_process_environment.snapshot());
}
