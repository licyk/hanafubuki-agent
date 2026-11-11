//! A request's thinking: the `thinking` and `output_config` fields, the
//! `max_tokens` that leaves room for them, and whether the turn's thinking
//! blocks are sent back.
//!
//! Which form a model takes is read from its id:
//!
//! - **Manual** (`thinking: {type: "enabled", budget_tokens}`): Claude 3
//!   and Claude 4 up to 4.5 (Opus 4, 4.1 and 4.5, Sonnet 4 and 4.5, Haiku
//!   4.5), which refuse adaptive thinking, and every id that does not name
//!   a Claude model: the form Anthropic-compatible servers implement.
//! - **Adaptive** (`thinking: {type: "adaptive", display: "summarized"}`
//!   with `output_config: {effort}`): every other Claude model, 4.6 and
//!   later; 4.7 and later refuse the manual form.
//!
//! Manual mode requires the assistant message a tool result continues to
//! start with its thinking block, so its blocks are sent back unchanged.
//! Adaptive mode drops that requirement and allows leaving every block
//! out; it leaves them out, since the models that bind a block to what
//! precedes it (Claude 5.1 and later) refuse one whose prefix changed, and
//! the Agent's requests move context between steps.

use serde_json::{json, Value};

use super::DEFAULT_MAX_OUTPUT_TOKENS;
use crate::model::{ModelInputItem, ModelRequest, ModelToolChoice};
use crate::ModelReasoningEffort;

/// The room a manual request keeps for its answer after the thinking
/// budget.
const MINIMUM_ANSWER_TOKENS: u64 = 4_096;
/// Anthropic's smallest thinking budget.
const MINIMUM_BUDGET_TOKENS: u64 = 1_024;
/// The `max_tokens` an adaptive request asks for, under the model's largest
/// output when that is known; `DEFAULT_MAX_OUTPUT_TOKENS` otherwise.
const ADAPTIVE_MAX_OUTPUT_TOKENS: u64 = 32_768;

/// What a request sends for its reasoning.
pub(super) struct Thinking {
    fields: Vec<(&'static str, Value)>,
    pub(super) max_tokens: u64,
    /// Whether the turn's thinking blocks are sent back.
    pub(super) replay: bool,
}

impl Thinking {
    pub(super) fn of(request: &ModelRequest) -> Self {
        let none = |ceiling: u64| Self {
            fields: Vec::new(),
            max_tokens: ceiling.min(DEFAULT_MAX_OUTPUT_TOKENS),
            replay: false,
        };
        let Some(reasoning) = &request.reasoning else {
            return none(DEFAULT_MAX_OUTPUT_TOKENS);
        };
        let ceiling = reasoning
            .max_output_tokens
            .unwrap_or(DEFAULT_MAX_OUTPUT_TOKENS);
        if !takes_manual_thinking(&request.model) {
            return Self {
                fields: vec![
                    (
                        "thinking",
                        json!({"type": "adaptive", "display": "summarized"}),
                    ),
                    (
                        "output_config",
                        json!({"effort": effort_name(&reasoning.effort)}),
                    ),
                ],
                max_tokens: ceiling.min(ADAPTIVE_MAX_OUTPUT_TOKENS),
                replay: false,
            };
        }
        // Manual thinking refuses a forced tool, and a continuation of an
        // assistant message that starts with no thinking block.
        if matches!(request.tool_choice, ModelToolChoice::Required { .. })
            || continues_without_thinking(&request.input)
        {
            return none(ceiling);
        }
        let budget = budget_tokens(&reasoning.effort);
        let max_tokens = ceiling.min(budget.saturating_add(DEFAULT_MAX_OUTPUT_TOKENS));
        let budget = budget.min(max_tokens.saturating_sub(MINIMUM_ANSWER_TOKENS));
        if budget < MINIMUM_BUDGET_TOKENS {
            return none(ceiling);
        }
        Self {
            fields: vec![(
                "thinking",
                json!({"type": "enabled", "budget_tokens": budget}),
            )],
            max_tokens,
            replay: true,
        }
    }

    pub(super) fn apply(self, body: &mut Value) {
        for (field, value) in self.fields {
            body[field] = value;
        }
    }
}

/// Whether `model` takes manual thinking; see the module's note.
pub(super) fn takes_manual_thinking(model: &str) -> bool {
    let model = model.to_ascii_lowercase();
    let Some(start) = model.find("claude-") else {
        return true;
    };
    let mut parts = model[start + "claude-".len()..]
        .split(['-', '@', ':', '.'])
        .filter(|part| !part.is_empty());
    let first = parts.next();
    if first == Some("3") {
        return true;
    }
    let Some(major) = parts.next() else {
        return false;
    };
    // A minor version is one digit; a date (`-20250514`) or a suffix
    // (`-v1`) means none.
    let minor = parts
        .next()
        .filter(|part| part.len() == 1)
        .and_then(|part| part.parse::<u8>().ok())
        .unwrap_or(0);
    major == "4" && minor <= 5
}

/// The thinking budget of a manual request:
///
/// | effort | budget_tokens |
/// | --- | --- |
/// | `minimal` | 1024 |
/// | `low` | 4096 |
/// | `medium`, or an effort off the ladder | 8192 |
/// | `high` | 16384 |
/// | `xhigh` | 24576 |
/// | `max` | 32000 |
fn budget_tokens(effort: &ModelReasoningEffort) -> u64 {
    match effort.as_str() {
        "minimal" => 1_024,
        "low" => 4_096,
        "high" => 16_384,
        "xhigh" => 24_576,
        "max" => 32_000,
        _ => 8_192,
    }
}

/// The effort an adaptive request names: Anthropic's levels are `low`
/// through `max`, so `minimal` is sent as `low`; any other effort as it is.
fn effort_name(effort: &ModelReasoningEffort) -> &str {
    match effort.as_str() {
        "minimal" => "low",
        effort => effort,
    }
}

/// Whether the request answers an assistant message's tool calls (a tool
/// result follows its last assistant message) and that message does not
/// start with a thinking block to send back.
fn continues_without_thinking(input: &[ModelInputItem]) -> bool {
    let Some(last) = input.iter().rposition(is_assistant_item) else {
        return false;
    };
    if !input[last + 1..]
        .iter()
        .any(|item| matches!(item, ModelInputItem::ToolOutput { .. }))
    {
        return false;
    }
    let start = input[..last]
        .iter()
        .rposition(|item| !is_assistant_item(item))
        .map_or(0, |index| index + 1);
    !matches!(input[start], ModelInputItem::Reasoning { .. })
}

fn is_assistant_item(item: &ModelInputItem) -> bool {
    matches!(
        item,
        ModelInputItem::Reasoning { .. }
            | ModelInputItem::AssistantMessage { .. }
            | ModelInputItem::ToolCall { .. }
    )
}

#[cfg(test)]
mod tests;
