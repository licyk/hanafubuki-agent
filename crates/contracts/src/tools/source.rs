use std::sync::Arc;

use hanafubuki_agent_foundation::cancellation::CancellationToken;
use hanafubuki_agent_tool_contracts::{ToolGeneration, ToolId, ToolProviderId};
use serde::{Deserialize, Serialize};

use super::{AgentToolError, AgentToolHandler, ToolFuture, ToolPlanLease};
use crate::{AgentGeneration, AgentRevision, AgentSessionId, AgentTurnId};

/// The most omissions one tool plan reports to the model.
pub const MAX_AGENT_TOOL_OMISSIONS: usize = 64;

/// Whether a failure fails the whole tool plan (`Required`) or only leaves
/// the source, or one of its tools, out of it with an omission (`Optional`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToolSourceTrust {
    Required,
    Optional,
}

/// The sampling step a source plans tools for.
#[derive(Clone)]
pub struct ToolSourceStep {
    pub session_id: AgentSessionId,
    pub turn_id: AgentTurnId,
    pub generation: AgentGeneration,
    pub session_revision: AgentRevision,
    /// Whether the session contributes to Agent memory at this step.
    pub memory_contribution: bool,
    /// Cancelled with the step, and when the source's plugin stops while
    /// the source is still contributing.
    pub cancellation: CancellationToken,
}

/// One tool a source plans for one step: a handler the kernel runs through
/// its pipeline, with the identity the plan records for it.
///
/// The model sees the descriptor's name as its flattened alias; a namespaced
/// name becomes a provider namespace described by `namespace_description`
/// where the provider supports namespaces. `(provider_id, tool_id,
/// generation)` is the identity a persisted deferred-tool selection names,
/// so a source keeps it stable for as long as the tool does not change.
#[derive(Clone)]
pub struct SourcedTool {
    pub handler: Arc<dyn AgentToolHandler>,
    pub provider_id: ToolProviderId,
    pub tool_id: ToolId,
    pub generation: ToolGeneration,
    pub namespace_description: Option<String>,
    /// What the activity shows the tool acting on (`MCP files`).
    pub subject: Option<String>,
    pub trust: ToolSourceTrust,
}

impl std::fmt::Debug for SourcedTool {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SourcedTool")
            .field("name", &self.handler.descriptor().name)
            .field("provider_id", &self.provider_id)
            .field("tool_id", &self.tool_id)
            .field("generation", &self.generation)
            .field("trust", &self.trust)
            .finish_non_exhaustive()
    }
}

/// A tool left out of a plan, reported to the model in the
/// `tool_catalog_omissions` world-state section.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentToolOmission {
    pub contributor: String,
    pub tool_name: Option<String>,
    pub reason: String,
}

/// What a source contributes to one step's plan.
#[derive(Clone, Default)]
pub struct ToolSourceContribution {
    pub tools: Vec<SourcedTool>,
    pub omissions: Vec<AgentToolOmission>,
    /// Kept by the plan and handed to every call planned from it, so the
    /// source's tools run against the state they were planned from.
    pub lease: Option<Arc<dyn ToolPlanLease>>,
}

impl std::fmt::Debug for ToolSourceContribution {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ToolSourceContribution")
            .field("tools", &self.tools)
            .field("omissions", &self.omissions)
            .field("has_lease", &self.lease.is_some())
            .finish()
    }
}

impl ToolSourceContribution {
    pub fn from_tools(tools: Vec<SourcedTool>) -> Self {
        Self {
            tools,
            omissions: Vec::new(),
            lease: None,
        }
    }

    pub fn with_lease(mut self, lease: Arc<dyn ToolPlanLease>) -> Self {
        self.lease = Some(lease);
        self
    }
}

/// Plans tools per sampling step, registered in
/// [`super::ToolSourceRegistry`]: for tools that exist only while something
/// outside the Agent offers them (an MCP server's catalog, memory settings).
///
/// The kernel asks every source once per step, under an entry lease, and
/// plans its tools like registered ones: the same pipeline, the same Plan
/// mode criterion ([`super::ToolEffect::ReadOnly`]), plus a check of the
/// model's arguments against the descriptor's schema before `permission`.
pub trait AgentToolSource: Send + Sync + 'static {
    /// Unique within the registry; names the source in omissions.
    fn id(&self) -> &str;

    /// Whether a failing `contribute` fails the plan.
    fn trust(&self) -> ToolSourceTrust {
        ToolSourceTrust::Required
    }

    /// `AgentToolError::Cancelled` when `step.cancellation` was cancelled.
    fn contribute<'a>(
        &'a self,
        step: &'a ToolSourceStep,
    ) -> ToolFuture<'a, Result<ToolSourceContribution, AgentToolError>>;
}
