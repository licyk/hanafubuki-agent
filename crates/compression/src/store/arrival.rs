// Derived from billion-context (https://github.com/ranxianglei/billion-context),
// `storeLargeResults` and `storeCoveredOriginals` of `kernel/src/ccr.ts` of
// acp-kernel 0.0.189 at commit `4be21b08e`. Copyright (c) 2026 ranxianglei.
// MIT License with an additional attribution term (NOTICE).
// Modified for hanafubuki-agent: the store is changed in place.

use std::collections::{HashMap, HashSet};

use super::placeholder::{
    build_stored_placeholder, classify_kind, extract_command, normalize_head,
    parse_stored_placeholder, restore_stored_placeholder_text, strip_leading_tag,
    StoredPlaceholder,
};
use super::{ContentStore, StoreSpec};
use crate::model::{Config, Kind, Message, State};
use crate::prompts::{ACP_TOOL_NAMES, RETRIEVE_TOOL_NAME};
use crate::protect::{
    collect_failed_compress_call_ids, collect_protected_tool_call_ids,
    is_message_protected_with_pairing, match_tool_pattern,
};
use crate::refs::BLOCKED_REF;
use crate::render::default_count_tokens;

/// The view with the large tool results replaced.
#[derive(Clone, Debug, PartialEq)]
pub struct StoreLargeResults {
    pub messages: Vec<Message>,
    /// Originals stored by this call.
    pub stored_count: usize,
}

/// Stores every large tool result of the view that is not stored yet and
/// shows its placeholder instead; a stored ref shows the placeholder it got
/// when it arrived, whatever text comes back for it, so its bytes never
/// change again. Only a result's text changes: its id, call id and call
/// stay. ACP tools, excluded and protected tools are left alone.
pub fn store_large_results(
    messages: Vec<Message>,
    state: &State,
    store: &mut ContentStore,
    config: &Config,
) -> StoreLargeResults {
    let settings = &config.content_store;
    if !settings.enabled {
        return StoreLargeResults {
            messages,
            stored_count: 0,
        };
    }
    let arguments = call_arguments(&messages);
    let protected_calls = collect_protected_tool_call_ids(&messages, config);
    let failed_calls = collect_failed_compress_call_ids(&messages);
    let mut stored_count = 0;
    let mut replaced = Vec::with_capacity(messages.len());
    for message in &messages {
        let text = if message.kind == Kind::ToolResult && !message.text.is_empty() {
            let r#ref = state.message_refs.by_raw.get(&message.id);
            let eligible = !is_own_placeholder(&message.text, r#ref)
                && message.tool_call_id.is_some()
                && !is_excluded(message, &settings.exclude_tools)
                && !is_message_protected_with_pairing(
                    message,
                    config,
                    &protected_calls,
                    &failed_calls,
                );
            match r#ref.filter(|r#ref| eligible && *r#ref != BLOCKED_REF) {
                Some(r#ref) => arrive(message, r#ref, &arguments, store, config, &mut stored_count),
                None => None,
            }
        } else {
            None
        };
        replaced.push(match text {
            Some(text) => Message {
                text,
                ..message.clone()
            },
            None => message.clone(),
        });
    }
    StoreLargeResults {
        messages: replaced,
        stored_count,
    }
}

/// The placeholder for one eligible result, storing its original when it is
/// large and new; `None` leaves it as it is.
fn arrive(
    message: &Message,
    r#ref: &str,
    arguments: &HashMap<&str, &str>,
    store: &mut ContentStore,
    config: &Config,
    stored_count: &mut usize,
) -> Option<String> {
    let max_head = config.content_store.max_head_chars;
    let call = message
        .tool_call_id
        .as_deref()
        .and_then(|call_id| arguments.get(call_id).copied());
    if let Some(entry) = store.by_ref.get(r#ref) {
        return Some(restore_stored_placeholder_text(
            r#ref, entry, call, max_head,
        ));
    }
    let tokens = default_count_tokens(&message.text);
    if tokens < config.content_store.min_tool_tokens {
        return None;
    }
    let placeholder = StoredPlaceholder {
        r#ref: r#ref.to_owned(),
        kind: classify_kind(message.tool_name.as_deref()).to_owned(),
        tokens,
        head: normalize_head(&message.text, max_head),
        command: extract_command(call, max_head),
    };
    store.store_original(StoreSpec {
        r#ref: r#ref.to_owned(),
        raw_id: message.id.clone(),
        text: message.text.clone(),
        kind: placeholder.kind.clone(),
        tool_name: message.tool_name.clone(),
        tokens,
        head: placeholder.head.clone(),
        command: placeholder.command.clone(),
    });
    *stored_count += 1;
    Some(build_stored_placeholder(&placeholder))
}

/// Only a placeholder citing the message's own ref is one the store made;
/// text that merely quotes a placeholder is ordinary content.
fn is_own_placeholder(text: &str, r#ref: Option<&String>) -> bool {
    parse_stored_placeholder(text)
        .is_some_and(|placeholder| r#ref.is_none_or(|r#ref| placeholder.r#ref == *r#ref))
}

fn is_excluded(message: &Message, exclude_tools: &[String]) -> bool {
    let Some(name) = message.tool_name.as_deref() else {
        return false;
    };
    ACP_TOOL_NAMES.contains(&name)
        || name == RETRIEVE_TOOL_NAME
        || exclude_tools
            .iter()
            .any(|pattern| match_tool_pattern(name, pattern))
}

fn call_arguments(messages: &[Message]) -> HashMap<&str, &str> {
    messages
        .iter()
        .filter(|message| message.kind == Kind::ToolCall)
        .filter_map(|message| Some((message.tool_call_id.as_deref()?, message.text.as_str())))
        .collect()
}

/// Stores the originals of the messages the given blocks (active ones) now
/// cover, so a ref folded away can still be retrieved; a ref stored when it
/// arrived keeps that entry. Reasoning and a message's own placeholder are
/// never stored: a placeholder whose original was lost would otherwise be
/// retrieved as if it were the original.
pub fn store_covered_originals(
    store: &mut ContentStore,
    messages: &[Message],
    state: &State,
    block_ids: &[String],
    max_head_chars: usize,
) {
    let covered: HashSet<&str> = state
        .active_blocks()
        .filter(|block| block_ids.contains(&block.block_id))
        .flat_map(|block| block.effective_message_ids.iter().map(String::as_str))
        .collect();
    let arguments = call_arguments(messages);
    for message in messages {
        if !covered.contains(message.id.as_str()) || message.kind == Kind::Reasoning {
            continue;
        }
        let text = strip_leading_tag(&message.text);
        let Some(r#ref) = state.message_refs.by_raw.get(&message.id) else {
            continue;
        };
        if text.is_empty()
            || r#ref == BLOCKED_REF
            || parse_stored_placeholder(text).is_some_and(|placeholder| placeholder.r#ref == *r#ref)
        {
            continue;
        }
        let command = match (&message.kind, message.tool_call_id.as_deref()) {
            (Kind::ToolResult, Some(call_id)) => extract_command(
                arguments
                    .get(call_id)
                    .map(|arguments| strip_leading_tag(arguments)),
                max_head_chars,
            ),
            _ => None,
        };
        store.store_original(StoreSpec {
            r#ref: r#ref.clone(),
            raw_id: message.id.clone(),
            text: text.to_owned(),
            kind: classify_kind(message.tool_name.as_deref()).to_owned(),
            tool_name: message.tool_name.clone(),
            tokens: default_count_tokens(text),
            head: normalize_head(text, max_head_chars),
            command,
        });
    }
}
