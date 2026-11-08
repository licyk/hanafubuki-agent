use std::sync::Arc;

use hanafubuki_agent_approval::ApprovalAction;
use kardia_macros::Service;

use super::{ApprovalRule, ApprovalRuleVerdict};
use crate::{ExtensionPoint, RegistrationError, Registry, RegistryService, RegistrySnapshot};

/// The approval rules, one entry per rule id. The kernel asks one snapshot
/// of it for every action that needs approval.
#[derive(Service, Default)]
#[service(name = "agent-approval-rules")]
pub struct ApprovalRuleRegistry {
    registry: Registry<Arc<dyn ApprovalRule>>,
}

impl ApprovalRuleRegistry {
    /// An empty registry whose entries report their faults to `faults`.
    pub fn with_faults(faults: Arc<crate::PluginFaults>) -> Self {
        Self {
            registry: Registry::with_faults(faults),
        }
    }
}

impl ApprovalRuleRegistry {
    /// The rules in force now. The kernel takes one with each sampling
    /// step's tool plan, so a call is judged by the rules of the plugins that
    /// were running when the step's tools were planned.
    pub fn snapshot(&self) -> ApprovalRuleSnapshot {
        ApprovalRuleSnapshot(self.registry.snapshot())
    }
}

/// A frozen set of approval rules.
#[derive(Clone)]
pub struct ApprovalRuleSnapshot(RegistrySnapshot<Arc<dyn ApprovalRule>>);

impl ApprovalRuleSnapshot {
    /// The combined verdict of the registered rules for `action`.
    ///
    /// A denial beats `RequireUser`, which beats no opinion. Several denials
    /// give their distinct reasons sorted and joined, so the verdict is the
    /// same whatever order the rules were registered in. A rule that panics
    /// fails closed: it denies the action (and is reported as a fault of its
    /// plugin), never "no opinion".
    pub fn verdict(&self, action: &ApprovalAction) -> Option<ApprovalRuleVerdict> {
        let mut require_user = false;
        let mut denials = Vec::new();
        for entry in self.0.entries() {
            match entry.guard(ExtensionPoint::ApprovalRule, |rule| rule.evaluate(action)) {
                Ok(None) => {}
                Ok(Some(ApprovalRuleVerdict::RequireUser)) => require_user = true,
                Ok(Some(ApprovalRuleVerdict::Deny { reason })) => denials.push(reason),
                Err(_) => denials.push(format!(
                    "an approval rule of the {} plugin failed with an internal error",
                    entry.owner()
                )),
            }
        }
        if !denials.is_empty() {
            denials.sort();
            denials.dedup();
            return Some(ApprovalRuleVerdict::Deny {
                reason: denials.join("; "),
            });
        }
        require_user.then_some(ApprovalRuleVerdict::RequireUser)
    }
}

impl RegistryService for ApprovalRuleRegistry {
    type Item = Arc<dyn ApprovalRule>;

    fn registry(&self) -> &Registry<Self::Item> {
        &self.registry
    }

    /// Rejects a second rule with the same id.
    fn admit(existing: &[&Self::Item], candidate: &Self::Item) -> Result<(), RegistrationError> {
        if existing.iter().any(|rule| rule.id() == candidate.id()) {
            return Err(RegistrationError::Duplicate {
                registry: <Self as kardia_core::Service>::NAME,
                identity: candidate.id().to_string(),
            });
        }
        Ok(())
    }
}
