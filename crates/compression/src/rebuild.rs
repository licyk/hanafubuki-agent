// Derived from billion-context (https://github.com/ranxianglei/billion-context),
// `kernel/src/rebuild.ts` of acp-kernel 0.0.189 at commit `4be21b08e`.
// Copyright (c) 2026 ranxianglei. MIT License with an additional attribution
// term (NOTICE).
// Modified for hanafubuki-agent: refs are minted with the view's protection,
// so a protected message takes no number here either and the refs the
// historical calls cite land where they did; the caller passes the time.

use serde_json::Value;

use crate::apply::{apply_compression, ApplyInput};
use crate::model::{Config, Kind, Message, State};
use crate::parse::parse_compress_args;
use crate::protect::COMPRESS_TOOL_NAME;
use crate::refs::assign_message_refs;

#[derive(Clone, Debug, PartialEq)]
pub struct RebuildResult {
    pub state: State,
    pub blocks_rebuilt: usize,
}

/// Rebuilds a state by replaying the `compress` calls in `messages`, in
/// order, over `state` (an empty one when the state was lost). Refs are
/// minted in history order, so a ref an old call cites names the same
/// message as when it was made. The result approximates the original: the
/// recent zone is the one at the end of `messages`, and the arguments are
/// parsed leniently, so a truncated or stringified call still counts.
pub fn rebuild_state(
    state: State,
    messages: &[Message],
    config: &Config,
    now_ms: u64,
) -> RebuildResult {
    let mut working = state;
    assign_message_refs(&mut working, messages, config);
    let mut blocks_rebuilt = 0;
    for call in messages.iter().filter(|message| {
        message.kind == Kind::ToolCall && message.tool_name_is(COMPRESS_TOOL_NAME)
    }) {
        let parsed = parse_compress_args(
            &Value::String(call.text.clone()),
            call.tool_call_id.as_deref(),
        );
        if parsed.ranges.is_empty() {
            continue;
        }
        let outcome = apply_compression(&ApplyInput {
            ranges: &parsed.ranges,
            messages,
            state: &working,
            config,
            protected_refs: None,
            now_ms,
        });
        working = outcome.state;
        blocks_rebuilt += outcome.result.blocks_created;
    }
    RebuildResult {
        state: working,
        blocks_rebuilt,
    }
}
