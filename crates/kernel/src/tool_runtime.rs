use std::sync::Arc;

use serde_json::Value;

pub use hanafubuki_agent_contracts::{
    AgentToolError, AgentToolProgressSink, NoopAgentToolProgressSink,
};

use crate::{
    AgentGeneration, AgentSessionId, AgentToolCatalogRevision, AgentToolGeneration, AgentToolId,
    AgentToolProviderId, AgentTurnId,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentToolOutput {
    pub output: hanafubuki_agent_tool_contracts::ToolOutput,
    pub prepared_images: Vec<crate::PreparedAgentImage>,
    pub selected_tools: Vec<crate::AgentToolSelection>,
    pub tool_search_results: Vec<hanafubuki_agent_tool_contracts::ToolSpec>,
    /// Canonical Skill instructions to append immediately after this tool's
    /// output, from a handler's `ToolContextFragment`. Only the Agent core
    /// persists this as developer context.
    pub skill_context: Option<ActivatedSkillContext>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ActivatedSkillContext {
    pub skill: hanafubuki_agent_skills::SkillId,
    pub name: String,
    pub content: String,
}

impl AgentToolOutput {
    pub fn success(value: Value) -> Self {
        Self {
            output: hanafubuki_agent_tool_contracts::ToolOutput::structured(value, false),
            prepared_images: Vec::new(),
            selected_tools: Vec::new(),
            tool_search_results: Vec::new(),
            skill_context: None,
        }
    }

    pub fn model_visible_error(value: Value) -> Self {
        Self {
            output: hanafubuki_agent_tool_contracts::ToolOutput::structured(value, true),
            prepared_images: Vec::new(),
            selected_tools: Vec::new(),
            tool_search_results: Vec::new(),
            skill_context: None,
        }
    }

    pub fn typed(output: hanafubuki_agent_tool_contracts::ToolOutput) -> Self {
        Self {
            output,
            prepared_images: Vec::new(),
            selected_tools: Vec::new(),
            tool_search_results: Vec::new(),
            skill_context: None,
        }
    }

    pub fn image(image: crate::PreparedAgentImage) -> Self {
        Self {
            output: hanafubuki_agent_tool_contracts::ToolOutput {
                content: Vec::new(),
                structured_content: None,
                is_error: false,
            },
            prepared_images: vec![image],
            selected_tools: Vec::new(),
            tool_search_results: Vec::new(),
            skill_context: None,
        }
    }

    pub fn is_error(&self) -> bool {
        self.output.is_error
    }
}

impl From<Value> for AgentToolOutput {
    fn from(value: Value) -> Self {
        Self::success(value)
    }
}

impl From<hanafubuki_agent_tool_contracts::ToolOutput> for AgentToolOutput {
    fn from(output: hanafubuki_agent_tool_contracts::ToolOutput) -> Self {
        Self::typed(output)
    }
}

#[derive(Clone)]
pub struct AgentToolInvocationContext {
    pub session_id: AgentSessionId,
    pub turn_id: AgentTurnId,
    pub generation: AgentGeneration,
    pub mode: crate::AgentSessionMode,
    pub input_capabilities: hanafubuki_agent_model_api::ModelInputCapabilities,
    pub session_revision: crate::AgentRevision,
    pub call_id: String,
    pub catalog_revision: AgentToolCatalogRevision,
    pub provider_id: AgentToolProviderId,
    pub tool_id: AgentToolId,
    pub tool_generation: AgentToolGeneration,
    /// The opaque leases the tool sources froze into the exact sampling-step
    /// Tool Plan. The Agent only retains and forwards them; a source's tools
    /// downcast their own lease type and must reject a missing binding.
    pub tool_plan_leases: Vec<Arc<dyn hanafubuki_agent_contracts::ToolPlanLease>>,
    /// The approval rules frozen with the step's plan.
    pub approval_rules: Option<hanafubuki_agent_contracts::ApprovalRuleSnapshot>,
    pub progress: Arc<dyn AgentToolProgressSink>,
}
