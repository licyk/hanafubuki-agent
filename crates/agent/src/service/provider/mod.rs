#[cfg(test)]
mod tests;

/// The Provider registry is projected to IPC exactly as it is persisted.
///
/// Credentials are stored verbatim and are readable, so there is nothing to
/// strip on the way out and no presence-only shadow type to keep in sync.
mod contracts;
mod registry;
mod resolution;
mod selection;

pub use contracts::{
    AgentModelPolicyWriteRequest, AgentProviderDeleteRequest, AgentProviderDraft,
    AgentProviderEnabledRequest, AgentProviderModelCatalogRequest, AgentProviderModelCatalogResult,
    AgentProviderRegistrySnapshot, AgentProviderWriteRequest, AgentSessionModelSelectionRequest,
    AgentSessionReasoningEffortRequest,
};
pub(crate) use contracts::{ResolvedAgentProvider, ResolvedModelPolicy};
#[cfg(test)]
use resolution::resolve_model_policy;
pub(crate) use resolution::{resolve_provider_definition, resolve_settings_provider};
