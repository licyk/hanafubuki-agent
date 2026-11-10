//! The methods of the service every guest provides (`plugin`), as the
//! proxies call them. Each takes one JSON object of camelCase fields and
//! answers one JSON value; the site's plugin rules ("The guest protocol")
//! describe each.

use serde_json::Value;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum GuestMethod {
    /// Runs a declared tool: `{ name, arguments, call }`.
    Tool,
    /// One declared context section: `{ kind, sessionId, mode, userMessage }`
    /// → a string, or `null` for none.
    Context,
    /// The prompt fragment of a sampling step: `{ sessionId, turnId,
    /// samplingStep }` → a string, or `null` for none.
    Prompt,
    /// Before a tool call: `{ call }` → `null` to let it run, or
    /// `{ "deny": reason }`.
    BeforeToolUse,
    /// After a tool call: `{ call, output }` → `null`, or `{ "context":
    /// [text, …] }`.
    AfterToolUse,
    /// A candidate final answer: `{ sessionId, turnId, samplingStep,
    /// candidateFinalAnswer }` → `null` (pass), `"stop"`, or `{ "continue":
    /// [text, …] }`.
    Continuation,
    /// A committed event the guest declared: `{ event, sessionId, … }`; its
    /// answer is ignored.
    Observe,
}

impl GuestMethod {
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Tool => "tool",
            Self::Context => "context",
            Self::Prompt => "prompt",
            Self::BeforeToolUse => "beforeToolUse",
            Self::AfterToolUse => "afterToolUse",
            Self::Continuation => "continuation",
            Self::Observe => "observe",
        }
    }
}

/// What a JSON value is, for a message about an answer.
pub(crate) fn kind(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "a boolean",
        Value::Number(_) => "a number",
        Value::String(_) => "a string",
        Value::Array(_) => "a list",
        Value::Object(_) => "an object",
    }
}

/// The strings of a list answer at `key` of `answer`: `None` when it has no
/// such list of strings.
pub(crate) fn strings(answer: &Value, key: &str) -> Option<Vec<String>> {
    answer
        .get(key)?
        .as_array()?
        .iter()
        .map(|item| item.as_str().map(str::to_string))
        .collect()
}
