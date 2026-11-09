// Derived from billion-context (https://github.com/ranxianglei/billion-context),
// `kernel/src/decompress.ts` of acp-kernel 0.0.189 at commit `4be21b08e`.
// Copyright (c) 2026 ranxianglei. MIT License with an additional attribution
// term (NOTICE).
// Modified for hanafubuki-agent: the state is updated in place, and a
// permanent expansion is one call (`expand_block`), since `sync_blocks`
// reactivates any block not flagged `expanded`.

use std::collections::{HashSet, VecDeque};

use crate::boundaries::resolve_block_span;
use crate::model::{Block, Kind, Message, State};
use crate::prune::SUMMARY_HEADER;
use crate::text::{clamp_prefix, utf16_len};

const MAX_PREVIEW_UNITS: usize = 2000;
const MAX_PREVIEW_UNITS_PER_MESSAGE: usize = 200;

/// A block's content as text, the block left folded.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CollectedContent {
    /// Empty when `count` is 0.
    pub text: String,
    /// The messages rendered, plus the nested summaries one tier up.
    pub count: usize,
}

/// The messages a decompression brought back into the view.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RestoredPreview {
    pub preview: String,
    pub restored_count: usize,
}

/// A block flagged restored inline, with its span for the host's re-fold
/// hint when one can be derived.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InlineRestore {
    pub block_id: String,
    pub restored_start_ref: Option<String>,
    pub restored_end_ref: Option<String>,
}

/// A block id as the model may write it: `b5`, `b005` or `5`.
pub fn parse_block_id_arg(arg: &str) -> Option<String> {
    let normalized = arg.trim().to_lowercase();
    if let Some(digits) = normalized.strip_prefix('b') {
        if is_ascii_number(digits) {
            let significant = digits.trim_start_matches('0');
            let kept = if significant.is_empty() {
                "0"
            } else {
                significant
            };
            return Some(format!("b{kept}"));
        }
        return None;
    }
    is_ascii_number(&normalized).then(|| format!("b{normalized}"))
}

/// The active blocks covering any of `message_ids`, by block number.
pub fn find_blocks_overlapping_messages<'a>(
    state: &'a State,
    message_ids: &HashSet<&str>,
) -> Vec<&'a Block> {
    let mut matched: Vec<&Block> = state
        .active_blocks()
        .filter(|block| {
            block
                .effective_message_ids
                .iter()
                .any(|id| message_ids.contains(id.as_str()))
        })
        .collect();
    matched.sort_by_key(|block| block_number(&block.block_id));
    matched
}

/// The nearest active block among those `block_id` consumed, searched
/// breadth first down its lineage.
pub fn find_active_ancestor(state: &State, block_id: &str) -> Option<String> {
    let start = state.block(block_id)?;
    let mut queue: VecDeque<&str> = start.direct_block_ids.iter().map(String::as_str).collect();
    let mut visited: HashSet<&str> = HashSet::new();
    while let Some(current_id) = queue.pop_front() {
        if !visited.insert(current_id) {
            continue;
        }
        let Some(current) = state.block(current_id) else {
            continue;
        };
        if current.active {
            return Some(current.block_id.clone());
        }
        queue.extend(current.direct_block_ids.iter().map(String::as_str));
    }
    None
}

/// Deactivates `block_ids`; `deep` also every block they consumed, down
/// the whole lineage.
pub fn deactivate_blocks(state: &mut State, block_ids: &[&str], deep: bool) {
    for block in &mut state.blocks {
        if block_ids.contains(&block.block_id.as_str()) {
            block.active = false;
        }
    }
    if !deep {
        return;
    }
    let mut queue: VecDeque<String> = block_ids
        .iter()
        .filter_map(|id| state.block(id))
        .flat_map(|block| block.direct_block_ids.iter().cloned())
        .collect();
    let mut visited: HashSet<String> = HashSet::new();
    while let Some(id) = queue.pop_front() {
        if !visited.insert(id.clone()) {
            continue;
        }
        for block in state.blocks.iter_mut().filter(|block| block.block_id == id) {
            queue.extend(block.direct_block_ids.iter().cloned());
            block.active = false;
        }
    }
}

/// Expands a block for good: it and its lineage go inactive and its
/// messages return to the view. False when there is no such block.
pub fn expand_block(state: &mut State, block_id: &str) -> bool {
    if state.block(block_id).is_none() {
        return false;
    }
    deactivate_blocks(state, &[block_id], true);
    if let Some(block) = state.block_mut(block_id) {
        block.expanded = true;
    }
    true
}

/// The messages that were covered before (`before_active_ids`) and no
/// active block covers now, one line each, in history order.
pub fn build_restored_content_preview(
    messages: &[Message],
    before_active_ids: &HashSet<&str>,
    state: &State,
) -> RestoredPreview {
    let covered = state.covered_message_ids();
    let restored: Vec<&Message> = messages
        .iter()
        .filter(|message| before_active_ids.contains(message.id.as_str()))
        .filter(|message| !covered.contains(message.id.as_str()))
        .collect();
    if restored.is_empty() {
        return RestoredPreview::default();
    }

    let mut lines = Vec::new();
    let mut total_units = 0;
    for message in &restored {
        if total_units >= MAX_PREVIEW_UNITS {
            break;
        }
        let truncated = if utf16_len(&message.text) > MAX_PREVIEW_UNITS_PER_MESSAGE {
            format!(
                "{}...",
                clamp_prefix(&message.text, MAX_PREVIEW_UNITS_PER_MESSAGE)
            )
        } else {
            message.text.clone()
        };
        let label = match tool_label(message) {
            Some(tool) => format!("{tool}: {truncated}"),
            _ => format!("[{}] {truncated}", message.role.name()),
        };
        total_units += utf16_len(&label) + 1;
        lines.push(label);
    }
    RestoredPreview {
        preview: lines.join("\n"),
        restored_count: restored.len(),
    }
}

/// A block's content as readable text, the state untouched, so the prompt
/// cache survives: the block stays folded and the host returns the text as
/// a tool result. One tier up (`full` false), nested active blocks show as
/// their summaries and the block's own messages in full; `full` renders
/// every covered message.
pub fn collect_block_content(
    state: &State,
    block: &Block,
    messages: &[Message],
    full: bool,
) -> CollectedContent {
    let target: HashSet<&str> = block
        .effective_message_ids
        .iter()
        .map(String::as_str)
        .collect();

    if full {
        let parts: Vec<String> = messages
            .iter()
            .filter(|message| target.contains(message.id.as_str()))
            .map(format_message)
            .collect();
        return content(parts);
    }

    let nested: Vec<&Block> = block
        .direct_block_ids
        .iter()
        .filter_map(|id| state.block(id))
        .filter(|child| child.active)
        .collect();
    let nested_covered: HashSet<&str> = nested
        .iter()
        .flat_map(|child| child.effective_message_ids.iter().map(String::as_str))
        .collect();
    let mut parts: Vec<String> = nested
        .iter()
        .map(|child| {
            let label = match &child.topic {
                Some(topic) if !topic.is_empty() => format!("{}: {topic}", child.block_id),
                _ => child.block_id.clone(),
            };
            format!("{SUMMARY_HEADER} — {label}\n{}", child.summary)
        })
        .collect();
    parts.extend(
        messages
            .iter()
            .filter(|message| target.contains(message.id.as_str()))
            .filter(|message| !nested_covered.contains(message.id.as_str()))
            .map(format_message),
    );
    content(parts)
}

/// Flags a block whose content was restored into the live conversation, so
/// a `compress` over the same span updates it in place. It stays active and
/// no ref moves. `None` when there is no such block.
pub fn mark_block_restored_inline(state: &mut State, block_id: &str) -> Option<InlineRestore> {
    let span = resolve_block_span(state.block(block_id)?, &state.message_refs.by_raw);
    if let Some(block) = state.block_mut(block_id) {
        block.restored_inline = true;
    }
    let (restored_start_ref, restored_end_ref) = span.unzip();
    Some(InlineRestore {
        block_id: block_id.to_owned(),
        restored_start_ref,
        restored_end_ref,
    })
}

/// Every active block above `block_id`, nearest generation first: the
/// blocks whose lineage consumed it, climbing through inactive ones. A
/// refold needs none above the restored block.
pub fn active_ancestor_ids(state: &State, block_id: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut visited: HashSet<&str> = HashSet::from([block_id]);
    let mut frontier: Vec<&str> = vec![block_id];
    while !frontier.is_empty() {
        let mut next = Vec::new();
        for block in &state.blocks {
            if visited.contains(block.block_id.as_str())
                || !block
                    .direct_block_ids
                    .iter()
                    .any(|id| frontier.contains(&id.as_str()))
            {
                continue;
            }
            visited.insert(block.block_id.as_str());
            if block.active {
                out.push(block.block_id.clone());
            }
            next.push(block.block_id.as_str());
        }
        frontier = next;
    }
    out
}

fn content(parts: Vec<String>) -> CollectedContent {
    CollectedContent {
        count: parts.len(),
        text: parts.join("\n\n"),
    }
}

fn format_message(message: &Message) -> String {
    match tool_label(message) {
        Some(tool) => format!("[{} • {tool}]\n{}", message.role.name(), message.text),
        None => format!("[{}]\n{}", message.role.name(), message.text),
    }
}

/// The tool a message is labelled with: a tool part's own name.
fn tool_label(message: &Message) -> Option<&str> {
    message
        .tool_name
        .as_deref()
        .filter(|name| !name.is_empty() && message.kind != Kind::Text)
}

fn is_ascii_number(text: &str) -> bool {
    !text.is_empty() && text.bytes().all(|byte| byte.is_ascii_digit())
}

fn block_number(block_id: &str) -> u64 {
    block_id
        .strip_prefix('b')
        .filter(|digits| is_ascii_number(digits))
        .and_then(|digits| digits.parse().ok())
        .unwrap_or(0)
}
