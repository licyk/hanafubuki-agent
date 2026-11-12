//! How an interface shows the kernel's own tools: `compress` names the
//! ranges it folds, and shows its receipt then each range's summary. The
//! others take the fallback.

use hanafubuki_agent_contracts::{output_text, AgentToolDisplay, AgentToolPresenter};
use hanafubuki_agent_tool_contracts::ToolOutput;
use serde_json::Value;

use crate::model::{compress_call_ranges, AgentCompressRange};

pub(crate) struct CoreToolPresenter;

impl AgentToolPresenter for CoreToolPresenter {
    fn tools(&self) -> &[&'static str] {
        &[hanafubuki_agent_compression::COMPRESS_TOOL_NAME]
    }

    /// `m00002–m00005 Setup, m00009–m00012`: the ranges with their topics.
    fn subject(&self, _tool: &str, arguments: &Value) -> Option<String> {
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
        _tool: &str,
        arguments: &Value,
        output: &ToolOutput,
    ) -> Option<AgentToolDisplay> {
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

fn range_label(range: &AgentCompressRange) -> String {
    match &range.topic {
        Some(topic) => format!("{}–{} {topic}", range.start_ref, range.end_ref),
        None => format!("{}–{}", range.start_ref, range.end_ref),
    }
}
