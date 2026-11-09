// Derived from billion-context (https://github.com/ranxianglei/billion-context),
// `kernel/src/prune.ts` of acp-kernel 0.0.189 at commit `4be21b08e`.
// Copyright (c) 2026 ranxianglei. MIT License with an additional attribution
// term (NOTICE).
// Modified for hanafubuki-agent: ids are compared exactly (no `base#part`
// sub-ids), coverage is by id only (no ledger intervals), the first-user pin
// is one message, and there is no head placement for handoff views.

mod hide;
mod sync;

use std::collections::{HashMap, HashSet};

pub use hide::{hide_consumed_compress_calls, HiddenCompressCalls};
pub use sync::sync_blocks;

use crate::model::{Kind, Message, Role, State};
use crate::protect::COMPRESS_TOOL_NAME;

/// The first line of a rendered block summary.
pub const SUMMARY_HEADER: &str = "[Compressed conversation section]";

const SUMMARY_ID_PREFIX: &str = "acp_summary_";

/// The id of a block's rendered summary in a view; never recorded as
/// coverage.
pub fn summary_message_id(block_id: &str) -> String {
    format!("{SUMMARY_ID_PREFIX}{block_id}")
}

pub(crate) fn is_summary_message_id(id: &str) -> bool {
    id.starts_with(SUMMARY_ID_PREFIX)
}

/// A summary as [`prune`] renders it; a host message that only reuses the
/// id prefix is content.
pub fn is_rendered_summary(message: &Message) -> bool {
    is_summary_message_id(&message.id) && message.role == Role::System && message.kind == Kind::Text
}

/// The view of `messages` under the active blocks: covered messages leave,
/// and each block whose `compress` call is not in the view is rendered as a
/// system text at its anchor. The first user message always stays, and no
/// summary goes before it; an anchor never splits an assistant run or a
/// tool call from its result. Tool calls without a later result (but
/// `compress` calls), results without an earlier call, and reasoning runs
/// without their assistant act are dropped.
pub fn prune(messages: &[Message], state: &State) -> Vec<Message> {
    let covered = state.covered_message_ids();
    if covered.is_empty() {
        return messages.to_vec();
    }
    let first_user = messages
        .iter()
        .position(|message| message.role == Role::User);
    // An anchor whose messages are all gone parks before the newest user
    // message, never at the tail, where the model answered the stale recap
    // instead of the question.
    let fallback = messages
        .iter()
        .rposition(|message| message.role == Role::User)
        .unwrap_or(messages.len());

    let mut first_index: HashMap<&str, usize> = HashMap::new();
    let mut summary_index: HashMap<&str, usize> = HashMap::new();
    for (index, message) in messages.iter().enumerate() {
        first_index.entry(message.id.as_str()).or_insert(index);
        if is_rendered_summary(message) {
            summary_index.insert(message.id.as_str(), index);
        }
    }
    let visible_compress_calls: HashSet<&str> = messages
        .iter()
        .filter(|message| {
            message.kind == Kind::ToolCall && message.tool_name_is(COMPRESS_TOOL_NAME)
        })
        .filter_map(|message| message.tool_call_id.as_deref())
        .collect();
    let anchors = collect_summary_anchors(
        state,
        &first_index,
        &summary_index,
        fallback,
        &visible_compress_calls,
    );
    // Copies of every summary this session rendered, active or consumed,
    // give way to the fresh render.
    let known_summary_ids: HashSet<String> = state
        .blocks
        .iter()
        .map(|block| summary_message_id(&block.block_id))
        .collect();

    let rebuilt = rebuild(messages, &covered, first_user, anchors, &known_summary_ids);
    strip_orphaned_reasoning(strip_orphaned_tool_results(strip_orphaned_tool_calls(
        rebuilt,
    )))
}

struct SummaryAnchor {
    block_id: String,
    summary: String,
    topic: Option<String>,
    insert_at: usize,
    /// None of the block's messages is in the view.
    unresolved: bool,
}

fn collect_summary_anchors(
    state: &State,
    first_index: &HashMap<&str, usize>,
    summary_index: &HashMap<&str, usize>,
    fallback: usize,
    visible_compress_calls: &HashSet<&str>,
) -> Vec<SummaryAnchor> {
    let mut anchors = Vec::new();
    for block in state.active_blocks() {
        // The visible call's arguments already carry the summary.
        if block
            .compress_call_id
            .as_deref()
            .is_some_and(|call_id| visible_compress_calls.contains(call_id))
        {
            continue;
        }
        let anchor = |insert_at, unresolved| SummaryAnchor {
            block_id: block.block_id.clone(),
            summary: block.summary.clone(),
            topic: block.topic.clone(),
            insert_at,
            unresolved,
        };
        // A summary a previous view rendered keeps its place.
        if let Some(existing) = summary_index.get(summary_message_id(&block.block_id).as_str()) {
            anchors.push(anchor(*existing, false));
            continue;
        }
        // The block's own fold, not what it inherited: anchoring on the
        // inherited ids would move the summary back over cached bytes.
        let ids = if block.direct_message_ids.is_empty() {
            &block.effective_message_ids
        } else {
            &block.direct_message_ids
        };
        let earliest = ids
            .iter()
            .filter_map(|id| first_index.get(id.as_str()).copied())
            .min();
        anchors.push(anchor(earliest.unwrap_or(fallback), earliest.is_none()));
    }
    anchors.sort_by_key(|anchor| anchor.insert_at);
    anchors
}

fn rebuild(
    messages: &[Message],
    covered: &HashSet<&str>,
    first_user: Option<usize>,
    anchors: Vec<SummaryAnchor>,
    known_summary_ids: &HashSet<String>,
) -> Vec<Message> {
    let pairing = (!anchors.is_empty()).then(|| AnchorPairing::new(messages));
    let mut anchors: Vec<SummaryAnchor> = anchors
        .into_iter()
        .map(|mut anchor| {
            if let Some(pairing) = &pairing {
                anchor.insert_at = pairing.safe_index(messages, anchor.insert_at);
            }
            // No summary goes before the pinned first user message: it is the
            // provider's cache head. An unresolved anchor may sit right
            // before it.
            if let Some(first_user) = first_user {
                let floor = if anchor.unresolved {
                    first_user
                } else {
                    first_user + 1
                };
                anchor.insert_at = anchor.insert_at.max(floor);
            }
            anchor
        })
        .collect();
    anchors.sort_by_key(|anchor| anchor.insert_at);

    let mut pending = anchors.into_iter().peekable();
    let mut result = Vec::with_capacity(messages.len());
    for (index, message) in messages.iter().enumerate() {
        while let Some(anchor) = pending.next_if(|anchor| anchor.insert_at == index) {
            result.push(render_summary(&anchor));
        }
        // The pin comes before coverage: strict providers reject a
        // conversation without a user message.
        if Some(index) == first_user {
            result.push(message.clone());
            continue;
        }
        if covered.contains(message.id.as_str()) {
            continue;
        }
        if is_rendered_summary(message) && known_summary_ids.contains(&message.id) {
            continue;
        }
        result.push(message.clone());
    }
    result.extend(pending.map(|anchor| render_summary(&anchor)));
    result
}

fn render_summary(anchor: &SummaryAnchor) -> Message {
    let body = anchor.summary.trim();
    let topic_line = match &anchor.topic {
        Some(topic) if !topic.is_empty() => format!("{SUMMARY_HEADER} — {topic}"),
        _ => SUMMARY_HEADER.to_owned(),
    };
    let text = if body.is_empty() {
        topic_line
    } else {
        format!("{topic_line}\n{body}")
    };
    Message::text(summary_message_id(&anchor.block_id), Role::System, text)
}

/// Where a summary may go: never inside an assistant run (consecutive
/// assistant messages merge into one wire message, and its tool calls would
/// lose their reasoning) and never between a tool call and its result.
struct AnchorPairing {
    /// The start of the assistant run holding each index.
    run_start: Vec<Option<usize>>,
    /// For each index, the furthest `result + 1` of the calls before it.
    prefix_end: Vec<usize>,
}

impl AnchorPairing {
    fn new(messages: &[Message]) -> Self {
        let mut first_result: HashMap<&str, usize> = HashMap::new();
        for (index, message) in messages.iter().enumerate() {
            if message.kind == Kind::ToolResult {
                if let Some(call_id) = message.tool_call_id.as_deref() {
                    first_result.entry(call_id).or_insert(index);
                }
            }
        }
        let mut run_start = vec![None; messages.len()];
        let mut start = None;
        for (index, message) in messages.iter().enumerate() {
            if message.role == Role::Assistant {
                let run = *start.get_or_insert(index);
                run_start[index] = Some(run);
            } else {
                start = None;
            }
        }
        let mut prefix_end = vec![0; messages.len() + 1];
        for (index, message) in messages.iter().enumerate() {
            let end = match (message.role, message.kind, message.tool_call_id.as_deref()) {
                (Role::Assistant, Kind::ToolCall, Some(call_id)) => {
                    first_result.get(call_id).map_or(0, |result| result + 1)
                }
                _ => 0,
            };
            prefix_end[index + 1] = prefix_end[index].max(end);
        }
        Self {
            run_start,
            prefix_end,
        }
    }

    fn safe_index(&self, messages: &[Message], index: usize) -> usize {
        let mut safe = index;
        if safe > 0 && safe < messages.len() && messages[safe - 1].role == Role::Assistant {
            if let Some(run_start) = self.run_start[safe] {
                safe = run_start;
            }
        }
        if safe > messages.len() {
            return safe;
        }
        // Each move lands just past a result, so this reaches a fixed point.
        for _ in 0..messages.len() {
            let next = safe.max(self.prefix_end[safe]);
            if next == safe {
                break;
            }
            safe = next;
        }
        safe
    }
}

/// Drops results with no call of the same id before them. Pairing is by
/// order, so a re-issued call id cannot keep an orphan alive.
fn strip_orphaned_tool_results(messages: Vec<Message>) -> Vec<Message> {
    let mut open: HashSet<String> = HashSet::new();
    let mut keep = vec![true; messages.len()];
    for (index, message) in messages.iter().enumerate() {
        let Some(call_id) = &message.tool_call_id else {
            continue;
        };
        match message.kind {
            Kind::ToolCall => {
                open.insert(call_id.clone());
            }
            Kind::ToolResult => keep[index] = open.contains(call_id),
            Kind::Text | Kind::Reasoning => {}
        }
    }
    retain_marked(messages, &keep)
}

/// Drops calls with no result of the same id after them; `compress` calls
/// stay, their results managed by [`hide_consumed_compress_calls`].
fn strip_orphaned_tool_calls(messages: Vec<Message>) -> Vec<Message> {
    let mut answered: HashSet<String> = HashSet::new();
    let mut keep = vec![true; messages.len()];
    for (index, message) in messages.iter().enumerate().rev() {
        let Some(call_id) = &message.tool_call_id else {
            continue;
        };
        match message.kind {
            Kind::ToolResult => {
                answered.insert(call_id.clone());
            }
            Kind::ToolCall if !message.tool_name_is(COMPRESS_TOOL_NAME) => {
                keep[index] = answered.contains(call_id);
            }
            _ => {}
        }
    }
    retain_marked(messages, &keep)
}

/// Drops a reasoning run not followed by its assistant text or tool call:
/// strict thinking providers reject reasoning that lost its response.
fn strip_orphaned_reasoning(messages: Vec<Message>) -> Vec<Message> {
    let mut keep = vec![true; messages.len()];
    let mut index = 0;
    while index < messages.len() {
        if messages[index].kind != Kind::Reasoning {
            index += 1;
            continue;
        }
        let mut end = index;
        while messages
            .get(end + 1)
            .is_some_and(|next| next.kind == Kind::Reasoning)
        {
            end += 1;
        }
        if !messages.get(end + 1).is_some_and(Message::is_assistant_act) {
            keep[index..=end].fill(false);
        }
        index = end + 1;
    }
    retain_marked(messages, &keep)
}

fn retain_marked(messages: Vec<Message>, keep: &[bool]) -> Vec<Message> {
    messages
        .into_iter()
        .zip(keep)
        .filter_map(|(message, keep)| keep.then_some(message))
        .collect()
}
