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
            "request_user_input_async",
        ]
    }

    /// `m00002–m00005 Setup, m00009–m00012`: the ranges with their topics.
    fn subject(&self, tool: &str, arguments: &Value) -> Option<String> {
        if tool == "request_user_input_async" {
            return async_questions(arguments).next().map(|q| q.title);
        }
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
        if tool == "request_user_input_async" {
            if output.is_error {
                return None;
            }
            let lines = async_questions(arguments)
                .flat_map(|question| {
                    std::iter::once(question.title).chain(
                        question
                            .options
                            .into_iter()
                            .flatten()
                            .map(|option| format!("  • {option}")),
                    )
                })
                .collect::<Vec<_>>();
            return Some(AgentToolDisplay::Text {
                text: lines.join("\n"),
            });
        }
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

fn async_questions(arguments: &Value) -> impl Iterator<Item = crate::AgentAsyncQuestion> + '_ {
    arguments
        .get("questions")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|question| serde_json::from_value(question.clone()).ok())
}

fn question_display(arguments: &Value, output: &ToolOutput) -> Option<AgentToolDisplay> {
    if output.is_error {
        return None;
    }
    let call: super::RequestUserInputTool = serde_json::from_value(arguments.clone()).ok()?;
    let answers = structured_output(output)?
        .get("answers")?
        .as_array()?
        .clone();
    let lines = call
        .questions
        .iter()
        .map(|question| {
            let answer = answers.iter().find(|answer| {
                answer.get("questionId").and_then(Value::as_str) == Some(question.id.as_str())
            })?;
            let label = answer
                .get("label")
                .and_then(Value::as_str)
                .map(str::to_string)
                .or_else(|| {
                    answer
                        .get("text")
                        .and_then(Value::as_str)
                        .map(str::to_string)
                })
                .or_else(|| {
                    let option_id = answer.get("optionId").and_then(Value::as_str)?;
                    Some(
                        question
                            .options
                            .iter()
                            .find(|option| option.id == option_id)
                            .map_or_else(|| option_id.to_string(), |option| option.label.clone()),
                    )
                })
                .or_else(|| {
                    answer
                        .get("unanswered")
                        .and_then(Value::as_bool)
                        .filter(|unanswered| *unanswered)
                        .map(|_| "unanswered".to_string())
                })?;
            Some(format!("{} → {label}", question.question))
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
