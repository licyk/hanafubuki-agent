// Derived from billion-context (https://github.com/ranxianglei/billion-context),
// `kernel/src/turn-integrity.ts` of acp-kernel 0.0.189 at commit
// `4be21b08e`. Copyright (c) 2026 ranxianglei. MIT License with an
// additional attribution term (NOTICE).

use std::collections::{HashMap, HashSet};

use crate::ids::OrderedIds;
use crate::model::{Kind, Message, Role};

/// The atomic turn groups of `messages`: a reasoning run, the assistant
/// text and tool-call burst after it, and the results of the burst's calls.
/// A burst without reasoning is a group too; user messages, system messages
/// and orphan reasoning belong to none. Groups are disjoint.
pub fn compute_turn_groups(messages: &[Message]) -> Vec<Vec<String>> {
    let mut first_result: HashMap<&str, &str> = HashMap::new();
    for message in messages
        .iter()
        .filter(|message| message.kind == Kind::ToolResult)
    {
        if let Some(call_id) = message.tool_call_id.as_deref() {
            if !message.id.is_empty() {
                first_result.entry(call_id).or_insert(message.id.as_str());
            }
        }
    }

    let mut grouped: HashSet<String> = HashSet::new();
    let mut groups = Vec::new();
    for (index, message) in messages.iter().enumerate() {
        if message.id.is_empty() || grouped.contains(&message.id) {
            continue;
        }
        if !(message.kind == Kind::Reasoning || message.is_assistant_act()) {
            continue;
        }
        let mut reasoning_start = index;
        if message.kind != Kind::Reasoning {
            while reasoning_start > 0 && messages[reasoning_start - 1].is_assistant_act() {
                reasoning_start -= 1;
            }
        }
        while reasoning_start > 0 && messages[reasoning_start - 1].kind == Kind::Reasoning {
            reasoning_start -= 1;
        }
        let mut burst_start = reasoning_start;
        while messages
            .get(burst_start)
            .is_some_and(|message| message.kind == Kind::Reasoning)
        {
            burst_start += 1;
        }
        if !messages
            .get(burst_start)
            .is_some_and(Message::is_assistant_act)
        {
            continue;
        }
        let mut burst_end = burst_start;
        while messages
            .get(burst_end + 1)
            .is_some_and(Message::is_assistant_act)
        {
            burst_end += 1;
        }

        let mut members = OrderedIds::new();
        for member in messages[reasoning_start..=burst_end]
            .iter()
            .filter(|member| !member.id.is_empty())
        {
            members.insert(&member.id);
            if member.role == Role::Assistant && member.kind == Kind::ToolCall {
                if let Some(result) = member
                    .tool_call_id
                    .as_deref()
                    .and_then(|id| first_result.get(id))
                {
                    members.insert(result);
                }
            }
        }
        let members = members.to_vec();
        grouped.extend(members.iter().cloned());
        groups.push(members);
    }
    groups
}

/// What a fold must leave visible to keep the view valid.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct IntegrityWithdrawals {
    /// In the order they were withdrawn.
    pub withdrawn: Vec<String>,
    pub split_turn_count: usize,
    pub split_pair_count: usize,
}

/// The per-message inputs of the integrity gate, built once per array.
#[derive(Clone, Debug, Default)]
pub(crate) struct IntegrityPre {
    reasoning_ids: HashSet<String>,
    call_ids: HashSet<String>,
    /// Each assistant tool call's message id and tool-call id.
    calls: Vec<(String, Option<String>)>,
    result_by_call: HashMap<String, String>,
    groups: Vec<Vec<String>>,
}

impl IntegrityPre {
    pub(crate) fn new(messages: &[Message]) -> Self {
        let mut pre = Self {
            groups: compute_turn_groups(messages),
            ..Self::default()
        };
        for message in messages.iter().filter(|message| !message.id.is_empty()) {
            if message.kind == Kind::Reasoning {
                pre.reasoning_ids.insert(message.id.clone());
            }
            if message.role == Role::Assistant && message.kind == Kind::ToolCall {
                pre.call_ids.insert(message.id.clone());
                pre.calls
                    .push((message.id.clone(), message.tool_call_id.clone()));
            }
            if message.kind == Kind::ToolResult {
                if let Some(call_id) = &message.tool_call_id {
                    pre.result_by_call
                        .entry(call_id.clone())
                        .or_insert_with(|| message.id.clone());
                }
            }
        }
        pre
    }
}

/// Given the ids a fold removes from the view, the ids that must stay
/// visible instead:
///
/// - a turn whose reasoning folds while one of its calls stays visible is
///   withdrawn whole (a kept tool call must keep its reasoning);
/// - a call and its result split by the fold are withdrawn together.
///
/// The two feed each other, so the gate repeats until nothing changes. The
/// reverse split (reasoning and text kept, the call and result folded)
/// stays allowed. An id in `invisible` is already hidden by a block: it
/// neither folds nor counts as kept.
pub fn compute_integrity_withdrawals(
    messages: &[Message],
    folded: &HashSet<String>,
    invisible: Option<&HashSet<String>>,
) -> IntegrityWithdrawals {
    withdrawals(&IntegrityPre::new(messages), folded, invisible)
}

pub(crate) fn withdrawals(
    pre: &IntegrityPre,
    folded: &HashSet<String>,
    invisible: Option<&HashSet<String>>,
) -> IntegrityWithdrawals {
    let mut remaining = folded.clone();
    let mut withdrawn = OrderedIds::new();
    let mut handled_turns: HashSet<usize> = HashSet::new();
    let mut handled_pairs: HashSet<&str> = HashSet::new();
    let is_invisible = |id: &str| invisible.is_some_and(|invisible| invisible.contains(id));

    let mut changed = true;
    while changed {
        changed = false;
        for (index, group) in pre.groups.iter().enumerate() {
            if handled_turns.contains(&index) {
                continue;
            }
            let fold_has_reasoning = group
                .iter()
                .any(|id| remaining.contains(id) && pre.reasoning_ids.contains(id));
            let kept_has_call = group.iter().any(|id| {
                !remaining.contains(id) && pre.call_ids.contains(id) && !is_invisible(id)
            });
            if !fold_has_reasoning || !kept_has_call {
                continue;
            }
            handled_turns.insert(index);
            for id in group {
                remaining.remove(id);
                withdrawn.insert(id);
            }
            changed = true;
        }
        for (message_id, call_id) in &pre.calls {
            let Some(call_id) = call_id.as_deref() else {
                continue;
            };
            if handled_pairs.contains(call_id) {
                continue;
            }
            let Some(result_id) = pre.result_by_call.get(call_id) else {
                continue;
            };
            if remaining.contains(message_id) == remaining.contains(result_id) {
                continue;
            }
            handled_pairs.insert(call_id);
            remaining.remove(message_id);
            remaining.remove(result_id);
            withdrawn.insert(message_id);
            withdrawn.insert(result_id);
            changed = true;
        }
    }

    IntegrityWithdrawals {
        withdrawn: withdrawn.to_vec(),
        split_turn_count: handled_turns.len(),
        split_pair_count: handled_pairs.len(),
    }
}
