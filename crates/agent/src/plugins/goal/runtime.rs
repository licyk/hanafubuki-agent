//! What the goal tools, the continuation hook and the facade's goal calls
//! share: the session goal through the application's port, and the usage
//! accounting of the goal's turns.
//!
//! Usage accrues while a goal is active: the session's model tokens (input
//! and output) and the time spent in turns, counted from one accounting
//! point to the next. A point is taken at each final answer of a goal turn,
//! at each change the model or the user makes, and at a turn's admission
//! (tokens only, which grow only in turns). The last point of each session
//! is kept in memory; a session without one (the plugin just started) starts
//! counting at its next point.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use hanafubuki_agent_contracts::BlockingService;
use hanafubuki_agent_kernel::{
    AgentError, AgentGoal, AgentGoalChange, AgentGoalOrigin, AgentGoalStatus, AgentSessionId,
    AgentSessionMode,
};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};

use super::templates;
use crate::goals::{AgentGoalApplication, AgentGoalSession};

/// The longest objective, in characters (Codex's bound).
pub const MAXIMUM_GOAL_OBJECTIVE_CHARS: usize = 4_000;

/// The budgets of a goal; each `None` is unbounded.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentGoalBudgets {
    pub token_budget: Option<u64>,
    pub time_budget_seconds: Option<u64>,
}

/// The goal statuses the model may set with `update_goal`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ModelGoalStatus {
    Complete,
    Blocked,
    Paused,
}

/// The last accounting point of a session.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct AccountingPoint {
    /// The session's model tokens then.
    pub(super) tokens: u64,
    /// Milliseconds since the Unix epoch.
    pub(super) at: u128,
}

#[derive(Default)]
struct SessionAccounting {
    point: Option<AccountingPoint>,
    /// The objective the model was last shown, by the turn's context or a
    /// continuation.
    seen_objective: Option<String>,
}

pub(super) struct GoalRuntime {
    goals: AgentGoalApplication,
    blocking: Arc<BlockingService>,
    sessions: Mutex<HashMap<AgentSessionId, SessionAccounting>>,
}

impl GoalRuntime {
    pub(super) fn new(goals: AgentGoalApplication, blocking: Arc<BlockingService>) -> Self {
        Self {
            goals,
            blocking,
            sessions: Mutex::new(HashMap::new()),
        }
    }

    /// Whether the session's goal is active, read under the session's lock
    /// alone.
    pub(super) fn active(&self, session_id: AgentSessionId) -> bool {
        self.goals
            .goal(session_id)
            .ok()
            .flatten()
            .is_some_and(|goal| goal.status == AgentGoalStatus::Active)
    }

    pub(super) async fn goal(
        &self,
        session_id: AgentSessionId,
    ) -> Result<Option<AgentGoal>, AgentError> {
        let goals = self.goals.clone();
        self.run(move || goals.goal(session_id)).await
    }

    /// A new active goal, replacing the session's. The model may only
    /// create one when the session has none or its goal is complete.
    pub(super) async fn create(
        &self,
        session_id: AgentSessionId,
        objective: &str,
        budgets: AgentGoalBudgets,
        origin: AgentGoalOrigin,
    ) -> Result<AgentGoal, AgentError> {
        let objective = validated_objective(objective)?;
        validate_budgets(budgets)?;
        let session = self.session(session_id).await?;
        let now = now_millis();
        let goal = AgentGoal {
            objective,
            status: AgentGoalStatus::Active,
            origin,
            token_budget: budgets.token_budget,
            time_budget_seconds: budgets.time_budget_seconds,
            tokens_used: 0,
            time_used_seconds: 0,
            created_at: now,
            updated_at: now,
        };
        let created = goal.clone();
        let goals = self.goals.clone();
        self.run(move || {
            goals.update(session_id, move |current| {
                if origin == AgentGoalOrigin::Model
                    && current.is_some_and(|goal| goal.status != AgentGoalStatus::Complete)
                {
                    return Err(AgentError::InvalidInput(
                        "cannot create a new goal because this session has an unfinished goal; complete the existing goal first".to_string(),
                    ));
                }
                Ok(AgentGoalChange::Set(created))
            })
        })
        .await?;
        self.restart_accounting(session_id, &session, now);
        Ok(goal)
    }

    /// The model's status change: complete, blocked or paused. A spent
    /// budget outranks paused and blocked.
    pub(super) async fn update_by_model(
        &self,
        session_id: AgentSessionId,
        status: ModelGoalStatus,
    ) -> Result<AgentGoal, AgentError> {
        let status = match status {
            ModelGoalStatus::Complete => AgentGoalStatus::Complete,
            ModelGoalStatus::Blocked => AgentGoalStatus::Blocked,
            ModelGoalStatus::Paused => AgentGoalStatus::Paused,
        };
        self.change_status(session_id, move |goal| {
            if goal.status == AgentGoalStatus::BudgetLimited && status != AgentGoalStatus::Complete
            {
                goal.status
            } else {
                status
            }
        })
        .await
    }

    /// Pauses an active goal; a goal that is not active stays as it is.
    pub(super) async fn pause(&self, session_id: AgentSessionId) -> Result<AgentGoal, AgentError> {
        self.change_status(session_id, |goal| match goal.status {
            AgentGoalStatus::Active => AgentGoalStatus::Paused,
            status => status,
        })
        .await
    }

    /// Makes the goal active again, or budget-limited while a budget is
    /// spent.
    pub(super) async fn resume(&self, session_id: AgentSessionId) -> Result<AgentGoal, AgentError> {
        let goal = self
            .change_status(session_id, |goal| {
                if goal.budget_exhausted() {
                    AgentGoalStatus::BudgetLimited
                } else {
                    AgentGoalStatus::Active
                }
            })
            .await?;
        if goal.status == AgentGoalStatus::Active {
            let session = self.session(session_id).await?;
            self.restart_accounting(session_id, &session, now_millis());
        }
        Ok(goal)
    }

    /// Removes the session's goal; whether it had one.
    pub(super) async fn clear(&self, session_id: AgentSessionId) -> Result<bool, AgentError> {
        let goals = self.goals.clone();
        let (before, _) = self
            .run(move || goals.update(session_id, |_| Ok(AgentGoalChange::Clear)))
            .await?;
        self.sessions.lock().remove(&session_id);
        Ok(before.is_some())
    }

    /// At a turn's admission: the session's goal for the turn's context.
    /// The tokens spent since the last point (a turn that ended without a
    /// final answer) are counted; the time between turns is not.
    pub(super) async fn admitted(
        &self,
        session_id: AgentSessionId,
    ) -> Result<Option<AgentGoal>, AgentError> {
        let Some(goal) = self.goal(session_id).await? else {
            self.sessions.lock().remove(&session_id);
            return Ok(None);
        };
        if goal.status != AgentGoalStatus::Active {
            let mut sessions = self.sessions.lock();
            let accounting = sessions.entry(session_id).or_default();
            accounting.point = None;
            accounting.seen_objective = Some(goal.objective.clone());
            return Ok(Some(goal));
        }
        let session = self.session(session_id).await?;
        let now = now_millis();
        let point = self.point(session_id);
        let (tokens, _, next) = accrual(&session, point, now);
        let goal = if tokens > 0 {
            self.accrue(session_id, tokens, 0, now)
                .await?
                .1
                .unwrap_or(goal)
        } else {
            goal
        };
        let mut sessions = self.sessions.lock();
        let accounting = sessions.entry(session_id).or_default();
        accounting.point = Some(AccountingPoint { at: now, ..next });
        accounting.seen_objective = Some(goal.objective.clone());
        Ok(Some(goal))
    }

    /// At a final answer: accounts the goal's usage, and returns what keeps
    /// an active goal going: the budget-limit notice when this spent a
    /// budget, the objective-updated notice when the user replaced the
    /// objective the model last saw, and the continuation otherwise. `None`
    /// ends the turn.
    pub(super) async fn continuation(
        &self,
        session_id: AgentSessionId,
    ) -> Result<Option<String>, AgentError> {
        let session = self.session(session_id).await?;
        if session.mode == AgentSessionMode::Plan {
            return Ok(None);
        }
        let now = now_millis();
        let (tokens, seconds, next) = accrual(&session, self.point(session_id), now);
        let (before, after) = self.accrue(session_id, tokens, seconds, now).await?;
        let (Some(before), Some(goal)) = (before, after) else {
            return Ok(None);
        };
        if before.status != AgentGoalStatus::Active {
            return Ok(None);
        }
        let seen = {
            let mut sessions = self.sessions.lock();
            let accounting = sessions.entry(session_id).or_default();
            accounting.point = (goal.status == AgentGoalStatus::Active).then_some(next);
            accounting.seen_objective.replace(goal.objective.clone())
        };
        Ok(match goal.status {
            AgentGoalStatus::BudgetLimited => Some(templates::budget_limit(&goal)),
            AgentGoalStatus::Active
                if goal.origin == AgentGoalOrigin::User
                    && seen.is_some_and(|seen| seen != goal.objective) =>
            {
                Some(templates::objective_updated(&goal))
            }
            AgentGoalStatus::Active => Some(templates::continuation(&goal)),
            _ => None,
        })
    }

    /// Whether the session lives in memory only, where goals are refused.
    pub(super) async fn ensure_persistent(
        &self,
        session_id: AgentSessionId,
    ) -> Result<(), AgentError> {
        if self.session(session_id).await?.ephemeral {
            return Err(AgentError::InvalidInput(
                "goals need a saved session; this session is temporary".to_string(),
            ));
        }
        Ok(())
    }

    /// Accounts the usage up to now, then sets the status `status` makes of
    /// the goal; refused when the session has no goal.
    async fn change_status(
        &self,
        session_id: AgentSessionId,
        status: impl FnOnce(&AgentGoal) -> AgentGoalStatus + Send + 'static,
    ) -> Result<AgentGoal, AgentError> {
        let session = self.session(session_id).await?;
        let now = now_millis();
        let (tokens, seconds, next) = accrual(&session, self.point(session_id), now);
        let goals = self.goals.clone();
        let (before, after) = self
            .run(move || {
                goals.update(session_id, move |current| {
                    let Some(current) = current else {
                        return Err(AgentError::InvalidInput(
                            "cannot update the goal because this session has no goal".to_string(),
                        ));
                    };
                    let mut goal = current.clone();
                    if goal.status == AgentGoalStatus::Active {
                        add_usage(&mut goal, tokens, seconds);
                    }
                    goal.status = status(&goal);
                    if goal == *current {
                        return Ok(AgentGoalChange::Keep);
                    }
                    goal.updated_at = now;
                    Ok(AgentGoalChange::Set(goal))
                })
            })
            .await?;
        let goal = after.ok_or_else(|| AgentError::Internal("the goal vanished".to_string()))?;
        let was_active = before.is_some_and(|goal| goal.status == AgentGoalStatus::Active);
        let mut sessions = self.sessions.lock();
        let accounting = sessions.entry(session_id).or_default();
        accounting.point = match goal.status {
            AgentGoalStatus::Active if was_active => Some(next),
            AgentGoalStatus::Active => accounting.point,
            _ => None,
        };
        Ok(goal)
    }

    /// Adds `tokens` and `seconds` to an active goal, which becomes
    /// budget-limited once a budget is spent.
    async fn accrue(
        &self,
        session_id: AgentSessionId,
        tokens: u64,
        seconds: u64,
        now: u128,
    ) -> Result<(Option<AgentGoal>, Option<AgentGoal>), AgentError> {
        let goals = self.goals.clone();
        self.run(move || {
            goals.update(session_id, move |current| {
                let Some(current) = current.filter(|goal| goal.status == AgentGoalStatus::Active)
                else {
                    return Ok(AgentGoalChange::Keep);
                };
                let mut goal = current.clone();
                add_usage(&mut goal, tokens, seconds);
                if goal.budget_exhausted() {
                    goal.status = AgentGoalStatus::BudgetLimited;
                }
                if goal == *current {
                    return Ok(AgentGoalChange::Keep);
                }
                goal.updated_at = now;
                Ok(AgentGoalChange::Set(goal))
            })
        })
        .await
    }

    fn restart_accounting(
        &self,
        session_id: AgentSessionId,
        session: &AgentGoalSession,
        now: u128,
    ) {
        self.sessions.lock().entry(session_id).or_default().point = Some(AccountingPoint {
            tokens: session.tokens,
            at: now,
        });
    }

    fn point(&self, session_id: AgentSessionId) -> Option<AccountingPoint> {
        self.sessions
            .lock()
            .get(&session_id)
            .and_then(|accounting| accounting.point)
    }

    async fn session(&self, session_id: AgentSessionId) -> Result<AgentGoalSession, AgentError> {
        let goals = self.goals.clone();
        self.run(move || goals.session(session_id)).await
    }

    async fn run<T: Send + 'static>(
        &self,
        work: impl FnOnce() -> Result<T, AgentError> + Send + 'static,
    ) -> Result<T, AgentError> {
        self.blocking
            .run("agent goal", work)
            .await
            .map_err(|error| AgentError::Internal(error.to_string()))?
    }
}

/// The tokens and whole seconds spent since `point`, and the next point.
/// Time counts only within the running turn; the part of a second not
/// counted is carried to the next point.
pub(super) fn accrual(
    session: &AgentGoalSession,
    point: Option<AccountingPoint>,
    now: u128,
) -> (u64, u64, AccountingPoint) {
    let Some(point) = point else {
        return (
            0,
            0,
            AccountingPoint {
                tokens: session.tokens,
                at: now,
            },
        );
    };
    let tokens = session.tokens.saturating_sub(point.tokens);
    let elapsed = session
        .turn_started_at
        .map_or(0, |started| now.saturating_sub(point.at.max(started)));
    let seconds = u64::try_from(elapsed / 1000).unwrap_or(u64::MAX);
    (
        tokens,
        seconds,
        AccountingPoint {
            tokens: session.tokens,
            at: now - elapsed % 1000,
        },
    )
}

fn add_usage(goal: &mut AgentGoal, tokens: u64, seconds: u64) {
    goal.tokens_used = goal.tokens_used.saturating_add(tokens);
    goal.time_used_seconds = goal.time_used_seconds.saturating_add(seconds);
}

/// The objective trimmed; refused when empty or over the bound.
pub(super) fn validated_objective(objective: &str) -> Result<String, AgentError> {
    let objective = objective.trim();
    if objective.is_empty() {
        return Err(AgentError::InvalidInput(
            "goal objective must not be empty".to_string(),
        ));
    }
    if objective.chars().count() > MAXIMUM_GOAL_OBJECTIVE_CHARS {
        return Err(AgentError::InvalidInput(format!(
            "goal objective must be at most {MAXIMUM_GOAL_OBJECTIVE_CHARS} characters; put longer instructions in a file and name it in the objective"
        )));
    }
    Ok(objective.to_string())
}

fn validate_budgets(budgets: AgentGoalBudgets) -> Result<(), AgentError> {
    if budgets.token_budget == Some(0) || budgets.time_budget_seconds == Some(0) {
        return Err(AgentError::InvalidInput(
            "goal budgets must be positive when provided".to_string(),
        ));
    }
    Ok(())
}

fn now_millis() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_millis())
}
