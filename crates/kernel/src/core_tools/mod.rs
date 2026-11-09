//! The kernel's own tools. They act on the turn itself (its context window,
//! its task checklist, a question to the user) or on the session's
//! hierarchical compression, so they are a fixed set the kernel runs directly
//! rather than registered handlers.

mod arguments;
mod compression_spec;
mod specs;

#[cfg(test)]
mod tests;

use hanafubuki_agent_model_api::ModelToolSpec;
use serde::{Deserialize, Serialize};

use crate::{AgentActivityAction, AgentSessionMode};

pub(crate) use arguments::{CoreToolCall, RequestUserInputTool, UpdateTaskPlanTool};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentCoreTool {
    NewContext,
    UpdatePlan,
    RequestUserInput,
    /// billion-context's core tools, planned while the session's strategy is
    /// `hierarchical`.
    Compress,
    Decompress,
    SearchContext,
    AcpStatus,
    AcpCache,
    /// The opt-in features' tools, each planned only while its feature is
    /// on: `absorb`, the content store's `acp_retrieve`, `acp_rule`.
    Absorb,
    AcpRetrieve,
    AcpRule,
}

impl AgentCoreTool {
    pub const ALL: [Self; 11] = [
        Self::NewContext,
        Self::UpdatePlan,
        Self::RequestUserInput,
        Self::Compress,
        Self::Decompress,
        Self::SearchContext,
        Self::AcpStatus,
        Self::AcpCache,
        Self::Absorb,
        Self::AcpRetrieve,
        Self::AcpRule,
    ];

    /// The tools every session plans.
    pub const STANDARD: [Self; 3] = [Self::NewContext, Self::UpdatePlan, Self::RequestUserInput];

    /// The tools a session with the `hierarchical` strategy plans too, in
    /// billion-context's order.
    pub const COMPRESSION: [Self; 5] = [
        Self::Compress,
        Self::Decompress,
        Self::SearchContext,
        Self::AcpStatus,
        Self::AcpCache,
    ];

    pub const fn name(self) -> &'static str {
        match self {
            Self::NewContext => "new_context",
            Self::UpdatePlan => "update_plan",
            Self::RequestUserInput => "request_user_input",
            Self::Compress => hanafubuki_agent_compression::COMPRESS_TOOL_NAME,
            Self::Decompress => hanafubuki_agent_compression::DECOMPRESS_TOOL_NAME,
            Self::SearchContext => hanafubuki_agent_compression::SEARCH_CONTEXT_TOOL_NAME,
            Self::AcpStatus => hanafubuki_agent_compression::ACP_STATUS_TOOL_NAME,
            Self::AcpCache => hanafubuki_agent_compression::ACP_CACHE_TOOL_NAME,
            Self::Absorb => hanafubuki_agent_compression::ABSORB_TOOL_NAME,
            Self::AcpRetrieve => hanafubuki_agent_compression::RETRIEVE_TOOL_NAME,
            Self::AcpRule => hanafubuki_agent_compression::RULE_TOOL_NAME,
        }
    }

    /// One of the hierarchical compression's tools.
    pub const fn is_compression(self) -> bool {
        matches!(
            self,
            Self::Compress
                | Self::Decompress
                | Self::SearchContext
                | Self::AcpStatus
                | Self::AcpCache
                | Self::Absorb
                | Self::AcpRetrieve
                | Self::AcpRule
        )
    }

    /// A compression tool that changes the session's compression state: it
    /// is applied again when its output is committed, so the state and the
    /// output it explains land in one commit. `acp_retrieve` counts its
    /// retrievals in the state.
    pub const fn changes_compression_state(self) -> bool {
        matches!(
            self,
            Self::Compress | Self::Decompress | Self::Absorb | Self::AcpRetrieve | Self::AcpRule
        )
    }

    pub fn model_spec(self) -> ModelToolSpec {
        // Match Codex's ordinary function-tool contract. The definition keeps
        // its complete local schema, while Tool Plan lowering produces the
        // portable provider projection and Rust remains the validation authority.
        let mut spec = specs::model_spec(self);
        spec.strict = false;
        spec
    }

    /// Plan mode keeps the question to the user: a plan is not a task
    /// checklist, and a fresh context would drop the plan being written. The
    /// compression tools manage the context in either mode.
    pub const fn allowed_in_mode(self, mode: AgentSessionMode) -> bool {
        match mode {
            AgentSessionMode::Default => true,
            AgentSessionMode::Plan => {
                matches!(self, Self::RequestUserInput) || self.is_compression()
            }
        }
    }

    pub(crate) const fn activity_action(self) -> AgentActivityAction {
        match self {
            Self::NewContext
            | Self::Compress
            | Self::Decompress
            | Self::SearchContext
            | Self::AcpStatus
            | Self::AcpCache
            | Self::Absorb
            | Self::AcpRetrieve
            | Self::AcpRule => AgentActivityAction::USE_TOOL,
            Self::UpdatePlan => AgentActivityAction::UPDATE_PLAN,
            Self::RequestUserInput => AgentActivityAction::REQUEST_USER_INPUT,
        }
    }
}
