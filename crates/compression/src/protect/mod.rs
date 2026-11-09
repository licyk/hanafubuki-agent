// Derived from billion-context (https://github.com/ranxianglei/billion-context),
// `kernel/src/protected.ts` of acp-kernel 0.0.189 at commit `4be21b08e`.
// Copyright (c) 2026 ranxianglei. MIT License with an additional attribution
// term (NOTICE).
// Modified for hanafubuki-agent: the host's protection is the message's own
// flag; a `compress` call is the bare `compress` tool only; there are no
// wire media sidecars (a message's images are reachable through their
// handles, so none is unrecoverable) and no host-projected virtual paths.

mod patterns;
mod zone;

use std::collections::HashSet;

use crate::model::{Config, Kind, Message};

pub(crate) use patterns::match_tool_message_pattern;
pub use patterns::{is_never_preserve_recent, match_tool_path, match_tool_pattern};
pub use zone::{compute_protected_refs, last_user_anchor};

/// The model-facing name of the compress tool.
pub const COMPRESS_TOOL_NAME: &str = "compress";

/// Tools whose records always stay: `compress` calls carry the summaries,
/// `acp_rule` calls the persistent rules. Failed and aged `compress`
/// exchanges are the exception.
pub const ALWAYS_PROTECTED_TOOLS: [&str; 2] = ["compress", "acp_rule"];

/// How a `compress` result that applied nothing begins; such an exchange
/// folds like any other.
pub const FAILED_COMPRESS_RESULT_PREFIXES: [&str; 3] = [
    "[Compression FAILED",
    "[Compression refused",
    "[Compression no-op",
];

/// The newest `compress` exchanges that keep their protection; older ones
/// fold like ordinary exchanges, their summaries living on in the blocks.
pub const COMPRESS_EXCHANGE_KEEP_RECENT: usize = 3;

/// Tools whose large results never count toward the recent zone, so they
/// can be folded again at once.
pub const NEVER_PRESERVE_RECENT_TOOLS: [&str; 4] = ["decompress", "search_context", "read", "bash"];

fn is_compress_call(message: &Message) -> bool {
    message.kind == Kind::ToolCall && message.tool_name_is(COMPRESS_TOOL_NAME)
}

/// The result half of a `compress` exchange that applied nothing.
pub fn is_failed_compress_result(message: &Message) -> bool {
    message.kind == Kind::ToolResult
        && message.tool_name_is(COMPRESS_TOOL_NAME)
        && FAILED_COMPRESS_RESULT_PREFIXES
            .iter()
            .any(|prefix| message.text.starts_with(prefix))
}

/// The tool-call ids of the `compress` exchanges that applied nothing; the
/// call half has no outcome of its own.
pub fn collect_failed_compress_call_ids(messages: &[Message]) -> HashSet<String> {
    messages
        .iter()
        .filter(|message| is_failed_compress_result(message))
        .filter_map(|message| message.tool_call_id.clone())
        .collect()
}

/// The tool-call ids of every `compress` exchange but the newest
/// [`COMPRESS_EXCHANGE_KEEP_RECENT`].
pub fn collect_aged_compress_call_ids(messages: &[Message], keep_recent: usize) -> HashSet<String> {
    let mut recent: Vec<&str> = Vec::new();
    for message in messages.iter().filter(|message| is_compress_call(message)) {
        let Some(call_id) = message.tool_call_id.as_deref() else {
            continue;
        };
        recent.retain(|id| *id != call_id);
        recent.push(call_id);
        if recent.len() > keep_recent {
            recent.remove(0);
        }
    }
    messages
        .iter()
        .filter(|message| is_compress_call(message))
        .filter_map(|message| message.tool_call_id.as_deref())
        .filter(|call_id| !recent.contains(call_id))
        .map(str::to_owned)
        .collect()
}

/// Whether a message must never fold: the host says so, or it belongs to an
/// always-protected or configured protected tool. `failed_call_ids` are the
/// failed `compress` exchanges, which never are.
pub fn is_message_protected(
    message: &Message,
    config: &Config,
    failed_call_ids: &HashSet<String>,
) -> bool {
    if message.protected {
        return true;
    }
    let failed_call = message
        .tool_call_id
        .as_ref()
        .is_some_and(|call_id| failed_call_ids.contains(call_id));
    if is_failed_compress_result(message) || failed_call {
        return false;
    }
    let Some(tool_name) = message.tool_name.as_deref().filter(|name| !name.is_empty()) else {
        return false;
    };
    if !message.is_tool() {
        return false;
    }
    ALWAYS_PROTECTED_TOOLS.contains(&tool_name)
        || config
            .protected_tools
            .iter()
            .any(|pattern| match_tool_message_pattern(message, pattern))
}

/// The tool-call ids of the protected calls, so results without a tool
/// name are protected through their call.
pub(crate) fn collect_protected_tool_call_ids(
    messages: &[Message],
    config: &Config,
) -> HashSet<String> {
    let failed = collect_failed_compress_call_ids(messages);
    messages
        .iter()
        .filter(|message| message.kind == Kind::ToolCall)
        .filter(|message| is_message_protected(message, config, &failed))
        .filter_map(|message| message.tool_call_id.clone())
        .collect()
}

/// [`is_message_protected`], with a result also protected when its call is.
pub(crate) fn is_message_protected_with_pairing(
    message: &Message,
    config: &Config,
    protected_call_ids: &HashSet<String>,
    failed_call_ids: &HashSet<String>,
) -> bool {
    if is_message_protected(message, config, failed_call_ids) {
        return true;
    }
    message.kind == Kind::ToolResult
        && !is_failed_compress_result(message)
        && message
            .tool_call_id
            .as_ref()
            .is_some_and(|call_id| protected_call_ids.contains(call_id))
}

/// The latest call of each `protected_latest_tools` pattern (per tool path
/// for a pattern with `/`): by tool-call id, or by message id for a call
/// without one.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LatestProtected {
    pub call_ids: HashSet<String>,
    pub message_ids: HashSet<String>,
}

pub fn collect_latest_protected(messages: &[Message], config: &Config) -> LatestProtected {
    let mut latest = LatestProtected::default();
    let calls = || {
        messages
            .iter()
            .filter(|message| message.kind == Kind::ToolCall && message.tool_name.is_some())
    };
    let mut record = |message: &Message| match &message.tool_call_id {
        Some(call_id) => {
            latest.call_ids.insert(call_id.clone());
        }
        None => {
            latest.message_ids.insert(message.id.clone());
        }
    };
    for pattern in &config.protected_latest_tools {
        if pattern.contains('/') {
            let mut last_by_path: Vec<(&str, &Message)> = Vec::new();
            for message in calls().filter(|message| match_tool_message_pattern(message, pattern)) {
                let path = message.tool_name.as_deref().unwrap_or_default();
                match last_by_path.iter_mut().find(|(known, _)| *known == path) {
                    Some(entry) => entry.1 = message,
                    None => last_by_path.push((path, message)),
                }
            }
            for (_, message) in last_by_path {
                record(message);
            }
            continue;
        }
        if let Some(message) = calls().rfind(|message| match_tool_message_pattern(message, pattern))
        {
            record(message);
        }
    }
    latest
}

/// A latest-protected call, or the result paired with one.
pub fn is_message_latest_protected(message: &Message, latest: &LatestProtected) -> bool {
    if latest.message_ids.contains(&message.id) {
        return true;
    }
    message.is_tool()
        && message
            .tool_call_id
            .as_ref()
            .is_some_and(|call_id| latest.call_ids.contains(call_id))
}
