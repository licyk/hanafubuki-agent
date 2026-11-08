//! Executor id reservations taken before a domain-owned spawn.

use super::{ExecutorManager, ExecutorReservation};
use crate::ownership::{OwnedReleaseStatus, ProcessOwnerId};

pub fn reserve_executor_id(
    manager: &ExecutorManager,
    executor_id: &str,
    reservation_id: &ProcessOwnerId,
) -> Result<ExecutorReservation, String> {
    let executor_id = executor_id.trim();
    if executor_id.is_empty() {
        return Err("reserved executor id must not be empty".to_string());
    }
    let mut registry = manager.registry.lock();
    if registry.sessions.contains_key(executor_id) {
        return Err(format!("executor already exists: {executor_id}"));
    }
    registry
        .ownership
        .reserve(executor_id, reservation_id)
        .map_err(|_| format!("executor id is already reserved: {executor_id}"))
}

pub fn release_executor_reservation(
    manager: &ExecutorManager,
    reservation: &ExecutorReservation,
) -> bool {
    let mut registry = manager.registry.lock();
    registry.ownership.release_reservation(reservation)
}

#[cfg(any(test, feature = "test-support"))]
pub fn finalize_executor_reservation(
    manager: &ExecutorManager,
    executor_id: &str,
    reservation_id: &ProcessOwnerId,
) -> OwnedReleaseStatus {
    manager
        .registry
        .lock()
        .ownership
        .finalize_reservation(executor_id, reservation_id)
}

pub fn finalize_executor_reservation_handle(
    manager: &ExecutorManager,
    reservation: &ExecutorReservation,
) -> OwnedReleaseStatus {
    manager
        .registry
        .lock()
        .ownership
        .finalize_reservation(reservation.process_id(), reservation.owner_id())
}
