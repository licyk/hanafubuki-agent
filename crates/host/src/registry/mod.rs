//! The one registry mechanism behind every extension point a host's plugins
//! contribute to (each Agent extension point is one).
//!
//! A [`Registry`] holds entries contributed by plugins. Its reader never
//! iterates it under its lock: it takes a [`RegistrySnapshot`] (the Agent
//! kernel one per turn or sampling step) and works on that immutable list. An entry contributed by a
//! plugin is removed when that plugin stops; calls already running against the
//! entry hold an [`EntryLease`] and are drained (then cancelled) before the
//! plugin's own services are torn down.

mod lease;
mod service;

use std::fmt;
use std::future::Future;
use std::sync::Arc;

use kardia_core::IdTable;
use parking_lot::Mutex;

use crate::fault::{FaultPoint, PluginFaults, PluginPanicked};

pub use lease::{EntryLease, DRAIN_GRACE, RETIRE_CANCEL_BOUND};
pub use service::{Register, RegistryService};

use lease::EntryState;

pub struct Registry<T> {
    state: Mutex<RegistryState<T>>,
    /// Where a panic in an entry's plugin code is reported; every entry
    /// holds it.
    faults: Arc<PluginFaults>,
}

struct RegistryState<T> {
    entries: IdTable<Arc<RegistryEntry<T>>>,
    revision: u64,
}

impl<T> Default for Registry<T> {
    /// A registry whose faults are only logged; a host's registries share
    /// one [`PluginFaults`] ([`Self::with_faults`]).
    fn default() -> Self {
        Self::with_faults(Arc::default())
    }
}

impl<T> Registry<T> {
    pub fn with_faults(faults: Arc<PluginFaults>) -> Self {
        Self {
            state: Mutex::new(RegistryState {
                entries: IdTable::new(),
                revision: 0,
            }),
            faults,
        }
    }
}

impl<T: Send + Sync + 'static> Registry<T> {
    /// The live entries in registration order.
    pub fn snapshot(&self) -> RegistrySnapshot<T> {
        let state = self.state.lock();
        RegistrySnapshot {
            revision: state.revision,
            entries: state.entries.values().cloned().collect(),
        }
    }

    /// Adds an entry no plugin owns, for what the host's own composition
    /// contributes: it is never retired. `owner` names it in faults. A
    /// plugin registers through [`Register`] instead, which removes its
    /// entry when it stops.
    pub fn register_permanent(
        &self,
        owner: impl Into<String>,
        item: T,
        admit: impl FnOnce(&[&T], &T) -> Result<(), RegistrationError>,
    ) -> Result<(), RegistrationError> {
        self.insert(owner.into(), item, admit).map(|_| ())
    }

    pub(crate) fn insert(
        &self,
        owner: String,
        item: T,
        admit: impl FnOnce(&[&T], &T) -> Result<(), RegistrationError>,
    ) -> Result<Arc<RegistryEntry<T>>, RegistrationError> {
        let mut state = self.state.lock();
        let existing = state
            .entries
            .values()
            .map(|entry| &entry.item)
            .collect::<Vec<_>>();
        admit(&existing, &item)?;
        let entry = Arc::new(RegistryEntry {
            owner,
            item,
            state: Arc::new(EntryState::default()),
            faults: self.faults.clone(),
        });
        state.entries.insert(entry.clone());
        state.revision += 1;
        Ok(entry)
    }

    /// Removes `entry` from future snapshots, then waits for calls that still
    /// hold a lease on it (see [`lease`]).
    pub(crate) async fn retire(&self, entry: &Arc<RegistryEntry<T>>) {
        {
            let mut state = self.state.lock();
            let id = state
                .entries
                .iter()
                .find_map(|(id, candidate)| Arc::ptr_eq(candidate, entry).then_some(id));
            if let Some(id) = id {
                state.entries.remove(id);
                state.revision += 1;
            }
        }
        entry.state.retire_and_drain(&entry.owner).await;
    }
}

pub struct RegistryEntry<T> {
    owner: String,
    item: T,
    state: Arc<EntryState>,
    faults: Arc<PluginFaults>,
}

impl<T> RegistryEntry<T> {
    #[cfg(any(test, feature = "test-support"))]
    pub(crate) fn detached(owner: String, item: T) -> Arc<Self> {
        Arc::new(Self {
            owner,
            item,
            state: Arc::new(EntryState::default()),
            faults: Arc::default(),
        })
    }

    pub fn item(&self) -> &T {
        &self.item
    }

    /// The name of the plugin that registered the entry.
    pub fn owner(&self) -> &str {
        &self.owner
    }

    /// Pins the entry for one call; `None` once its plugin is stopping. The
    /// plugin stopping cancels `call` if the call outlives the drain grace.
    pub fn acquire(
        &self,
        call: &hanafubuki_agent_foundation::cancellation::CancellationToken,
    ) -> Option<EntryLease> {
        EntryLease::acquire(&self.state, call)
    }

    pub fn is_retired(&self) -> bool {
        self.state.is_retired()
    }

    /// Where a panic in this entry's plugin code is reported.
    pub fn faults(&self) -> &PluginFaults {
        &self.faults
    }

    /// Calls the entry's plugin code at `point`: a panic is reported as a
    /// fault of its owner and becomes [`PluginPanicked`], which the caller
    /// turns into the call's own error. Never call it under a lock of the
    /// caller's own state.
    pub fn guard<R>(
        &self,
        point: impl Into<FaultPoint>,
        call: impl FnOnce(&T) -> R,
    ) -> Result<R, PluginPanicked> {
        self.faults.guard(&self.owner, point, || call(&self.item))
    }

    /// [`Self::guard`] for an async call; `call` builds the future.
    pub async fn guard_async<'a, F, Fut>(
        &'a self,
        point: impl Into<FaultPoint>,
        call: F,
    ) -> Result<Fut::Output, PluginPanicked>
    where
        F: FnOnce(&'a T) -> Fut,
        Fut: Future,
    {
        let item = &self.item;
        self.faults
            .guard_async(&self.owner, point, move || call(item))
            .await
    }
}

/// An immutable view of a registry. Entries stay usable after their plugin
/// stops only through a lease taken before the retirement.
pub struct RegistrySnapshot<T> {
    revision: u64,
    entries: Vec<Arc<RegistryEntry<T>>>,
}

impl<T> Clone for RegistrySnapshot<T> {
    fn clone(&self) -> Self {
        Self {
            revision: self.revision,
            entries: self.entries.clone(),
        }
    }
}

impl<T> RegistrySnapshot<T> {
    /// Increases on every registration and removal.
    pub fn revision(&self) -> u64 {
        self.revision
    }

    pub fn entries(&self) -> &[Arc<RegistryEntry<T>>] {
        &self.entries
    }

    pub fn items(&self) -> impl Iterator<Item = &T> {
        self.entries.iter().map(|entry| &entry.item)
    }

    /// The entry of a single-provider registry (a slot), if one is registered.
    pub fn single(&self) -> Option<&Arc<RegistryEntry<T>>> {
        self.entries.first()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }
}

/// A registration the registry refused; it fails the registering plugin.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RegistrationError {
    /// A single-provider registry already has its provider.
    Occupied { registry: &'static str },
    /// An entry with the same identity is already registered.
    Duplicate {
        registry: &'static str,
        identity: String,
    },
    /// The item itself is not admissible.
    Rejected {
        registry: &'static str,
        reason: String,
    },
}

impl fmt::Display for RegistrationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Occupied { registry } => {
                write!(formatter, "{registry} already has a provider")
            }
            Self::Duplicate { registry, identity } => {
                write!(formatter, "{registry} already contains {identity}")
            }
            Self::Rejected { registry, reason } => {
                write!(formatter, "{registry} rejected a registration: {reason}")
            }
        }
    }
}

impl std::error::Error for RegistrationError {}
