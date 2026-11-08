//! Executor session registry and lifecycle core functions (Tauri-free).

mod access;
mod disposal;
mod reservation;
mod run;
mod start;

use std::collections::HashMap;
use std::sync::atomic::AtomicU64;
use std::sync::Arc;

use parking_lot::Mutex;

use super::session::ExecutorSession;
use crate::ownership::{ProcessOwnershipRegistry, ProcessReceipt, ProcessReservation};
use crate::process_groups::ProcessOwnerGroup;
use hanafubuki_agent_foundation::environment::ChildProcessEnvironment;

pub use access::{
    get_executor_info, get_reserved_executor_info, interrupt_reserved_executor,
    read_executor_output, read_reserved_executor_output, reserved_executor_exists,
    resize_executor_pty, resize_reserved_executor_pty, write_executor_stdin,
    write_reserved_executor_stdin,
};
pub use disposal::{
    confirm_reserved_executor_termination, dispose_executor, dispose_reserved_executor,
    terminate_executor, terminate_owner_group, terminate_reserved_executor,
};
#[cfg(any(test, feature = "test-support"))]
pub use reservation::finalize_executor_reservation;
pub use reservation::{
    finalize_executor_reservation_handle, release_executor_reservation, reserve_executor_id,
};
pub use run::{run_and_wait, RunOutcome};
pub use start::{start_executor, start_independent_executor, start_reserved_executor};

#[derive(Default)]
struct ExecutorRegistry {
    sessions: HashMap<String, Arc<ExecutorSession>>,
    ownership: ProcessOwnershipRegistry,
}

#[derive(Clone)]
pub struct ExecutorManager {
    registry: Arc<Mutex<ExecutorRegistry>>,
    next_id: Arc<AtomicU64>,
    child_process_environment: ChildProcessEnvironment,
    /// The group every session started through this handle belongs to.
    owner_group: Option<ProcessOwnerGroup>,
}

impl Default for ExecutorManager {
    fn default() -> Self {
        Self::new(ChildProcessEnvironment::default())
    }
}

pub type ExecutorReservation = ProcessReservation;
pub type ReservedExecutorReceipt = ProcessReceipt;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ProcessSupervision {
    Tree,
    Independent,
}

impl ExecutorManager {
    pub fn new(child_process_environment: ChildProcessEnvironment) -> Self {
        Self {
            registry: Arc::new(Mutex::new(ExecutorRegistry::default())),
            next_id: Arc::new(AtomicU64::new(0)),
            child_process_environment,
            owner_group: None,
        }
    }

    /// This manager (the same registry), with every session started through
    /// the returned handle in `group`: its holder can terminate them all
    /// with [`terminate_owner_group`], and a recorded group records their
    /// process groups.
    pub fn owned_by(&self, group: ProcessOwnerGroup) -> Self {
        Self {
            owner_group: Some(group),
            ..self.clone()
        }
    }

    fn get(&self, executor_id: &str) -> Result<Arc<ExecutorSession>, String> {
        self.registry
            .lock()
            .sessions
            .get(executor_id)
            .cloned()
            .ok_or_else(|| format!("executor not found: {executor_id}"))
    }
}

#[cfg(any(test, feature = "test-support"))]
pub fn executor_registry_counts(manager: &ExecutorManager) -> (usize, usize, usize) {
    let registry = manager.registry.lock();
    (
        registry.sessions.len(),
        registry.ownership.counts().1,
        registry.ownership.counts().0,
    )
}

#[cfg(test)]
mod tests;
