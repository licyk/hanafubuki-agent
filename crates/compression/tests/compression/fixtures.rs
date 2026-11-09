// Derived from billion-context (https://github.com/ranxianglei/billion-context),
// the helpers the tests in `kernel/tests` of acp-kernel 0.0.189 share, at
// commit `4be21b08e`. Copyright (c) 2026 ranxianglei. MIT License with an
// additional attribution term (NOTICE).

use hanafubuki_agent_compression::{
    apply_compression, assign_refs, ApplyInput, ApplyOutcome, Config, Message, RangeSpec, Role,
    State,
};

pub fn user(id: &str, text: &str) -> Message {
    Message::text(id, Role::User, text)
}

pub fn assistant(id: &str, text: &str) -> Message {
    Message::text(id, Role::Assistant, text)
}

pub fn call(id: &str, call_id: &str, tool: &str, arguments: &str) -> Message {
    Message::tool_call(id, call_id, tool, arguments)
}

pub fn result(id: &str, call_id: &str, tool: &str, output: &str) -> Message {
    Message::tool_result(id, call_id, tool, output)
}

pub fn reasoning(id: &str, text: &str) -> Message {
    Message::reasoning(id, text)
}

/// The kernel's tests' configuration: every gate off (no minimum range, no
/// summary limits, no recent zone) unless a test turns one on.
pub fn config() -> Config {
    let mut config = Config::new(100_000);
    config.compress.min_compress_range = 0;
    config.compress.max_summary_length = 0;
    config.compress.min_summary_length = 0;
    config.preserve_recent_messages = 0;
    config.preserve_recent_tokens = 0;
    config
}

/// Refs for `messages` in array order, nothing protected, from `m00001`.
pub fn assign(messages: &[Message], state: &mut State) {
    state.message_refs = assign_refs(messages, &state.message_refs, 1, |_| false).map;
}

/// A fresh state with refs assigned to `messages`.
pub fn state_for(messages: &[Message]) -> State {
    let mut state = State::default();
    assign(messages, &mut state);
    state
}

pub fn range(start: &str, end: &str, summary: &str) -> RangeSpec {
    RangeSpec {
        start_ref: start.to_owned(),
        end_ref: end.to_owned(),
        summary: summary.to_owned(),
        ..RangeSpec::default()
    }
}

pub fn with_topic(mut spec: RangeSpec, topic: &str) -> RangeSpec {
    spec.topic = Some(topic.to_owned());
    spec
}

pub fn with_call(mut spec: RangeSpec, call_id: &str) -> RangeSpec {
    spec.compress_call_id = Some(call_id.to_owned());
    spec
}

/// `applyCompression` with no explicit recent zone (it is computed).
pub fn apply(
    ranges: &[RangeSpec],
    messages: &[Message],
    state: &State,
    config: &Config,
) -> ApplyOutcome {
    apply_compression(&ApplyInput {
        ranges,
        messages,
        state,
        config,
        protected_refs: None,
        now_ms: 0,
    })
}

pub fn ids(messages: &[Message]) -> Vec<&str> {
    messages.iter().map(|message| message.id.as_str()).collect()
}

pub fn sorted(mut ids: Vec<String>) -> Vec<String> {
    ids.sort();
    ids
}
