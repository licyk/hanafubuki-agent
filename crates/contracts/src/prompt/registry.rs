use std::sync::Arc;

use kardia_macros::Service;

use super::{PromptContributor, PromptFragment, PromptRequest};
use crate::{ExtensionPoint, RegistrationError, Registry, RegistryEntry, RegistryService};

/// The prompt contributors, one entry per contributor id.
#[derive(Service, Default)]
#[service(name = "agent-prompt")]
pub struct PromptContributorRegistry {
    registry: Registry<Arc<dyn PromptContributor>>,
}

impl PromptContributorRegistry {
    /// An empty registry whose entries report their faults to `faults`.
    pub fn with_faults(faults: Arc<crate::PluginFaults>) -> Self {
        Self {
            registry: Registry::with_faults(faults),
        }
    }
}

impl PromptContributorRegistry {
    /// The contributors in force now, in `(rank, id)` order. The kernel takes
    /// one per sampling step, so a request is prepared and rendered by the
    /// plugins that were running when its step started.
    pub fn snapshot(&self) -> PromptContributorSnapshot {
        let mut entries = self.registry.snapshot().entries().to_vec();
        entries.sort_by(|left, right| {
            let (left, right) = (left.item(), right.item());
            left.rank()
                .cmp(&right.rank())
                .then_with(|| left.id().cmp(right.id()))
        });
        PromptContributorSnapshot(entries)
    }
}

/// A frozen, ordered set of prompt contributors.
#[derive(Clone, Default)]
pub struct PromptContributorSnapshot(Vec<Arc<RegistryEntry<Arc<dyn PromptContributor>>>>);

impl PromptContributorSnapshot {
    /// The contributors' entries in contributor order; a `prepare` call
    /// leases its contributor's entry.
    pub fn entries(&self) -> &[Arc<RegistryEntry<Arc<dyn PromptContributor>>>] {
        &self.0
    }

    /// Every contributor's fragments, in contributor order. A contributor
    /// that panics is reported as a fault of its plugin and contributes
    /// nothing.
    pub fn fragments(&self, request: &PromptRequest<'_>) -> Vec<PromptFragment> {
        self.0
            .iter()
            .flat_map(|entry| {
                entry
                    .guard(ExtensionPoint::PromptContribute, |contributor| {
                        contributor.contribute(request)
                    })
                    .unwrap_or_default()
            })
            .collect()
    }
}

impl RegistryService for PromptContributorRegistry {
    type Item = Arc<dyn PromptContributor>;

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
