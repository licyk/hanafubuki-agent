//! A session's goal: an objective the session keeps working toward across
//! turns, with optional budgets and the usage spent on it. The kernel only
//! stores it with the session's runtime state (so a fork, an archive and a
//! deletion carry it); the `goal` plugin gives it its meaning.

use serde::{Deserialize, Deserializer, Serialize};

/// Where a goal stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentGoalStatus {
    /// Pursued: a turn that ends while the goal is active continues.
    Active,
    Paused,
    /// The model found itself at an impasse it cannot pass without the user.
    Blocked,
    /// A budget is spent.
    BudgetLimited,
    Complete,
}

/// Who set a goal.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentGoalOrigin {
    #[default]
    User,
    Model,
}

/// A session's goal as it is persisted.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentGoal {
    pub objective: String,
    pub status: AgentGoalStatus,
    #[serde(default)]
    pub origin: AgentGoalOrigin,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token_budget: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub time_budget_seconds: Option<u64>,
    /// Model tokens (input and output) spent while the goal was active.
    #[serde(default)]
    pub tokens_used: u64,
    /// Time spent in turns while the goal was active.
    #[serde(default)]
    pub time_used_seconds: u64,
    /// Milliseconds since the Unix epoch.
    #[serde(default)]
    pub created_at: u128,
    #[serde(default)]
    pub updated_at: u128,
}

impl AgentGoal {
    /// Whether a budget is spent.
    pub fn budget_exhausted(&self) -> bool {
        self.token_budget
            .is_some_and(|budget| self.tokens_used >= budget)
            || self
                .time_budget_seconds
                .is_some_and(|budget| self.time_used_seconds >= budget)
    }
}

/// What an update makes of a session's goal.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AgentGoalChange {
    Keep,
    Set(AgentGoal),
    Clear,
}

/// Reads a persisted goal tolerantly: one this build cannot read (an
/// unknown status, a missing objective) reads as none rather than failing
/// the record.
pub(crate) fn tolerant_goal<'de, D>(deserializer: D) -> Result<Option<AgentGoal>, D::Error>
where
    D: Deserializer<'de>,
{
    let value = Option::<serde_json::Value>::deserialize(deserializer)?;
    Ok(value.and_then(|value| match serde_json::from_value(value) {
        Ok(goal) => Some(goal),
        Err(error) => {
            log::warn!("Agent session goal ignored: it could not be read: {error}");
            None
        }
    }))
}

#[cfg(test)]
mod tests;
