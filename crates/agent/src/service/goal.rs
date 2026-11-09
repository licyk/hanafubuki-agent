//! The session goal's feature calls, served by the `goal` plugin
//! (`site/content/docs/sessions.en.mdx`, "Goals").

use std::sync::Arc;

use hanafubuki_agent_kernel::{AgentError, AgentGoal, AgentSessionId};

use super::AgentApplicationService;
use crate::plugins::{AgentGoalBudgets, GoalService, GOAL_PLUGIN};

impl AgentApplicationService {
    /// The session's goal, if it has one.
    pub async fn goal(&self, session_id: AgentSessionId) -> Result<Option<AgentGoal>, AgentError> {
        self.feature(GOAL_PLUGIN, |goals: Arc<GoalService>| async move {
            goals.goal(session_id).await
        })
        .await
    }

    /// Gives the session a new active goal, replacing the one it has (its
    /// usage starts again). The objective is trimmed and at most
    /// `MAXIMUM_GOAL_OBJECTIVE_CHARS` characters; budgets are positive. A
    /// session kept in memory only is refused. A turn that is running
    /// continues toward the new objective at its next final answer.
    pub async fn set_goal(
        &self,
        session_id: AgentSessionId,
        objective: String,
        budgets: AgentGoalBudgets,
    ) -> Result<AgentGoal, AgentError> {
        self.feature(GOAL_PLUGIN, |goals: Arc<GoalService>| async move {
            goals.set(session_id, objective, budgets).await
        })
        .await
    }

    /// Removes the session's goal; whether it had one.
    pub async fn clear_goal(&self, session_id: AgentSessionId) -> Result<bool, AgentError> {
        self.feature(GOAL_PLUGIN, |goals: Arc<GoalService>| async move {
            goals.clear(session_id).await
        })
        .await
    }

    /// Pauses an active goal; any other goal is returned as it is. Refused
    /// when the session has no goal.
    pub async fn pause_goal(&self, session_id: AgentSessionId) -> Result<AgentGoal, AgentError> {
        self.feature(GOAL_PLUGIN, |goals: Arc<GoalService>| async move {
            goals.pause(session_id).await
        })
        .await
    }

    /// Makes the goal active again (budget-limited while a budget is spent).
    /// No turn starts: the session's next turn continues the goal. Refused
    /// when the session has no goal.
    pub async fn resume_goal(&self, session_id: AgentSessionId) -> Result<AgentGoal, AgentError> {
        self.feature(GOAL_PLUGIN, |goals: Arc<GoalService>| async move {
            goals.resume(session_id).await
        })
        .await
    }
}
