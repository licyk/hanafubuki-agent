use std::sync::Arc;

use kardia_macros::Service;

use super::{ContextContributor, ContextSectionKind};
use crate::{RegistrationError, Registry, RegistryService};

/// The world-state contributors, one entry per contributor id. The kernel
/// collects a turn's world state from one snapshot of it.
#[derive(Service, Default)]
#[service(name = "agent-context")]
pub struct ContextRegistry {
    registry: Registry<Arc<dyn ContextContributor>>,
}

impl ContextRegistry {
    /// An empty registry whose entries report their faults to `faults`.
    pub fn with_faults(faults: Arc<crate::PluginFaults>) -> Self {
        Self {
            registry: Registry::with_faults(faults),
        }
    }
}

impl RegistryService for ContextRegistry {
    type Item = Arc<dyn ContextContributor>;

    fn registry(&self) -> &Registry<Self::Item> {
        &self.registry
    }

    /// Rejects a second contributor with the same id, a declared kind that is
    /// not an identifier or that the kernel writes itself, and a kind another
    /// contributor already declares.
    fn admit(existing: &[&Self::Item], candidate: &Self::Item) -> Result<(), RegistrationError> {
        let registry = <Self as kardia_core::Service>::NAME;
        if existing
            .iter()
            .any(|contributor| contributor.id() == candidate.id())
        {
            return Err(RegistrationError::Duplicate {
                registry,
                identity: candidate.id().to_string(),
            });
        }
        for kind in candidate.kinds() {
            if !ContextSectionKind::is_valid(kind.as_str())
                || *kind == ContextSectionKind::TOOL_CATALOG_OMISSIONS
            {
                return Err(RegistrationError::Rejected {
                    registry,
                    reason: format!("{} declares the kind \"{kind}\"", candidate.id()),
                });
            }
            if existing
                .iter()
                .any(|contributor| contributor.kinds().contains(kind))
            {
                return Err(RegistrationError::Duplicate {
                    registry,
                    identity: format!("the context section kind {kind}"),
                });
            }
        }
        Ok(())
    }
}
