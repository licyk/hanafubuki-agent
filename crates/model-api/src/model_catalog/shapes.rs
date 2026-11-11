//! The fields a listing entry reports its model's limits and capabilities
//! under, across the listing shapes: OpenAI-compatible servers, OpenRouter,
//! Anthropic, vLLM and Codex.
//!
//! Reasoning efforts are the first list of these that names any: Codex's
//! `supported_reasoning_levels[].effort`, OpenRouter's
//! `reasoning.supported_efforts`, Anthropic's `capabilities.effort.<level>`
//! whose `supported` is true. The default is Codex's
//! `default_reasoning_level`, else OpenRouter's `reasoning.default_effort`.
//! An effort that does not parse (`none` among them) is left out.
//!
//! A limit is the first of its fields that holds a positive integer. A
//! capability is yes when any of its fields says yes, else no when any says
//! no, else unknown: a wrong yes costs one refused request, a wrong no would
//! withhold images or reasoning from a model that takes them. A flag that is
//! not a boolean, or a list that holds no string, says nothing.

use serde_json::Value;

use crate::ModelReasoningEffort;

const CONTEXT_WINDOW_FIELDS: &[&[&str]] = &[
    &["max_input_tokens"],
    &["context_length"],
    &["max_model_len"],
    &["context_window"],
    &["contextWindow"],
    &["top_provider", "context_length"],
];

/// Anthropic's `max_tokens` is the largest output it accepts.
const MAX_OUTPUT_FIELDS: &[&[&str]] = &[
    &["max_output_tokens"],
    &["max_completion_tokens"],
    &["maxTokens"],
    &["top_provider", "max_completion_tokens"],
    &["max_tokens"],
];

const INPUT_MODALITY_FIELDS: &[&[&str]] = &[
    &["architecture", "input_modalities"],
    &["input_modalities"],
    &["input"],
];

const IMAGE_INPUT_FLAGS: &[&[&str]] = &[
    &["vision"],
    &["capabilities", "vision"],
    &["capabilities", "image_input"],
];

const REASONING_FLAGS: &[&[&str]] = &[
    &["reasoning"],
    &["capabilities", "reasoning"],
    &["capabilities", "thinking"],
];

pub(super) fn context_window(item: &Value) -> Option<u64> {
    first_positive(item, CONTEXT_WINDOW_FIELDS)
}

pub(super) fn max_output_tokens(item: &Value) -> Option<u64> {
    first_positive(item, MAX_OUTPUT_FIELDS)
}

pub(super) fn image_input(item: &Value) -> Option<bool> {
    let modalities = INPUT_MODALITY_FIELDS
        .iter()
        .map(|path| lists_any(field(item, path), &["image"]));
    any_yes(modalities.chain(flags(item, IMAGE_INPUT_FLAGS)))
}

/// `reports_levels`: the entry lists reasoning efforts, which says yes.
pub(super) fn reasoning(item: &Value, reports_levels: bool) -> Option<bool> {
    let parameters = lists_any(
        item.get("supported_parameters"),
        &["reasoning", "reasoning_effort"],
    );
    any_yes(
        [parameters, reports_levels.then_some(true)]
            .into_iter()
            .chain(flags(item, REASONING_FLAGS)),
    )
}

/// The efforts the entry lists, in its order; see the module's note.
pub(super) fn reasoning_efforts(item: &Value) -> Vec<ModelReasoningEffort> {
    let codex = item
        .get("supported_reasoning_levels")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|level| level.get("effort"));
    let openrouter = field(item, &["reasoning", "supported_efforts"])
        .and_then(Value::as_array)
        .into_iter()
        .flatten();
    let anthropic = field(item, &["capabilities", "effort"])
        .and_then(Value::as_object)
        .into_iter()
        .flatten()
        .filter(|(_, level)| level.get("supported").and_then(Value::as_bool) == Some(true))
        .filter_map(|(name, _)| ModelReasoningEffort::parse(name.as_str()).ok());
    let parsed = |values: &mut dyn Iterator<Item = &Value>| {
        values.filter_map(reasoning_effort).collect::<Vec<_>>()
    };
    [
        parsed(&mut codex.into_iter()),
        parsed(&mut openrouter.into_iter()),
        anthropic.collect(),
    ]
    .into_iter()
    .find(|efforts| !efforts.is_empty())
    .unwrap_or_default()
}

/// The effort the entry names its default; see the module's note.
pub(super) fn default_reasoning_effort(item: &Value) -> Option<ModelReasoningEffort> {
    item.get("default_reasoning_level")
        .and_then(reasoning_effort)
        .or_else(|| field(item, &["reasoning", "default_effort"]).and_then(reasoning_effort))
}

fn reasoning_effort(value: &Value) -> Option<ModelReasoningEffort> {
    ModelReasoningEffort::parse(value.as_str()?).ok()
}

fn first_positive(item: &Value, paths: &[&[&str]]) -> Option<u64> {
    paths
        .iter()
        .find_map(|path| field(item, path)?.as_u64().filter(|tokens| *tokens > 0))
}

/// A capability flag: a boolean, or Anthropic's `{"supported": boolean}`.
fn flags<'a>(item: &'a Value, paths: &'a [&[&str]]) -> impl Iterator<Item = Option<bool>> + 'a {
    paths.iter().map(|path| {
        let value = field(item, path)?;
        value
            .as_bool()
            .or_else(|| value.get("supported")?.as_bool())
    })
}

/// Whether the list at `value` holds one of `wanted`, ignoring case.
fn lists_any(value: Option<&Value>, wanted: &[&str]) -> Option<bool> {
    let mut strings = value?
        .as_array()?
        .iter()
        .filter_map(Value::as_str)
        .peekable();
    strings.peek()?;
    Some(strings.any(|listed| wanted.iter().any(|want| listed.eq_ignore_ascii_case(want))))
}

fn any_yes(signals: impl Iterator<Item = Option<bool>>) -> Option<bool> {
    signals.flatten().reduce(|left, right| left || right)
}

fn field<'a>(item: &'a Value, path: &[&str]) -> Option<&'a Value> {
    path.iter().try_fold(item, |value, key| value.get(key))
}

#[cfg(test)]
mod tests;
