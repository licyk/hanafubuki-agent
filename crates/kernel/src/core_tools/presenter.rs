//! How an interface shows the kernel's own tools: `compress` names the
//! ranges it folds, and shows its receipt then each range's summary. The
//! task checklist uses a typed display reconstructed from the successful call.

use hanafubuki_agent_contracts::{
    output_text, structured_output, AgentToolDisplay, AgentToolPresenter,
};
use hanafubuki_agent_tool_contracts::ToolOutput;
use serde_json::Value;

use crate::model::{compress_call_ranges, AgentCompressRange};

pub(crate) struct CoreToolPresenter;

impl AgentToolPresenter for CoreToolPresenter {
    fn tools(&self) -> &[&'static str] {
        &[
            hanafubuki_agent_compression::COMPRESS_TOOL_NAME,
            "update_plan",
            "request_user_input",
        ]
    }

    /// `m00002–m00005 Setup, m00009–m00012`: the ranges with their topics.
    fn subject(&self, tool: &str, arguments: &Value) -> Option<String> {
        if tool == "update_plan" {
            return None;
        }
        if tool == "request_user_input" {
            let call: super::RequestUserInputTool =
                serde_json::from_value(arguments.clone()).ok()?;
            return Some(
                call.questions
                    .iter()
                    .map(|q| q.header.as_str())
                    .collect::<Vec<_>>()
                    .join(", "),
            );
        }
        let ranges = compress_call_ranges(arguments);
        (!ranges.is_empty()).then(|| {
            ranges
                .iter()
                .map(range_label)
                .collect::<Vec<_>>()
                .join(", ")
        })
    }

    /// The receipt's lines, then each range under its label.
    fn display(
        &self,
        tool: &str,
        arguments: &Value,
        output: &ToolOutput,
    ) -> Option<AgentToolDisplay> {
        if tool == "request_user_input" {
            return question_display(arguments, output);
        }
        if tool == "update_plan" {
            if output.is_error {
                return None;
            }
            let call: crate::core_tools::UpdateTaskPlanTool =
                serde_json::from_value(arguments.clone()).ok()?;
            return Some(AgentToolDisplay::TaskPlan {
                explanation: call.explanation,
                plan: call.plan,
            });
        }
        let mut text = output_text(output).trim_end().to_string();
        for range in compress_call_ranges(arguments) {
            text.push('\n');
            text.push_str(&range_label(&range));
            text.push('\n');
            text.push_str(range.summary.trim_end());
        }
        Some(AgentToolDisplay::Text { text })
    }
}

fn question_display(arguments: &Value, output: &ToolOutput) -> Option<AgentToolDisplay> {
    if output.is_error {
        return None;
    }
    let call: super::RequestUserInputTool = serde_json::from_value(arguments.clone()).ok()?;
    let answers: Vec<crate::model::AgentUserInputAnswer> =
        serde_json::from_value(structured_output(output)?.get("answers")?.clone()).ok()?;
    let lines = call
        .questions
        .iter()
        .map(|question| {
            let answer = answers
                .iter()
                .find(|answer| answer.question_id == question.id)?;
            let label = question
                .options
                .iter()
                .find(|option| option.id == answer.option_id)
                .map_or(answer.option_id.as_str(), |option| option.label.as_str());
            Some(format!("{} → {}", question.question, label))
        })
        .collect::<Option<Vec<_>>>()?;
    Some(AgentToolDisplay::Text {
        text: lines.join("\n"),
    })
}

#[cfg(test)]
mod tests;

fn range_label(range: &AgentCompressRange) -> String {
    match &range.topic {
        Some(topic) => format!("{}–{} {topic}", range.start_ref, range.end_ref),
        None => format!("{}–{}", range.start_ref, range.end_ref),
    }
}
