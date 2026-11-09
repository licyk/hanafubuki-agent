//! `/goal` onto the facade's goal calls: read, set (after asking when an
//! unfinished goal would be replaced), clear, pause and resume the
//! session's goal. Every reply carries the goal as it is now, which the
//! status line takes (the session's events carry it only during a turn).

use hanafubuki_agent::{
    AgentError, AgentGoal, AgentGoalBudgets, AgentGoalStatus, AgentHandle, AgentSessionId,
};

use super::{AgentBridge, AgentReply, TurnUpdate};
use crate::app_event::AppEvent;

/// What `/goal` asks of the Agent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum GoalCall {
    /// The goal for the summary menu.
    Menu,
    /// The goal for the prompt that edits its objective.
    Edit,
    /// The goal for the prompt that sets its token budget.
    Budget,
    /// A new goal; unless `confirmed`, one that would replace an
    /// unfinished goal is asked about first.
    Set {
        objective: String,
        budgets: AgentGoalBudgets,
        confirmed: bool,
    },
    Clear,
    Pause,
    Resume,
}

/// What the Agent answered `/goal`.
#[derive(Debug)]
pub(crate) enum GoalReply {
    Menu(Option<AgentGoal>),
    Edit(AgentGoal),
    Budget(AgentGoal),
    /// `objective` would replace `current`, which is not complete.
    ConfirmReplace {
        current: AgentGoal,
        objective: String,
        budgets: AgentGoalBudgets,
    },
    Set(AgentGoal),
    /// Whether there was a goal to clear.
    Cleared(bool),
    Paused(AgentGoal),
    Resumed(AgentGoal),
    Failed(String),
}

impl GoalReply {
    /// The session's goal as the reply knows it, if it knows.
    pub(crate) fn goal(&self) -> Option<Option<&AgentGoal>> {
        match self {
            Self::Menu(goal) => Some(goal.as_ref()),
            Self::Edit(goal)
            | Self::Budget(goal)
            | Self::ConfirmReplace { current: goal, .. }
            | Self::Set(goal)
            | Self::Paused(goal)
            | Self::Resumed(goal) => Some(Some(goal)),
            Self::Cleared(_) => Some(None),
            Self::Failed(_) => None,
        }
    }
}

impl AgentBridge {
    pub(super) fn goal_call(&mut self, call: GoalCall) -> Option<TurnUpdate> {
        let Some(session_id) = self.session_id else {
            return Some(TurnUpdate::Goal(GoalReply::Failed(
                "there is no session yet".to_string(),
            )));
        };
        self.spawn(move |agent| async move {
            let reply = run(&agent, session_id, call)
                .await
                .unwrap_or_else(GoalReply::Failed);
            Ok(Some(AgentReply::Update(Box::new(TurnUpdate::Goal(reply)))))
        });
        None
    }

    /// What a goal call found, for `/goal`; the status line takes the goal
    /// it reports.
    pub(super) fn goal_reply(&mut self, reply: GoalReply) -> Option<TurnUpdate> {
        if let Some(status) = reply
            .goal()
            .and_then(|goal| self.goal_updated(goal.cloned()))
        {
            self.app_event_tx
                .send(AppEvent::AgentReply(AgentReply::Update(Box::new(status))));
        }
        Some(TurnUpdate::Goal(reply))
    }
}

async fn run(
    agent: &AgentHandle,
    session_id: AgentSessionId,
    call: GoalCall,
) -> Result<GoalReply, String> {
    let reply = match call {
        GoalCall::Menu => GoalReply::Menu(agent.goal(session_id).await.map_err(failed)?),
        GoalCall::Edit | GoalCall::Budget => {
            let goal = agent
                .goal(session_id)
                .await
                .map_err(failed)?
                .ok_or("no goal is set; /goal OBJECTIVE sets one")?;
            if call == GoalCall::Edit {
                GoalReply::Edit(goal)
            } else {
                GoalReply::Budget(goal)
            }
        }
        GoalCall::Set {
            objective,
            budgets,
            confirmed,
        } => {
            let current = if confirmed {
                None
            } else {
                agent.goal(session_id).await.map_err(failed)?
            };
            match current {
                Some(current) if current.status != AgentGoalStatus::Complete => {
                    GoalReply::ConfirmReplace {
                        current,
                        objective,
                        budgets,
                    }
                }
                _ => GoalReply::Set(
                    agent
                        .set_goal(session_id, objective, budgets)
                        .await
                        .map_err(failed)?,
                ),
            }
        }
        GoalCall::Clear => GoalReply::Cleared(agent.clear_goal(session_id).await.map_err(failed)?),
        GoalCall::Pause => GoalReply::Paused(agent.pause_goal(session_id).await.map_err(failed)?),
        GoalCall::Resume => {
            GoalReply::Resumed(agent.resume_goal(session_id).await.map_err(failed)?)
        }
    };
    Ok(reply)
}

fn failed(error: AgentError) -> String {
    match error {
        AgentError::FeatureUnavailable { reason, .. } => {
            format!("goals are unavailable: {reason}")
        }
        error => error.to_string(),
    }
}
