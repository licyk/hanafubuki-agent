use std::sync::Arc;

use kardia_macros::Service;

use super::{AgentNotification, AgentObserver};
use crate::{ExtensionPoint, RegistrationError, Registry, RegistryService};

/// The observers, one entry per observer id.
#[derive(Service, Default)]
#[service(name = "agent-observers")]
pub struct ObserverRegistry {
    registry: Registry<Arc<dyn AgentObserver>>,
}

impl ObserverRegistry {
    /// An empty registry whose entries report their faults to `faults`.
    pub fn with_faults(faults: Arc<crate::PluginFaults>) -> Self {
        Self {
            registry: Registry::with_faults(faults),
        }
    }
}

impl ObserverRegistry {
    /// Tells every observer whose plugin is running about `notification`,
    /// one after another on the calling thread.
    ///
    /// `observe` is bounded and synchronous, so a call holds no lease: a
    /// stopping plugin's observer is skipped from the moment its retirement
    /// starts. A panicking observer is reported as a fault of its plugin and
    /// the others are still told; the panic never reaches the caller.
    pub fn notify(&self, notification: &AgentNotification) {
        for entry in self.registry.snapshot().entries() {
            if !entry.is_retired() {
                let _ = entry.guard(ExtensionPoint::Observer, |observer| {
                    observer.observe(notification)
                });
            }
        }
    }
}

impl RegistryService for ObserverRegistry {
    type Item = Arc<dyn AgentObserver>;

    fn registry(&self) -> &Registry<Self::Item> {
        &self.registry
    }

    /// Rejects a second observer with the same id.
    fn admit(existing: &[&Self::Item], candidate: &Self::Item) -> Result<(), RegistrationError> {
        if existing
            .iter()
            .any(|observer| observer.id() == candidate.id())
        {
            return Err(RegistrationError::Duplicate {
                registry: <Self as kardia_core::Service>::NAME,
                identity: candidate.id().to_string(),
            });
        }
        Ok(())
    }
}
