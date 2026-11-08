use std::collections::{BTreeSet, HashSet};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use hanafubuki_agent_foundation::cancellation::CancellationToken;
use hanafubuki_agent_model_api::ModelToolSpec;
use hanafubuki_agent_tool_contracts::{
    validate_json_schema, FunctionToolSpec, NamespaceToolSpec, ToolExposure, ToolName,
    ToolSearchSpec, ToolSpec,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

use hanafubuki_agent_contracts::{
    AgentToolHandler, AgentToolOmission, AgentToolSource, EntryLease, RegistryEntry,
    RegistrySnapshot, SourcedTool, ToolPlanLease, ToolSourceTrust, MAX_AGENT_TOOL_OMISSIONS,
};

use crate::{
    AgentActivityAction, AgentGeneration, AgentSessionId, AgentToolCatalogRevision,
    AgentToolExecutionClass, AgentToolGeneration, AgentToolId, AgentToolPresentation,
    AgentToolProviderId, AgentTurnId,
};

pub const MAX_AGENT_TOOL_CATALOG_ENTRIES: usize = 4_096;
pub const MAX_AGENT_DIRECT_TOOL_ENTRIES: usize = 128;
pub const MAX_AGENT_TOOL_CATALOG_SCHEMA_BYTES: usize = 16 * 1024 * 1024;
pub const MAX_AGENT_DIRECT_TOOL_SCHEMA_BYTES: usize = 2 * 1024 * 1024;
pub const MAX_AGENT_TOOL_SEARCH_RESULTS: usize = 20;
pub const DEFAULT_AGENT_TOOL_SEARCH_RESULTS: usize = 8;
pub(crate) type AgentToolContributionFuture<'a> =
    Pin<Box<dyn Future<Output = Result<AgentToolContribution, AgentToolCatalogError>> + Send + 'a>>;

mod arguments;
mod contributor;
mod error;
mod kernel_tools;
mod plan;
mod provider_projection;
mod provider_table;
mod registered;
mod registration;
mod search_index;
mod sourced;
mod spec_cache;

pub(crate) use arguments::normalize_tool_arguments;
pub use contributor::{AgentToolCatalogContext, AgentToolSelection};
pub(crate) use contributor::{AgentToolContribution, AgentToolContributor, AgentToolPlanner};
pub use error::{validate_dynamic_tool_arguments, AgentToolCatalogError};
pub(crate) use kernel_tools::CoreAgentToolContributor;
pub use plan::{AgentToolPlan, AgentToolSearchMatch};
pub(crate) use registered::RegisteredAgentToolContributor;
pub use registration::{
    AgentMemoryContextEffect, AgentPlannedTool, AgentPlannedToolBinding, AgentToolHandlerBinding,
    AgentToolTrust,
};
pub(crate) use search_index::coalesce_search_specs;
use search_index::{build_search_info, AgentToolSearchIndex};
pub use search_index::{AgentToolSearchIndexCache, AgentToolSearchInfo};
use sourced::SourcedAgentToolContributor;
pub(crate) use spec_cache::AgentToolSpecCache;

#[cfg(test)]
use kernel_tools::core_tool_registrations;
use provider_projection::*;
use provider_table::{assign_provider_names, ProviderToolTable};

#[cfg(test)]
mod tests;
