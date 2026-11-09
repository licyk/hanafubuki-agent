// Derived from billion-context (https://github.com/ranxianglei/billion-context),
// `kernel/src/tool-pairs.ts`, `kernel/src/reasoning-pairs.ts` and the pair
// widening of `kernel/src/unit-projection.ts` of acp-kernel 0.0.189 at
// commit `4be21b08e`. Copyright (c) 2026 ranxianglei. MIT License with an
// additional attribution term (NOTICE).

use std::collections::{HashMap, HashSet};

use crate::boundaries::{BoundaryKind, ResolvedRange};
use crate::model::{Kind, Message};
use crate::protect::COMPRESS_TOOL_NAME;

/// Each tool-call id's lowest and highest position in `messages`: pairing is
/// by id, whatever lies between a call and its result, and a duplicated id
/// widens to its extremes.
pub(crate) fn tool_pair_extremes(messages: &[Message]) -> HashMap<String, (usize, usize)> {
    let mut extremes: HashMap<String, (usize, usize)> = HashMap::new();
    for (index, message) in messages.iter().enumerate() {
        let Some(call_id) = message.tool_call_id.as_ref().filter(|id| !id.is_empty()) else {
            continue;
        };
        extremes
            .entry(call_id.clone())
            .and_modify(|(low, high)| {
                *low = (*low).min(index);
                *high = (*high).max(index);
            })
            .or_insert((index, index));
    }
    extremes
}

/// Widens `start..=end` until no tool call or result in it has its other
/// half outside, so a pair folds together. `compress` exchanges do not
/// widen.
pub fn adjust_boundaries_for_tool_pairs(
    start: usize,
    end: usize,
    messages: &[Message],
) -> (usize, usize) {
    widen_for_tool_pairs(start, end, messages, &tool_pair_extremes(messages))
}

pub(crate) fn widen_for_tool_pairs(
    start: usize,
    end: usize,
    messages: &[Message],
    extremes: &HashMap<String, (usize, usize)>,
) -> (usize, usize) {
    let (mut start, mut end) = (start, end);
    let mut settled: HashSet<&str> = HashSet::new();
    let mut changed = true;
    while changed {
        changed = false;
        let mut index = start;
        while index <= end && index < messages.len() {
            let message = &messages[index];
            index += 1;
            let Some(call_id) = message.tool_call_id.as_deref().filter(|id| !id.is_empty()) else {
                continue;
            };
            if settled.contains(call_id) || message.tool_name_is(COMPRESS_TOOL_NAME) {
                continue;
            }
            settled.insert(call_id);
            let Some((low, high)) = extremes.get(call_id) else {
                continue;
            };
            if *low < start {
                start = *low;
                changed = true;
            }
            if *high > end {
                end = *high;
                changed = true;
            }
        }
    }
    (start, end)
}

/// Widens `start..=end` so a reasoning run and the assistant burst after it
/// fold together: a reasoning message pulls in the whole burst, an
/// assistant text or tool call pulls in the reasoning run before it.
pub fn adjust_boundaries_for_reasoning_pairs(
    start: usize,
    end: usize,
    messages: &[Message],
) -> (usize, usize) {
    if start > end {
        return (start, end);
    }
    let (mut new_start, mut new_end) = (start, end);
    for index in start..=end.min(messages.len().saturating_sub(1)) {
        let Some(message) = messages.get(index) else {
            break;
        };
        if message.kind == Kind::Reasoning {
            let mut run_end = index;
            while messages
                .get(run_end + 1)
                .is_some_and(|next| next.kind == Kind::Reasoning)
            {
                run_end += 1;
            }
            if messages
                .get(run_end + 1)
                .is_some_and(Message::is_assistant_act)
            {
                let mut burst_end = run_end + 1;
                while messages
                    .get(burst_end + 1)
                    .is_some_and(Message::is_assistant_act)
                {
                    burst_end += 1;
                }
                new_end = new_end.max(burst_end);
            }
        }
        if message.is_assistant_act() {
            let mut run_start = index;
            while run_start > 0 && messages[run_start - 1].kind == Kind::Reasoning {
                run_start -= 1;
            }
            if run_start < index && run_start < new_start {
                new_start = run_start;
            }
        }
    }
    (new_start, new_end)
}

/// A message range widened for reasoning and tool pairs to a fixed point
/// (each can pull in what the other then widens for); a block range is left
/// as resolved.
pub(crate) fn widened_range_ids(
    resolved: &ResolvedRange,
    messages: &[Message],
    extremes: &HashMap<String, (usize, usize)>,
) -> Vec<String> {
    if resolved.boundary_kind == BoundaryKind::Block {
        return resolved.message_ids.clone();
    }
    let (mut start, mut end) = (resolved.start_index, resolved.end_index);
    for _ in 0..2 {
        let (reasoning_start, reasoning_end) =
            adjust_boundaries_for_reasoning_pairs(start, end, messages);
        let (tool_start, tool_end) =
            widen_for_tool_pairs(reasoning_start, reasoning_end, messages, extremes);
        let changed = tool_start != start || tool_end != end;
        start = tool_start;
        end = tool_end;
        if !changed {
            break;
        }
    }
    if start == resolved.start_index && end == resolved.end_index {
        return resolved.message_ids.clone();
    }
    messages
        .iter()
        .take(end + 1)
        .skip(start)
        .map(|message| message.id.clone())
        .collect()
}
