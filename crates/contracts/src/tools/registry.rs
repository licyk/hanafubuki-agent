use std::sync::Arc;

use kardia_macros::Service;

use super::{AgentToolHandler, AgentToolSource};
use crate::{AgentActivityAction, RegistrationError, Registry, RegistryService};

/// The names no plugin may register: the kernel's own tools and the plan's
/// `tool_search`, which are not registry entries.
pub const RESERVED_TOOL_NAMES: [&str; 12] = [
    "new_context",
    "update_plan",
    "request_user_input",
    "compress",
    "decompress",
    "search_context",
    "acp_status",
    "acp_cache",
    "absorb",
    "acp_retrieve",
    "acp_rule",
    "tool_search",
];

/// The Agent's tools, one entry per model-visible name: a tool's flattened
/// name, which the planner keys its wire names by.
#[derive(Service, Default)]
#[service(name = "agent-tools")]
pub struct ToolRegistry {
    registry: Registry<Arc<dyn AgentToolHandler>>,
}

impl ToolRegistry {
    /// An empty registry whose entries report their faults to `faults`.
    pub fn with_faults(faults: Arc<crate::PluginFaults>) -> Self {
        Self {
            registry: Registry::with_faults(faults),
        }
    }
}

impl RegistryService for ToolRegistry {
    type Item = Arc<dyn AgentToolHandler>;

    fn registry(&self) -> &Registry<Self::Item> {
        &self.registry
    }

    fn admit(existing: &[&Self::Item], candidate: &Self::Item) -> Result<(), RegistrationError> {
        let descriptor = candidate.descriptor();
        if !AgentActivityAction::is_valid(descriptor.action.as_str()) {
            return Err(RegistrationError::Rejected {
                registry: <Self as kardia_core::Service>::NAME,
                reason: format!(
                    "{} names an invalid activity action",
                    descriptor.name.qualified()
                ),
            });
        }
        let name = descriptor.name.flattened_alias();
        if RESERVED_TOOL_NAMES.contains(&name.as_str()) {
            return Err(RegistrationError::Rejected {
                registry: <Self as kardia_core::Service>::NAME,
                reason: format!("{name} is reserved for the Agent kernel"),
            });
        }
        if hanafubuki_agent_tool_contracts::ToolId::parse(name.clone()).is_err() {
            return Err(RegistrationError::Rejected {
                registry: <Self as kardia_core::Service>::NAME,
                reason: format!("{name} is longer than a tool id"),
            });
        }
        if existing
            .iter()
            .any(|tool| tool.descriptor().name.flattened_alias() == name)
        {
            return Err(RegistrationError::Duplicate {
                registry: <Self as kardia_core::Service>::NAME,
                identity: name,
            });
        }
        Ok(())
    }
}

/// The tool sources, one entry per source id. The kernel snapshots it per
/// sampling step and asks each live source for its tools.
#[derive(Service, Default)]
#[service(name = "agent-tool-sources")]
pub struct ToolSourceRegistry {
    registry: Registry<Arc<dyn AgentToolSource>>,
}

impl ToolSourceRegistry {
    /// An empty registry whose entries report their faults to `faults`.
    pub fn with_faults(faults: Arc<crate::PluginFaults>) -> Self {
        Self {
            registry: Registry::with_faults(faults),
        }
    }
}

impl RegistryService for ToolSourceRegistry {
    type Item = Arc<dyn AgentToolSource>;

    fn registry(&self) -> &Registry<Self::Item> {
        &self.registry
    }

    fn admit(existing: &[&Self::Item], candidate: &Self::Item) -> Result<(), RegistrationError> {
        if existing.iter().any(|source| source.id() == candidate.id()) {
            return Err(RegistrationError::Duplicate {
                registry: <Self as kardia_core::Service>::NAME,
                identity: candidate.id().to_string(),
            });
        }
        Ok(())
    }
}
