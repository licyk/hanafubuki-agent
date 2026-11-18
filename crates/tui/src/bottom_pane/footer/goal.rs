// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! Codex's goal indicator, driven by the Agent's goal snapshots and turn clock.

use std::time::Instant;

use hanafubuki_agent::{AgentGoal, AgentGoalStatus};
use ratatui::{style::Stylize, text::Line};

use crate::history_cell::format_tokens_compact;
use crate::status_indicator_widget::fmt_elapsed_compact;

#[derive(Debug, Default)]
pub(crate) struct GoalStatus {
    observed: Option<(AgentGoal, Instant)>,
    running_since: Option<Instant>,
}

impl GoalStatus {
    pub(crate) fn update(&mut self, goal: Option<&AgentGoal>, now: Instant) {
        if self.observed.as_ref().map(|(goal, _)| goal) != goal {
            self.observed = goal.cloned().map(|goal| (goal, now));
        }
    }

    pub(crate) fn set_running(&mut self, running: bool, now: Instant) {
        if running {
            self.running_since.get_or_insert(now);
        } else if let Some(started) = self.running_since.take() {
            if let Some((goal, observed)) = &mut self.observed {
                if goal.status == AgentGoalStatus::Active {
                    goal.time_used_seconds = goal.time_used_seconds.saturating_add(
                        now.saturating_duration_since(started.max(*observed))
                            .as_secs(),
                    );
                    *observed = now;
                }
            }
        }
    }

    pub(super) fn line(&self, now: Instant) -> Option<Line<'static>> {
        let (goal, observed) = self.observed.as_ref()?;
        let elapsed = if goal.status == AgentGoalStatus::Active {
            self.running_since.map_or(0, |started| {
                now.saturating_duration_since(started.max(*observed))
                    .as_secs()
            })
        } else {
            0
        };
        let time = fmt_elapsed_compact(goal.time_used_seconds.saturating_add(elapsed));
        let tokens = format_tokens_compact(goal.tokens_used);
        let usage = match (goal.token_budget, goal.time_budget_seconds) {
            (Some(budget), _) => format!("{tokens} / {} tokens", format_tokens_compact(budget)),
            (_, Some(budget)) => format!("{time} / {}", fmt_elapsed_compact(budget)),
            _ => time,
        };
        let text = match goal.status {
            AgentGoalStatus::Active => format!("Pursuing goal ({usage})"),
            AgentGoalStatus::Paused => "Goal paused (/goal resume)".into(),
            AgentGoalStatus::Blocked => "Goal stalled (/goal resume)".into(),
            AgentGoalStatus::BudgetLimited => format!("Goal unmet ({usage})"),
            AgentGoalStatus::Complete => format!("Goal achieved ({usage})"),
        };
        Some(Line::from(text.magenta()))
    }
}

#[cfg(test)]
mod tests;
