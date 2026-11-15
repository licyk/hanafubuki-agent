//! Human-readable collaboration results; the model's output is unchanged.
use hanafubuki_agent_contracts::{AgentToolDisplay, AgentToolPresenter};
use hanafubuki_agent_tool_contracts::{ToolOutput, ToolOutputContent};
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
        _arguments: &Value,
        output: &ToolOutput,
    ) -> Option<AgentToolDisplay> {
        if !matches!(tool, "wait_agent" | "list_agents") {
            return None;
        }
        let value = output.content.iter().find_map(|content| match content {
            ToolOutputContent::Structured { value } => Some(value.clone()),
            ToolOutputContent::Text { text } => serde_json::from_str(text).ok(),
            _ => None,
        })?;
        let mut lines = Vec::new();
        if let Some(message) = value.get("message").and_then(Value::as_str) {
            lines.push(message.to_string());
        }
        let agents = value.get("agents").and_then(Value::as_array);
        if agents.is_none() {
            lines.push("Warning: agent results could not be read".into());
        }
        for agent in agents.into_iter().flatten() {
            let name = agent
                .get("agent_name")
                .and_then(Value::as_str)
                .unwrap_or("Agent");
            let Some(status) = agent.get("agent_status") else {
                lines.push(format!("{name} · Warning: missing agent status"));
                continue;
            };
            let (label, detail) =
                if let Some(answer) = status.get("completed").and_then(Value::as_str) {
                    ("Completed", Some(answer))
                } else if let Some(error) = status.get("errored").and_then(Value::as_str) {
                    ("Failed", Some(error))
                } else {
                    (
                        match status.as_str().unwrap_or_default() {
                            "completed" => "Completed",
                            "running" => "Running",
                            "waiting" => "Waiting",
                            "interrupted" => "Interrupted",
                            "closed" => "Closed",
                            "errored" => "Failed",
                            "pending_init" => "Starting",
                            _ => "Warning: unrecognized status",
                        },
                        None,
                    )
                };
            lines.push(format!("{name} · {label}"));
            if let Some(detail) = detail {
                lines.push(detail.to_string());
            }
        }
        Some(AgentToolDisplay::Text {
            text: lines.join("\n"),
        })
    }
}

#[cfg(test)]
mod tests;
