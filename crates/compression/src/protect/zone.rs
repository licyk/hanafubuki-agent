// Derived from billion-context (https://github.com/ranxianglei/billion-context),
// `computeProtectedRefs` of `kernel/src/recommend.ts` and `lastUserAnchor`
// of `kernel/src/protected.ts` of acp-kernel 0.0.189 at commit `4be21b08e`.
// Copyright (c) 2026 ranxianglei. MIT License with an additional attribution
// term (NOTICE).

use std::collections::{HashMap, HashSet};

use super::is_never_preserve_recent;
use crate::model::{Config, Kind, Message, Role, State};
use crate::prune::SUMMARY_HEADER;
use crate::refs::BLOCKED_REF;
use crate::render::count_message_tokens;

/// The newest user message that is a real request: not a rendered summary
/// and not covered by a block. It anchors the recent zone.
pub fn last_user_anchor<'a>(
    messages: &'a [Message],
    covered: &HashSet<&str>,
) -> Option<&'a Message> {
    messages.iter().rev().find(|message| {
        message.role == Role::User
            && !message.text.starts_with(SUMMARY_HEADER)
            && !covered.contains(message.id.as_str())
    })
}

/// The refs of the soft-protected recent zone: the last
/// `preserve_recent_messages` visible messages, the messages back from the
/// tail until `preserve_recent_tokens`, and the last user request (with
/// recent-message protection on); then both halves of every tool pair one
/// half of which is in it. Large-result tools
/// ([`is_never_preserve_recent`]) do not count.
pub fn compute_protected_refs(
    messages: &[Message],
    state: &State,
    config: &Config,
) -> HashSet<String> {
    let covered = state.covered_message_ids();
    let is_synthetic_or_pruned = |message: &Message| {
        message.text.starts_with(SUMMARY_HEADER) || covered.contains(message.id.as_str())
    };
    let ref_of = |message: &Message| {
        state
            .message_refs
            .by_raw
            .get(&message.id)
            .filter(|r#ref| *r#ref != BLOCKED_REF)
            .cloned()
    };

    let visible: Vec<(String, u64)> = messages
        .iter()
        .filter(|message| !is_synthetic_or_pruned(message))
        .filter(|message| {
            !is_never_preserve_recent(
                message,
                config.never_preserve_recent_tools.as_deref(),
                &config.preserve_recent_tools,
            )
        })
        .filter_map(|message| Some((ref_of(message)?, count_message_tokens(message))))
        .collect();

    let mut zone = HashSet::new();
    let preserve_messages = config.preserve_recent_messages;
    if preserve_messages > 0 {
        let start = visible.len().saturating_sub(preserve_messages);
        zone.extend(visible[start..].iter().map(|(r#ref, _)| r#ref.clone()));
    }
    if config.preserve_recent_tokens > 0 {
        let mut tokens = 0;
        for (r#ref, message_tokens) in visible.iter().rev() {
            if tokens >= config.preserve_recent_tokens {
                break;
            }
            zone.insert(r#ref.clone());
            tokens += message_tokens;
        }
    }
    if preserve_messages > 0 {
        if let Some(r#ref) = last_user_anchor(messages, &covered).and_then(ref_of) {
            zone.insert(r#ref);
        }
    }

    close_tool_pairs(messages, &mut zone, is_synthetic_or_pruned, ref_of);
    zone
}

/// A large-result tool's result never enters the zone while its call can,
/// and keeping the call while its result folds strands the call: protect
/// both halves of every pair one half of which is protected.
fn close_tool_pairs(
    messages: &[Message],
    zone: &mut HashSet<String>,
    is_synthetic_or_pruned: impl Fn(&Message) -> bool,
    ref_of: impl Fn(&Message) -> Option<String>,
) {
    let mut message_by_ref: HashMap<String, &Message> = HashMap::new();
    for message in messages.iter().filter(|message| !message.id.is_empty()) {
        if let Some(r#ref) = ref_of(message) {
            message_by_ref.insert(r#ref, message);
        }
    }
    let mut call_refs: HashMap<&str, String> = HashMap::new();
    let mut result_refs: HashMap<&str, String> = HashMap::new();
    for message in messages {
        let Some(call_id) = message.tool_call_id.as_deref() else {
            continue;
        };
        if message.id.is_empty() || is_synthetic_or_pruned(message) {
            continue;
        }
        let Some(r#ref) = ref_of(message) else {
            continue;
        };
        match message.kind {
            Kind::ToolCall => {
                call_refs.insert(call_id, r#ref);
            }
            Kind::ToolResult => {
                result_refs.insert(call_id, r#ref);
            }
            Kind::Text | Kind::Reasoning => {}
        }
    }
    let protected: Vec<String> = zone.iter().cloned().collect();
    for r#ref in protected {
        let Some(message) = message_by_ref.get(&r#ref) else {
            continue;
        };
        let Some(call_id) = message.tool_call_id.as_deref() else {
            continue;
        };
        let partner = match message.kind {
            Kind::ToolCall => result_refs.get(call_id),
            Kind::ToolResult => call_refs.get(call_id),
            Kind::Text | Kind::Reasoning => None,
        };
        if let Some(partner) = partner {
            zone.insert(partner.clone());
        }
    }
}
