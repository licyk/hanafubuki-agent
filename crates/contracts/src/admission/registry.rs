use std::sync::Arc;

use kardia_macros::Service;

use super::TurnAdmissionContributor;
use crate::{RegistrationError, Registry, RegistryEntry, RegistryService};

/// The turn admission contributors, one entry per contributor id. The kernel
/// collects a turn's extensions from one snapshot of it.
#[derive(Service, Default)]
#[service(name = "agent-turn-admission")]
pub struct TurnAdmissionRegistry {
    registry: Registry<Arc<dyn TurnAdmissionContributor>>,
}

impl TurnAdmissionRegistry {
    /// An empty registry whose entries report their faults to `faults`.
    pub fn with_faults(faults: Arc<crate::PluginFaults>) -> Self {
        Self {
            registry: Registry::with_faults(faults),
        }
    }
}

impl TurnAdmissionRegistry {
    /// A snapshot of the contributors in `(rank, id)` order, never
    /// registration order.
    pub fn ordered(&self) -> Vec<Arc<RegistryEntry<Arc<dyn TurnAdmissionContributor>>>> {
        let mut entries = self.registry.snapshot().entries().to_vec();
        entries.sort_by(|left, right| {
            let (left, right) = (left.item(), right.item());
            left.rank()
                .cmp(&right.rank())
                .then_with(|| left.id().cmp(right.id()))
        });
        entries
    }
}

impl RegistryService for TurnAdmissionRegistry {
    type Item = Arc<dyn TurnAdmissionContributor>;

    fn registry(&self) -> &Registry<Self::Item> {
        &self.registry
    }

    /// Rejects a second contributor with the same id.
    fn admit(existing: &[&Self::Item], candidate: &Self::Item) -> Result<(), RegistrationError> {
        if existing
            .iter()
            .any(|contributor| contributor.id() == candidate.id())
        {
            return Err(RegistrationError::Duplicate {
                registry: <Self as kardia_core::Service>::NAME,
                identity: candidate.id().to_string(),
            });
        }
        Ok(())
    }
}
