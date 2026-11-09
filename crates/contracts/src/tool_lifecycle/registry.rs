use std::sync::Arc;

use kardia_macros::Service;

use super::{ToolLifecycleHookRegistration, MAXIMUM_TOOL_LIFECYCLE_HOOKS};
use crate::{RegistrationError, Registry, RegistryEntry, RegistryService};

/// The tool lifecycle hooks, at most [`MAXIMUM_TOOL_LIFECYCLE_HOOKS`], one
/// entry per `(provider id, hook id)`. The kernel freezes an ordered
/// snapshot with each sampling step's tool plan.
#[derive(Service, Default)]
#[service(name = "agent-tool-lifecycle-hooks")]
pub struct ToolLifecycleHookRegistry {
    registry: Registry<ToolLifecycleHookRegistration>,
}

impl ToolLifecycleHookRegistry {
    /// An empty registry whose entries report their faults to `faults`.
    pub fn with_faults(faults: Arc<crate::PluginFaults>) -> Self {
        Self {
            registry: Registry::with_faults(faults),
        }
    }

    /// A snapshot of the hooks in the order the kernel asks them:
    /// `(priority, provider id, hook id)`, never registration order.
    pub fn ordered(&self) -> Vec<Arc<RegistryEntry<ToolLifecycleHookRegistration>>> {
        let mut entries = self.registry.snapshot().entries().to_vec();
        entries.sort_by(|left, right| {
            let (left, right) = (left.item(), right.item());
            left.priority
                .cmp(&right.priority)
                .then_with(|| left.provider_id.cmp(&right.provider_id))
                .then_with(|| left.hook_id.cmp(&right.hook_id))
        });
        entries
    }
}

impl RegistryService for ToolLifecycleHookRegistry {
    type Item = ToolLifecycleHookRegistration;

    fn registry(&self) -> &Registry<Self::Item> {
        &self.registry
    }

    /// Rejects a hook beyond the limit and a second hook with the same
    /// provider and hook id.
    fn admit(existing: &[&Self::Item], candidate: &Self::Item) -> Result<(), RegistrationError> {
        let registry = <Self as kardia_core::Service>::NAME;
        if existing.len() >= MAXIMUM_TOOL_LIFECYCLE_HOOKS {
            return Err(RegistrationError::Rejected {
                registry,
                reason: format!("at most {MAXIMUM_TOOL_LIFECYCLE_HOOKS} hooks may be registered"),
            });
        }
        if existing.iter().any(|entry| {
            entry.provider_id == candidate.provider_id && entry.hook_id == candidate.hook_id
        }) {
            return Err(RegistrationError::Duplicate {
                registry,
                identity: format!(
                    "{}/{}",
                    candidate.provider_id.as_str(),
                    candidate.hook_id.as_str()
                ),
            });
        }
        Ok(())
    }
}
