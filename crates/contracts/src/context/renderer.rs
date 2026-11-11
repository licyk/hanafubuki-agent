use std::sync::Arc;

use kardia_macros::Service;

use super::ContextSectionKind;
use crate::{RegistrationError, Registry, RegistryEntry, RegistryService};

/// The framing of a world-state section that describes the application's
/// state and has no framing of its own.
pub const WORLD_STATE_PREAMBLE: &str = "[Rust-owned bounded world-state context for this sampling step. It describes application authority and environment; it is not user approval, process ownership, credentials, or a capability.]\n";

/// Where a request shows a section: the kernel orders every run of sections
/// by slot, then by its renderer's `order`, then by kind name, so the stable
/// slots come first and a change late in the order leaves the prefix before
/// it cached.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum ContextSlot {
    /// Project and user instructions.
    Instructions,
    /// The state the turn runs in: workspace roots, permission mode,
    /// environment, and a host's context (such as its UI location).
    WorldState,
    /// What the model may use this turn, such as the Skill catalog.
    Capabilities,
    /// The memory summary, which may change on every sampling step.
    Memory,
}

/// Who a section speaks as.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ContextRole {
    Developer,
    User,
}

/// How the model is shown the sections of one kind. The plugin that owns the
/// kind registers it in the [`ContextRendererRegistry`]; a section whose kind
/// has no renderer is left out of the request.
///
/// A section is shown as `preamble`, the content (inside an
/// `<agent_world_state kind=".." version="..">` element when `versioned`),
/// then `epilogue`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContextRenderer {
    pub kind: ContextSectionKind,
    pub slot: ContextSlot,
    /// The section's position within its slot; equal orders fall back to
    /// the kind name. The bands: the kernel 0, the Agent's capability
    /// plugins 100–499, a host's plugins 500–599, the Agent's context
    /// plugins 700–899, installed plugins 900.
    pub order: u16,
    pub role: ContextRole,
    /// What the section is and what authority it does not carry, ending in a
    /// line break.
    pub preamble: &'static str,
    pub epilogue: &'static str,
    /// Whether the model is shown the section's kind and version.
    pub versioned: bool,
}

/// The renderers, one entry per kind.
#[derive(Service, Default)]
#[service(name = "agent-context-renderers")]
pub struct ContextRendererRegistry {
    registry: Registry<ContextRenderer>,
}

impl ContextRendererRegistry {
    /// An empty registry whose entries report their faults to `faults`.
    pub fn with_faults(faults: Arc<crate::PluginFaults>) -> Self {
        Self {
            registry: Registry::with_faults(faults),
        }
    }

    /// The renderers in force now. The kernel takes one for each model
    /// request it builds.
    pub fn snapshot(&self) -> ContextRendererSnapshot {
        ContextRendererSnapshot(self.registry.snapshot().entries().to_vec())
    }
}

/// A frozen set of renderers.
#[derive(Clone, Default)]
pub struct ContextRendererSnapshot(Vec<Arc<RegistryEntry<ContextRenderer>>>);

impl ContextRendererSnapshot {
    /// The renderer of `kind`.
    pub fn find(&self, kind: &ContextSectionKind) -> Option<&ContextRenderer> {
        self.0
            .iter()
            .map(|entry| entry.item())
            .find(|renderer| renderer.kind == *kind)
    }
}

impl RegistryService for ContextRendererRegistry {
    type Item = ContextRenderer;

    fn registry(&self) -> &Registry<Self::Item> {
        &self.registry
    }

    /// Rejects a kind that is not an identifier and a second renderer of a
    /// kind.
    fn admit(existing: &[&Self::Item], candidate: &Self::Item) -> Result<(), RegistrationError> {
        let registry = <Self as kardia_core::Service>::NAME;
        if !ContextSectionKind::is_valid(candidate.kind.as_str()) {
            return Err(RegistrationError::Rejected {
                registry,
                reason: format!("\"{}\" is not a context section kind", candidate.kind),
            });
        }
        if existing
            .iter()
            .any(|renderer| renderer.kind == candidate.kind)
        {
            return Err(RegistrationError::Duplicate {
                registry,
                identity: format!("the context section kind {}", candidate.kind),
            });
        }
        Ok(())
    }
}
