// Derived from billion-context (https://github.com/ranxianglei/billion-context),
// the helpers the tests in `kernel/tests` of acp-kernel 0.0.189 share, at
// commit `4be21b08e`. Copyright (c) 2026 ranxianglei. MIT License with an
// additional attribution term (NOTICE).

use hanafubuki_agent_compression::{
    apply_compression, assign_refs, ApplyInput, ApplyOutcome, BlockSpan, CompressibleRange, Config,
    ContextBreakdown, Message, NudgeDecision, ProtectedRange, RangeSpec, Role, State,
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

/// A nudge decision with ranges, protected ranges, a long block map and a
/// breakdown, whose texts are checked byte for byte against the kernel.
pub fn crossed() -> NudgeDecision {
    let range = |start: usize, end: usize, tokens, chars, tool_pct, user_msgs| CompressibleRange {
        start_ref: format!("m{start:05}"),
        end_ref: format!("m{end:05}"),
        count: end - start + 1,
        tokens,
        chars: Some(chars),
        tool_pct,
        text_pct: 100 - tool_pct,
        user_msgs,
        start_index: Some(start - 1),
        end_index: Some(end - 1),
        no_merge_before: false,
    };
    NudgeDecision {
        should_inject: true,
        reason: "r".to_owned(),
        compressible_ranges: vec![
            range(3, 10, 61_234, 240_000, 82, 3),
            range(11, 12, 1250, 5000, 50, 1),
            range(20, 30, 9999, 40_000, 0, 0),
        ],
        protected_ranges: vec![ProtectedRange {
            start_ref: "m00014".to_owned(),
            end_ref: "m00015".to_owned(),
            count: 2,
            tokens: 800,
            tools: vec!["skill".to_owned(), "task".to_owned()],
            start_index: Some(13),
            end_index: Some(14),
            no_merge_before: false,
        }],
        active_block_spans: (0..9)
            .map(|index| BlockSpan {
                block_id: format!("b{}", index + 1),
                tier: if index == 8 { 2 } else { 1 },
                start_ref: format!("m{:05}", 100 + index * 10),
                end_ref: format!("m{:05}", 108 + index * 10),
            })
            .collect(),
        context_usage: 0.5,
        context_breakdown: Some(ContextBreakdown {
            system: 1200,
            tool: 45_678,
            summaries: 3000,
            code: 0,
            text: 999,
            total: 60_000,
            growth: 22_500,
        }),
        ..NudgeDecision::default()
    }
}
