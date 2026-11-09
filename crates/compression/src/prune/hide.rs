// Derived from billion-context (https://github.com/ranxianglei/billion-context),
// `kernel/src/hide-consumed.ts` and the hide-compress-calls step of
// `kernel/src/compress.ts` of acp-kernel 0.0.189 at commit `4be21b08e`.
// Copyright (c) 2026 ranxianglei. MIT License with an additional attribution
// term (NOTICE).
// Modified for hanafubuki-agent: a `compress` call is the bare `compress`
// tool, so there are no foreign-name carriers to match by range; rewritten
// arguments keep their keys in order.

use std::collections::{HashMap, HashSet};

use super::json::Json;

use crate::model::{Kind, Message, State};
use crate::protect::COMPRESS_TOOL_NAME;
use crate::refs::{ref_to_index, BLOCKED_REF};
use crate::text::{clamp_prefix, utf16_len};

/// A live call keeps this much of each summary in its arguments; the block
/// is the durable record.
const SUMMARY_STUB_CHARS: usize = 200;

/// The view with consumed and swept `compress` exchanges hidden.
#[derive(Clone, Debug, PartialEq)]
pub struct HiddenCompressCalls {
    pub messages: Vec<Message>,
    pub hidden: usize,
    /// The refs of every hidden orphan call and result.
    pub hidden_orphan_refs: Vec<String>,
    /// Orphan call ids this pass swept.
    pub newly_swept_call_ids: Vec<String>,
}

impl HiddenCompressCalls {
    /// Records the pass in the state: the hidden orphan refs, the swept ids
    /// added to the permanent set, the fold marker consumed.
    pub fn commit(&self, state: &mut State) {
        state.hidden_orphan_refs = self.hidden_orphan_refs.clone();
        state.pending_orphan_sweep_min_ref = None;
        for call_id in &self.newly_swept_call_ids {
            if !state.swept_orphan_call_ids.contains(call_id) {
                state.swept_orphan_call_ids.push(call_id.clone());
            }
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Fate {
    Active,
    Inactive,
    Orphan,
}

/// Hides the `compress` exchanges whose block is inactive (consumed or
/// expanded), and the orphan ones (no block) once a fold happened at or
/// before them, after which they stay hidden; between folds no orphan is
/// hidden, so a quiet request never moves the cached prefix. A live call's
/// arguments are cut to the ranges still live, each summary to a stub.
pub fn hide_consumed_compress_calls(state: &State, messages: &[Message]) -> HiddenCompressCalls {
    let mut all_block_calls: HashSet<&str> = HashSet::new();
    let mut active_calls: HashSet<&str> = HashSet::new();
    let mut live_keys_by_call: HashMap<&str, HashSet<String>> = HashMap::new();
    let mut legacy_live: HashSet<&str> = HashSet::new();
    let mut active_range_keys: HashSet<String> = HashSet::new();
    for block in &state.blocks {
        let Some(call_id) = block.compress_call_id.as_deref() else {
            continue;
        };
        all_block_calls.insert(call_id);
        if !block.active {
            continue;
        }
        active_calls.insert(call_id);
        let (Some(start), Some(end)) = (&block.start_ref, &block.end_ref) else {
            legacy_live.insert(call_id);
            continue;
        };
        active_range_keys.insert(range_key(start, end));
        live_keys_by_call
            .entry(call_id)
            .or_default()
            .insert(range_key(start, end));
    }

    let is_compress_call = |message: &Message| {
        message.kind == Kind::ToolCall && message.tool_name_is(COMPRESS_TOOL_NAME)
    };
    let fate_of = |message: &Message| match message.tool_call_id.as_deref() {
        Some(id) if active_calls.contains(id) => Fate::Active,
        Some(id) if all_block_calls.contains(id) => Fate::Inactive,
        _ => Fate::Orphan,
    };

    let swept: HashSet<&str> = state
        .swept_orphan_call_ids
        .iter()
        .map(String::as_str)
        .collect();
    let mut newly_swept_call_ids = Vec::new();
    let mut keep_calls: HashSet<&str> = HashSet::new();
    let mut hidden_calls: HashSet<&str> = HashSet::new();
    let mut fate_by_call: HashMap<&str, Fate> = HashMap::new();
    for message in messages.iter().filter(|message| is_compress_call(message)) {
        let Some(call_id) = message.tool_call_id.as_deref() else {
            continue;
        };
        let fate = fate_of(message);
        fate_by_call.insert(call_id, fate);
        let hide = match fate {
            Fate::Active => false,
            Fate::Inactive => true,
            Fate::Orphan => {
                let r#ref = state.message_refs.by_raw.get(&message.id);
                if swept.contains(call_id)
                    || r#ref.is_some_and(|r#ref| state.hidden_orphan_refs.contains(r#ref))
                {
                    true
                } else {
                    let sweeps = state.pending_orphan_sweep_min_ref.is_some_and(|fold_min| {
                        !quotes_active_block(&message.text, &active_range_keys)
                            && ref_number(r#ref).is_some_and(|number| number >= fold_min)
                    });
                    if sweeps {
                        newly_swept_call_ids.push(call_id.to_owned());
                    }
                    sweeps
                }
            }
        };
        if hide {
            hidden_calls.insert(call_id);
        } else {
            keep_calls.insert(call_id);
        }
    }

    let mut hidden = 0;
    let mut hidden_orphan_refs: Vec<String> = Vec::new();
    let mut result = Vec::with_capacity(messages.len());
    for message in messages {
        let call_id = message.tool_call_id.as_deref();
        let compress_call = is_compress_call(message);
        let hides = match call_id {
            None => compress_call,
            Some(call_id) => {
                (compress_call || message.kind == Kind::ToolResult)
                    && hidden_calls.contains(call_id)
            }
        };
        if hides {
            hidden += 1;
            let orphan = call_id.and_then(|id| fate_by_call.get(id)) == Some(&Fate::Orphan);
            if let Some(r#ref) = state.message_refs.by_raw.get(&message.id) {
                if orphan && r#ref != BLOCKED_REF && !hidden_orphan_refs.contains(r#ref) {
                    hidden_orphan_refs.push(r#ref.clone());
                }
            }
            continue;
        }
        if let Some(call_id) = call_id.filter(|id| compress_call && keep_calls.contains(id)) {
            if let Some(text) = live_call_text(message, call_id, &live_keys_by_call, &legacy_live) {
                result.push(Message {
                    text,
                    ..message.clone()
                });
                continue;
            }
        }
        result.push(message.clone());
    }

    HiddenCompressCalls {
        messages: result,
        hidden,
        hidden_orphan_refs,
        newly_swept_call_ids,
    }
}

/// A kept call's arguments, cut to its live ranges, or only compacted.
fn live_call_text(
    message: &Message,
    call_id: &str,
    live_keys_by_call: &HashMap<&str, HashSet<String>>,
    legacy_live: &HashSet<&str>,
) -> Option<String> {
    if !legacy_live.contains(call_id) {
        if let Some(live_keys) = live_keys_by_call
            .get(call_id)
            .filter(|keys| !keys.is_empty())
        {
            if let Some(rewritten) = rewrite_compress_text(&message.text, live_keys) {
                return Some(rewritten);
            }
        }
    }
    compact_compress_text(&message.text)
}

fn range_key(start: &str, end: &str) -> String {
    format!("{start}::{end}")
}

/// The number of a ref; `None` for a blocked or unmapped one, so nothing is
/// swept on a guess.
fn ref_number(r#ref: Option<&String>) -> Option<u32> {
    r#ref
        .filter(|r#ref| *r#ref != BLOCKED_REF)
        .and_then(|r#ref| ref_to_index(r#ref))
}

/// The `startId`/`endId` (or `messageId`) key of an argument entry.
fn entry_range_key(entry: &Json) -> Option<String> {
    let Json::Object(_) = entry else {
        return None;
    };
    let field = |name: &str| entry.get(name).and_then(Json::as_str);
    let start = field("startId")
        .or_else(|| field("messageId"))
        .unwrap_or_default();
    let end = field("endId")
        .or_else(|| field("messageId"))
        .unwrap_or_default();
    Some(range_key(start, end))
}

/// The JSON object after any rendered prefix.
fn parse_arguments(text: &str) -> Option<(&str, Json)> {
    let start = text.find('{')?;
    let object @ Json::Object(_) = Json::parse(&text[start..])? else {
        return None;
    };
    Some((&text[..start], object))
}

/// Whether a call's arguments quote an active block's range: such a call is
/// that block's summary carrier, which no sweep may hide. Unparseable
/// arguments are not a carrier.
fn quotes_active_block(text: &str, active_range_keys: &HashSet<String>) -> bool {
    let Some((_, object)) = parse_arguments(text) else {
        return false;
    };
    let entries = match object.get("content") {
        Some(Json::Array(entries)) => entries.clone(),
        Some(Json::String(inner)) => match Json::parse(inner) {
            Some(Json::Array(entries)) => entries,
            Some(_) => Vec::new(),
            None => return false,
        },
        _ => vec![object.clone()],
    };
    entries.iter().any(|entry| {
        let field = |name: &str| {
            entry
                .get(name)
                .and_then(Json::as_str)
                .filter(|value| !value.is_empty())
        };
        let start = field("startId").or_else(|| field("messageId"));
        let end = field("endId").or_else(|| field("messageId"));
        matches!((start, end), (Some(start), Some(end)) if active_range_keys.contains(&range_key(start, end)))
    })
}

struct CallArguments<'a> {
    prefix: &'a str,
    object: Json,
    content: Vec<Json>,
    content_was_string: bool,
}

/// A call's `content` entries, an array or a stringified one.
fn parse_call_text(text: &str) -> Option<CallArguments<'_>> {
    let (prefix, object) = parse_arguments(text)?;
    let (content, content_was_string) = match object.get("content") {
        Some(Json::Array(entries)) => (entries.clone(), false),
        Some(Json::String(inner)) => match Json::parse(inner) {
            Some(Json::Array(entries)) => (entries, true),
            _ => return None,
        },
        _ => return None,
    };
    (!content.is_empty()).then_some(CallArguments {
        prefix,
        object,
        content,
        content_was_string,
    })
}

fn rewrite_compress_text(text: &str, live_keys: &HashSet<String>) -> Option<String> {
    let arguments = parse_call_text(text)?;
    let kept: Vec<Json> = arguments
        .content
        .iter()
        .filter(|entry| entry_range_key(entry).is_some_and(|key| live_keys.contains(&key)))
        .cloned()
        .collect();
    if kept.is_empty() {
        return None;
    }
    let (serialized, _) = serialize_compacted(arguments.object, kept, arguments.content_was_string);
    Some(format!("{}{serialized}", arguments.prefix))
}

fn compact_compress_text(text: &str) -> Option<String> {
    let arguments = parse_call_text(text)?;
    let (serialized, changed) = serialize_compacted(
        arguments.object,
        arguments.content,
        arguments.content_was_string,
    );
    changed.then(|| format!("{}{serialized}", arguments.prefix))
}

/// The arguments with each entry's summary cut to a stub, keys in their
/// order, as `JSON.stringify` writes them.
fn serialize_compacted(
    mut object: Json,
    content: Vec<Json>,
    content_was_string: bool,
) -> (String, bool) {
    let mut changed = false;
    let compacted: Vec<Json> = content
        .into_iter()
        .map(|entry| {
            let (entry, stubbed) = compact_entry(entry);
            changed |= stubbed;
            entry
        })
        .collect();
    let content = Json::Array(compacted);
    // A stringified array stays a string, so replays compare the same.
    let content = if content_was_string {
        Json::String(content.to_text())
    } else {
        content
    };
    object.set("content", content);
    (object.to_text(), changed)
}

fn compact_entry(mut entry: Json) -> (Json, bool) {
    let Some(summary) = entry.get("summary").and_then(Json::as_str) else {
        return (entry, false);
    };
    if utf16_len(summary) <= SUMMARY_STUB_CHARS {
        return (entry, false);
    }
    let stub = format!("{}…", clamp_prefix(summary, SUMMARY_STUB_CHARS - 1));
    entry.set("summary", Json::String(stub));
    (entry, true)
}
