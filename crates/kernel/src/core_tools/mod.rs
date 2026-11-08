//! The kernel's own tools. They act on the turn itself (its context window,
//! its task checklist, a question to the user), so they are a fixed set the
//! kernel runs directly rather than registered handlers.

mod arguments;
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
}

impl AgentCoreTool {
    pub const ALL: [Self; 3] = [Self::NewContext, Self::UpdatePlan, Self::RequestUserInput];

    pub const fn name(self) -> &'static str {
        match self {
            Self::NewContext => "new_context",
            Self::UpdatePlan => "update_plan",
            Self::RequestUserInput => "request_user_input",
        }
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
    /// checklist, and a fresh context would drop the plan being written.
    pub const fn allowed_in_mode(self, mode: AgentSessionMode) -> bool {
        match mode {
            AgentSessionMode::Default => true,
            AgentSessionMode::Plan => matches!(self, Self::RequestUserInput),
        }
    }

    pub(crate) const fn activity_action(self) -> AgentActivityAction {
        match self {
            Self::NewContext => AgentActivityAction::USE_TOOL,
            Self::UpdatePlan => AgentActivityAction::UPDATE_PLAN,
            Self::RequestUserInput => AgentActivityAction::REQUEST_USER_INPUT,
        }
    }
}
