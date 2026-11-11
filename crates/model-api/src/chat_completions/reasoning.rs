//! The reasoning effort of a Chat Completions request: OpenAI's top-level
//! `reasoning_effort`, which OpenRouter and DeepSeek also take, sent only
//! when the request carries an effort (none: the Provider's default).

use serde_json::{json, Value};

use crate::ModelReasoningConfig;

pub(super) fn encode_reasoning(body: &mut Value, reasoning: Option<&ModelReasoningConfig>) {
    if let Some(reasoning) = reasoning {
        body["reasoning_effort"] = json!(reasoning.effort.as_str());
    }
}
