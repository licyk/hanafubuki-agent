//! Goal receipts shown from committed output, including replayed calls.

use hanafubuki_agent_contracts::{structured_output, AgentToolDisplay, AgentToolPresenter};
use hanafubuki_agent_tool_contracts::ToolOutput;
use serde::Deserialize;
use serde_json::Value;

pub(super) struct GoalPresenter;

impl AgentToolPresenter for GoalPresenter {
    fn tools(&self) -> &[&'static str] {
        &["create_goal", "get_goal", "update_goal"]
    }

    fn subject(&self, tool: &str, arguments: &Value) -> Option<String> {
        let key = match tool {
            "create_goal" => "objective",
            "update_goal" => "status",
            _ => return None,
        };
        arguments.get(key)?.as_str().map(str::to_string)
    }

    fn display(
        &self,
        _tool: &str,
        _arguments: &Value,
        output: &ToolOutput,
    ) -> Option<AgentToolDisplay> {
        if output.is_error {
            return None;
        }
        let goal = structured_output(output)?.get("goal")?;
        if goal.is_null() {
            return Some(AgentToolDisplay::Text {
                text: "No goal set.".into(),
            });
        }
        let goal: GoalSummary = serde_json::from_value(goal.clone()).ok()?;
        let mut lines = vec![
            format!("Status: {}", goal.status.replace('_', " ")),
            format!("Goal: {}", goal.objective),
        ];
        for (label, used, budget, unit) in [
            ("Tokens", goal.tokens_used, goal.token_budget, ""),
            (
                "Time",
                goal.time_used_seconds,
                goal.time_budget_seconds,
                "s",
            ),
        ] {
            if let Some(used) = used {
                let line = match budget {
                    Some(budget) => format!(
                        "{label}: {used}{unit} / {budget}{unit} · {}{unit} remaining",
                        budget.saturating_sub(used)
                    ),
                    None => format!("{label}: {used}{unit} used"),
                };
                lines.push(line);
            }
        }
        Some(AgentToolDisplay::Text {
            text: lines.join("\n"),
        })
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct GoalSummary {
    objective: String,
    status: String,
    tokens_used: Option<u64>,
    token_budget: Option<u64>,
    time_used_seconds: Option<u64>,
    time_budget_seconds: Option<u64>,
}

#[cfg(test)]
mod tests;
