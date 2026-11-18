use hanafubuki_agent_contracts::{structured_output, AgentToolDisplay, AgentToolPresenter};
use hanafubuki_agent_tool_contracts::ToolOutput;
use serde_json::Value;

pub(super) struct AdvisorPresenter;

impl AgentToolPresenter for AdvisorPresenter {
    fn tools(&self) -> &[&'static str] {
        &["ask_advisor", "record_advisor_outcome", "advisor_config"]
    }

    fn subject(&self, tool: &str, args: &Value) -> Option<String> {
        match tool {
            "ask_advisor" if args["followUpTo"].is_string() => Some("Advisor (follow-up)".into()),
            "ask_advisor" => Some("Advisor".into()),
            "advisor_config" if args["changes"].as_object().is_some_and(|v| !v.is_empty()) => {
                Some("settings proposal".into())
            }
            "advisor_config" => Some("settings".into()),
            "record_advisor_outcome" => args["adviceId"].as_str().map(str::to_owned),
            _ => None,
        }
    }

    fn display(&self, tool: &str, args: &Value, output: &ToolOutput) -> Option<AgentToolDisplay> {
        if output.is_error {
            return None;
        }
        let value = structured_output(output)?;
        let mut lines = Vec::new();
        match tool {
            "ask_advisor" => {
                lines.push(format!(
                    "Model: {}/{}",
                    value["provider"].as_str()?,
                    value["model"].as_str()?
                ));
                if let Some(question) = args["question"].as_str().filter(|q| !q.is_empty()) {
                    lines.push(format!("Question: {question}"));
                }
                lines.push(format!("Advice: {}", value["text"].as_str()?));
                if let Some(previous) = args["followUpTo"].as_str() {
                    lines.push(format!("Follow-up to: {previous}"));
                }
                if let Some(id) = value["adviceId"].as_str() {
                    lines.push(format!("Advice ID: {id}"));
                }
            }
            "record_advisor_outcome" if value["recorded"] == true => {
                lines.push(format!("Adoption: {}", args["adoption"].as_str()?));
                lines.push(format!(
                    "Validation: {}",
                    args["validationStatus"].as_str()?
                ));
            }
            "advisor_config" if value.get("pendingUserConfirmation").is_some() => {
                let proposal = &value["pendingUserConfirmation"];
                lines.push("Awaiting user confirmation".into());
                for change in proposal["changes"].as_array()? {
                    lines.push(format!(
                        "{}: {} → {}",
                        change["key"].as_str()?,
                        change.get("before")?,
                        change.get("after")?
                    ));
                }
                if let Some(id) = proposal["id"].as_str() {
                    lines.push(format!("Proposal ID: {id}"));
                }
                if let Some(review) = value["review"].as_str() {
                    lines.push(review.to_owned());
                }
            }
            "advisor_config" => {
                for (key, setting) in value.as_object()? {
                    lines.push(format!("{key}: {setting}"));
                }
            }
            _ => return None,
        }
        Some(AgentToolDisplay::Text {
            text: lines.join("\n"),
        })
    }
}

#[cfg(test)]
mod tests;
