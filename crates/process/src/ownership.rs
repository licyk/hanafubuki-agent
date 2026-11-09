use std::collections::HashMap;
use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_OWNER_ID: AtomicU64 = AtomicU64::new(1);

/// Backend-only ownership capability shared by registry reservations that belong
/// to one process launch. The value is intentionally opaque and never serialized.
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct ProcessOwnerId(u64);

impl ProcessOwnerId {
    pub fn generate() -> Self {
        Self(NEXT_OWNER_ID.fetch_add(1, Ordering::Relaxed))
    }
}

impl fmt::Debug for ProcessOwnerId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ProcessOwnerId(..)")
    }
}

#[derive(Clone, Debug)]
pub struct RegistryReservation {
    key: String,
    reservation_id: ProcessOwnerId,
}

impl RegistryReservation {
    pub fn key(&self) -> &str {
        &self.key
    }

    pub fn reservation_id(&self) -> &ProcessOwnerId {
        &self.reservation_id
    }
}

#[derive(Default)]
pub struct ReservationSet(HashMap<String, ProcessOwnerId>);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OwnedReleaseStatus {
    Released,
    Missing,
    DifferentOwner,
}

/// Exclusive authorization to create one executor identity. It cannot be
/// constructed outside the ownership registry.
#[derive(Clone, Debug)]
pub struct ProcessReservation {
    process_id: String,
    owner_id: ProcessOwnerId,
}

impl ProcessReservation {
    #[cfg(any(test, feature = "test-support"))]
    pub fn for_test(process_id: impl Into<String>, owner_id: ProcessOwnerId) -> Self {
        Self {
            process_id: process_id.into(),
            owner_id,
        }
    }

    pub fn process_id(&self) -> &str {
        &self.process_id
    }

    pub fn owner_id(&self) -> &ProcessOwnerId {
        &self.owner_id
    }
}

/// Capability returned only after a process was spawned and ownership was
/// committed. Domain registries retain this value instead of reconstructing an
/// executor owner from string ids.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProcessReceipt {
    process_id: String,
    owner_id: ProcessOwnerId,
}

impl ProcessReceipt {
    fn from_committed_identity(process_id: String, owner_id: ProcessOwnerId) -> Self {
        Self {
            process_id,
            owner_id,
        }
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn for_test(process_id: impl Into<String>, owner_id: ProcessOwnerId) -> Self {
        Self::from_committed_identity(process_id.into(), owner_id)
    }

    pub fn process_id(&self) -> &str {
        &self.process_id
    }

    pub fn owner_id(&self) -> &ProcessOwnerId {
        &self.owner_id
    }
}

#[derive(Default)]
pub struct ProcessOwnershipRegistry {
    reservations: HashMap<String, ProcessReservationState>,
    owners: HashMap<String, ProcessOwnerId>,
}

struct ProcessReservationState {
    owner_id: ProcessOwnerId,
    spawning: bool,
}

impl ProcessOwnershipRegistry {
    pub fn reserve(
        &mut self,
        process_id: &str,
        owner_id: &ProcessOwnerId,
    ) -> Result<ProcessReservation, String> {
        if self.reservations.contains_key(process_id) || self.owners.contains_key(process_id) {
            return Err(format!("process identity is already owned: {process_id}"));
        }
        self.reservations.insert(
            process_id.to_string(),
            ProcessReservationState {
                owner_id: owner_id.clone(),
                spawning: false,
            },
        );
        Ok(ProcessReservation {
            process_id: process_id.to_string(),
            owner_id: owner_id.clone(),
        })
    }

    pub fn reservation_matches(&self, reservation: &ProcessReservation) -> bool {
        self.reservations
            .get(reservation.process_id())
            .is_some_and(|state| &state.owner_id == reservation.owner_id())
    }

    /// Pins a reservation across the external spawn call. Finalizers cannot
    /// release it until spawn either aborts or commits its receipt.
    pub fn begin_spawn(&mut self, reservation: &ProcessReservation) -> bool {
        let Some(state) = self.reservations.get_mut(reservation.process_id()) else {
            return false;
        };
        if &state.owner_id != reservation.owner_id() || state.spawning {
            return false;
        }
        state.spawning = true;
        true
    }

    pub fn abort_spawn(&mut self, reservation: &ProcessReservation) {
        if let Some(state) = self.reservations.get_mut(reservation.process_id()) {
            if &state.owner_id == reservation.owner_id() {
                state.spawning = false;
            }
        }
    }

    pub fn commit(&mut self, reservation: &ProcessReservation) -> Result<ProcessReceipt, String> {
        if !self.reservation_matches(reservation)
            || !self
                .reservations
                .get(reservation.process_id())
                .is_some_and(|state| state.spawning)
        {
            return Err(format!(
                "process reservation is missing, consumed, or stale: {}",
                reservation.process_id()
            ));
        }
        self.reservations.remove(reservation.process_id());
        self.owners.insert(
            reservation.process_id().to_string(),
            reservation.owner_id().clone(),
        );
        Ok(ProcessReceipt::from_committed_identity(
            reservation.process_id().to_string(),
            reservation.owner_id().clone(),
        ))
    }

    pub fn authorize(&self, receipt: &ProcessReceipt) -> bool {
        self.owners.get(receipt.process_id()) == Some(receipt.owner_id())
    }

    pub fn authorize_identity(&self, process_id: &str, owner_id: &ProcessOwnerId) -> bool {
        self.owners.get(process_id) == Some(owner_id)
    }

    pub fn release_reservation(&mut self, reservation: &ProcessReservation) -> bool {
        if self.reservation_matches(reservation)
            && !self
                .reservations
                .get(reservation.process_id())
                .is_some_and(|state| state.spawning)
        {
            self.reservations.remove(reservation.process_id());
            true
        } else {
            false
        }
    }

    pub fn finalize_reservation(
        &mut self,
        process_id: &str,
        owner_id: &ProcessOwnerId,
    ) -> OwnedReleaseStatus {
        match self.reservations.get(process_id) {
            Some(state) if &state.owner_id == owner_id && !state.spawning => {
                self.reservations.remove(process_id);
                OwnedReleaseStatus::Released
            }
            Some(_) => OwnedReleaseStatus::DifferentOwner,
            None => OwnedReleaseStatus::Missing,
        }
    }

    /// Must be called only after termination has been confirmed.
    pub fn release_owner(&mut self, receipt: &ProcessReceipt) -> bool {
        if self.authorize(receipt) {
            self.owners.remove(receipt.process_id());
            true
        } else {
            false
        }
    }

    pub fn release_confirmed_process(&mut self, process_id: &str) {
        self.owners.remove(process_id);
    }

    pub fn contains_process(&self, process_id: &str) -> bool {
        self.owners.contains_key(process_id)
    }

    pub fn contains_identity(&self, process_id: &str) -> bool {
        self.owners.contains_key(process_id) || self.reservations.contains_key(process_id)
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn counts(&self) -> (usize, usize) {
        (self.owners.len(), self.reservations.len())
    }
}

impl ReservationSet {
    pub fn reserve(
        &mut self,
        key: &str,
        reservation_id: &ProcessOwnerId,
    ) -> Result<RegistryReservation, String> {
        if self.0.contains_key(key) {
            return Err(format!("id is already reserved: {key}"));
        }
        self.0.insert(key.to_string(), reservation_id.clone());
        Ok(RegistryReservation {
            key: key.to_string(),
            reservation_id: reservation_id.clone(),
        })
    }

    pub fn matches(&self, reservation: &RegistryReservation) -> bool {
        self.0.get(reservation.key()) == Some(reservation.reservation_id())
    }

    pub fn release(&mut self, reservation: &RegistryReservation) -> bool {
        self.release_owned(reservation.key(), reservation.reservation_id())
            == OwnedReleaseStatus::Released
    }

    pub fn consume(&mut self, reservation: &RegistryReservation) -> bool {
        self.release(reservation)
    }

    pub fn release_owned(
        &mut self,
        key: &str,
        reservation_id: &ProcessOwnerId,
    ) -> OwnedReleaseStatus {
        match self.0.get(key) {
            Some(owner) if owner == reservation_id => {
                self.0.remove(key);
                OwnedReleaseStatus::Released
            }
            Some(_) => OwnedReleaseStatus::DifferentOwner,
            None => OwnedReleaseStatus::Missing,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReservedProcessReceipt {
    task_id: String,
    process: ProcessReceipt,
}

impl ReservedProcessReceipt {
    pub fn new(task_id: String, process: ProcessReceipt) -> Self {
        Self { task_id, process }
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn for_test(
        task_id: impl Into<String>,
        process_id: impl Into<String>,
        owner_id: ProcessOwnerId,
    ) -> Self {
        Self::new(
            task_id.into(),
            ProcessReceipt::from_committed_identity(process_id.into(), owner_id),
        )
    }

    pub fn task_id(&self) -> &str {
        &self.task_id
    }

    pub fn executor_id(&self) -> &str {
        self.process.process_id()
    }

    pub fn reservation_id(&self) -> &ProcessOwnerId {
        self.process.owner_id()
    }

    pub fn process(&self) -> &ProcessReceipt {
        &self.process
    }
}

#[derive(Clone, Debug)]
pub struct ReservedProcessLaunchError {
    pub message: String,
    pub created: Option<ReservedProcessReceipt>,
}

impl ReservedProcessLaunchError {
    pub fn before_spawn(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            created: None,
        }
    }

    pub fn after_spawn(message: impl Into<String>, created: ReservedProcessReceipt) -> Self {
        Self {
            message: message.into(),
            created: Some(created),
        }
    }
}

#[cfg(test)]
mod tests;
