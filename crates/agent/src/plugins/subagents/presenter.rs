//! Human-readable collaboration results; the model's output is unchanged.
use hanafubuki_agent_contracts::{structured_output, AgentToolDisplay, AgentToolPresenter};
use hanafubuki_agent_tool_contracts::{ToolOutput, ToolOutputContent};
use serde::Deserialize;
use serde_json::Value;

pub(super) struct SubagentPresenter;

impl AgentToolPresenter for SubagentPresenter {
    fn tools(&self) -> &[&'static str] {
        &[
            "spawn_agent",
            "send_message",
            "followup_task",
            "interrupt_agent",
            "wait_agent",
            "list_agents",
        ]
    }

    fn subject(&self, tool: &str, arguments: &Value) -> Option<String> {
        arguments
            .get(if tool == "spawn_agent" {
                "task_name"
            } else {
                "target"
            })?
            .as_str()
            .map(str::to_string)
    }

    fn display(
        &self,
        tool: &str,
        arguments: &Value,
        output: &ToolOutput,
    ) -> Option<AgentToolDisplay> {
        if output.is_error {
            return None;
        }
        let value = structured_output(output).cloned().or_else(|| {
            output.content.iter().find_map(|content| match content {
                ToolOutputContent::Text { text } => serde_json::from_str(text).ok(),
                _ => None,
            })
        })?;
        let lines = match tool {
            "spawn_agent" => spawn_lines(arguments, &value)?,
            "send_message" | "followup_task" => vec![
                format!("Delivery: {}", value.get("delivery")?.as_str()?),
                format!(
                    "{}: {}",
                    if tool == "send_message" {
                        "Message"
                    } else {
                        "Task"
                    },
                    arguments.get("message")?.as_str()?
                ),
            ],
            "interrupt_agent" => {
                let (previous, _) = status_label(value.get("previous_status")?);
                vec![format!("Previous status: {previous}")]
            }
            "wait_agent" | "list_agents" => agent_lines(&value),
            _ => return None,
        };
        Some(AgentToolDisplay::Text {
            text: lines.join("\n"),
        })
    }
}

#[derive(Deserialize)]
struct AgentSummary {
    agent_name: Option<String>,
    task_name: Option<String>,
    agent_id: Option<String>,
    model: Option<String>,
    provider: Option<String>,
    reasoning_effort: Option<String>,
    agent_type: Option<String>,
    agent_status: Option<Value>,
}

impl AgentSummary {
    fn name(&self) -> String {
        let name = self
            .agent_name
            .as_deref()
            .or(self.task_name.as_deref())
            .unwrap_or("Agent");
        match &self.agent_id {
            Some(id) => format!("{name} (agent {id})"),
            None => name.to_string(),
        }
    }

    fn model(&self) -> String {
        format!(
            "Model: {} · Reasoning: {}",
            self.model.as_deref().unwrap_or("not recorded"),
            self.reasoning_effort.as_deref().unwrap_or("unspecified")
        )
    }
}

fn spawn_lines(arguments: &Value, value: &Value) -> Option<Vec<String>> {
    let agent: AgentSummary = serde_json::from_value(value.clone()).ok()?;
    let mut lines = vec![
        format!("Agent: {}", agent.name()),
        agent.model(),
        format!("Task: {}", arguments.get("message")?.as_str()?),
    ];
    if let Some(provider) = agent.provider {
        lines.push(format!("Provider: {provider}"));
    }
    if let Some(role) = arguments.get("agent_type").and_then(Value::as_str) {
        lines.push(format!("Role: {role}"));
    }
    if let Some(fork) = arguments.get("fork_turns").and_then(Value::as_str) {
        lines.push(format!("Conversation context: {fork}"));
    }
    Some(lines)
}

fn agent_lines(value: &Value) -> Vec<String> {
    let mut lines = Vec::new();
    if let Some(message) = value.get("message").and_then(Value::as_str) {
        lines.push(message.to_string());
    }
    let Some(agents) = value.get("agents").and_then(Value::as_array) else {
        lines.push("Warning: agent results could not be read".into());
        return lines;
    };
    if agents.is_empty() && lines.is_empty() {
        lines.push("No subagents.".into());
    }
    for value in agents {
        let Ok(agent) = serde_json::from_value::<AgentSummary>(value.clone()) else {
            lines.push("Warning: agent result could not be read".into());
            continue;
        };
        let (label, detail) = agent
            .agent_status
            .as_ref()
            .map(status_label)
            .unwrap_or(("Warning: missing agent status", None));
        lines.push(format!("{} · {label}", agent.name()));
        if agent.model.is_some() {
            lines.push(agent.model());
        }
        if let Some(provider) = agent.provider {
            lines.push(format!("Provider: {provider}"));
        }
        if let Some(role) = agent.agent_type {
            lines.push(format!("Role: {role}"));
        }
        if let Some(detail) = detail {
            lines.push(detail.to_string());
        }
    }
    lines
}

fn status_label(status: &Value) -> (&str, Option<&str>) {
    if let Some(answer) = status.get("completed").and_then(Value::as_str) {
        return ("Completed", Some(answer));
    }
    if let Some(error) = status.get("errored").and_then(Value::as_str) {
        return ("Failed", Some(error));
    }
    let label = match status.as_str().unwrap_or_default() {
        "completed" => "Completed",
        "running" => "Running",
        "waiting" => "Waiting",
        "interrupted" => "Interrupted",
        "closed" => "Closed",
        "errored" => "Failed",
        "pending_init" => "Starting",
        _ => "Warning: unrecognized status",
    };
    (label, None)
}

#[cfg(test)]
mod tests;
