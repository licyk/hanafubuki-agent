// Derived from billion-context (https://github.com/ranxianglei/billion-context),
// `kernel/src/render-refs.ts` of acp-kernel 0.0.189 at commit `4be21b08e`.
// Copyright (c) 2026 ranxianglei. MIT License with an additional attribution
// term (NOTICE).
// Modified for hanafubuki-agent: a strategy that tags tool results but never
// tool-call arguments (whose text is JSON) replaces tagging everything, the
// token count is always frozen at first render, and there is no time
// attribute.

mod tokenize;

pub use tokenize::{count_message_tokens, default_count_tokens};

use crate::model::{Kind, Message, State};
use crate::refs::BLOCKED_REF;

/// Which messages get an `<acp>` ref tag prepended to their text. Refs are
/// assigned either way.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RenderStrategy {
    /// User and assistant text only.
    TextOnly,
    /// Text and tool results, never tool-call arguments.
    TextAndToolResults,
    /// No tags: the host reads the refs from the state.
    None,
}

const TAG_OPEN: &str = "<acp ";
const TAG_CLOSE: &str = "</acp>";

/// Prepends `<acp tokens="…" type="…">mNNNNN</acp>` and a newline to each
/// message the strategy tags. The token count is the one the ref showed
/// first, recorded in `state.token_snapshot`, so a tag never changes. A
/// message whose ref left `BLOCKED` stays untagged until the next fold.
pub fn render_refs(
    messages: &[Message],
    state: &mut State,
    strategy: RenderStrategy,
) -> Vec<Message> {
    messages
        .iter()
        .map(|message| render_message(message, state, strategy))
        .collect()
}

fn render_message(message: &Message, state: &mut State, strategy: RenderStrategy) -> Message {
    let Some(r#ref) = state.message_refs.by_raw.get(&message.id).cloned() else {
        return message.clone();
    };
    if r#ref == BLOCKED_REF || state.render_deferred_refs.contains(&r#ref) {
        return message.clone();
    }
    let tagged = match strategy {
        RenderStrategy::TextOnly => message.kind == Kind::Text,
        RenderStrategy::TextAndToolResults => matches!(message.kind, Kind::Text | Kind::ToolResult),
        RenderStrategy::None => false,
    };
    if !tagged {
        return message.clone();
    }
    let clean = strip_own_tag(&message.text, &r#ref);
    let tokens = *state
        .token_snapshot
        .entry(r#ref.clone())
        .or_insert_with(|| default_count_tokens(clean));
    let tag = format!(
        "{TAG_OPEN}tokens=\"{}\" type=\"{}\">{}{TAG_CLOSE}\n",
        format_tokens(tokens),
        type_of(message),
        r#ref,
    );
    Message {
        text: format!("{tag}{clean}"),
        ..message.clone()
    }
}

/// The text without a tag for this ref at its start, so rendering twice
/// changes nothing; tags for other refs are content and stay.
fn strip_own_tag<'a>(text: &'a str, r#ref: &str) -> &'a str {
    let Some(rest) = text.strip_prefix(TAG_OPEN) else {
        return text;
    };
    let Some(close) = rest.find('>') else {
        return text;
    };
    let Some(rest) = rest[close + 1..]
        .strip_prefix(r#ref)
        .and_then(|rest| rest.strip_prefix(TAG_CLOSE))
    else {
        return text;
    };
    rest.strip_prefix('\n').unwrap_or(rest)
}

fn type_of(message: &Message) -> &str {
    match message.kind {
        Kind::ToolCall | Kind::ToolResult => message
            .tool_name
            .as_deref()
            .filter(|name| !name.is_empty())
            .unwrap_or("tool"),
        Kind::Text => "text",
        Kind::Reasoning => "reasoning",
    }
}

/// Below 1000 as is, below 10,000 as `X.YK`, else as `XK`, rounded as
/// JavaScript's `toFixed(1)` and `Math.round` round.
pub(crate) fn format_tokens(tokens: u64) -> String {
    if tokens < 1000 {
        return tokens.to_string();
    }
    if tokens < 10_000 {
        // An exact tie (tokens ≡ 250 mod 500) rounds up in JavaScript and to
        // even in Rust; every other value rounds the same way in both.
        if tokens % 500 == 250 {
            let tenths = (tokens + 50) / 100;
            return format!("{}.{}K", tenths / 10, tenths % 10);
        }
        return format!("{:.1}K", tokens as f64 / 1000.0);
    }
    format!("{}K", (tokens as f64 / 1000.0).round())
}
