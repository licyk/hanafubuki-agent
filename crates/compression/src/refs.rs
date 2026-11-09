// Derived from billion-context (https://github.com/ranxianglei/billion-context),
// `kernel/src/refs.ts` and the assign-refs step of `kernel/src/compress.ts`
// of acp-kernel 0.0.189 at commit `4be21b08e`. Copyright (c) 2026
// ranxianglei. MIT License with an additional attribution term (NOTICE).
// Modified for hanafubuki-agent: no ledger-sequence refs and no one-time
// restamp onto a storage ledger; the host's protection is always consulted
// (billion-context's hosts always install a protection hook).

use crate::model::{Message, RefMap, State};
use crate::protect::{
    collect_failed_compress_call_ids, collect_latest_protected, collect_protected_tool_call_ids,
    is_message_latest_protected, is_message_protected, is_message_protected_with_pairing,
};
use crate::Config;

/// The ref of a protected message: it takes no number.
pub const BLOCKED_REF: &str = "BLOCKED";

const MIN_INDEX: u32 = 1;
const MAX_INDEX: u32 = 9_999_999;

/// `m` and the index, zero-padded to at least five digits; `None` outside
/// `1..=9_999_999`.
pub fn index_to_ref(index: u32) -> Option<String> {
    (MIN_INDEX..=MAX_INDEX)
        .contains(&index)
        .then(|| format!("m{index:05}"))
}

/// The index of a message ref, case and padding ignored.
pub fn ref_to_index(r#ref: &str) -> Option<u32> {
    let normalized = r#ref.trim().to_lowercase();
    let digits = normalized.strip_prefix('m')?;
    parse_padded_index(digits)
}

/// Digits with any leading zeros, at most seven significant.
pub(crate) fn parse_padded_index(digits: &str) -> Option<u32> {
    if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let significant = digits.trim_start_matches('0');
    if significant.is_empty() || significant.len() > 7 {
        return None;
    }
    significant
        .parse()
        .ok()
        .filter(|index| (MIN_INDEX..=MAX_INDEX).contains(index))
}

/// A ref pair in ascending order for display; a pair with a side that is
/// not a message ref is returned as given.
pub fn ordered_ref_pair<'a>(start: &'a str, end: &'a str) -> (&'a str, &'a str) {
    match (ref_to_index(start), ref_to_index(end)) {
        (Some(a), Some(b)) if b < a => (end, start),
        _ => (start, end),
    }
}

pub fn highest_used_index(map: &RefMap) -> u32 {
    map.by_raw
        .values()
        .filter(|r#ref| *r#ref != BLOCKED_REF)
        .filter_map(|r#ref| ref_to_index(r#ref))
        .max()
        .unwrap_or(0)
}

#[derive(Clone, Debug, PartialEq)]
pub struct AssignedRefs {
    pub map: RefMap,
    pub next_index: u32,
    pub newly_assigned: usize,
    /// Ids whose `BLOCKED` ref became a number this pass.
    pub migrated_ids: Vec<String>,
}

/// Gives every message without a ref one, in array order: a protected
/// message gets `BLOCKED`, any other the next free number from
/// `next_index`. A message keeps its ref forever, except that a `BLOCKED`
/// one takes a fresh number once no message with its id is protected.
pub fn assign_refs(
    messages: &[Message],
    existing: &RefMap,
    next_index: u32,
    is_protected: impl Fn(&Message) -> bool,
) -> AssignedRefs {
    let mut map = existing.clone();
    let mut cursor = next_index.max(MIN_INDEX);
    let mut newly_assigned = 0;
    let mut migrated_ids = Vec::new();

    // A BLOCKED binding is shared by every message with that id, while
    // protection is decided per message: it migrates only when no sibling
    // is still protected, or a protected sibling's sent bytes would change.
    let sibling_protected: std::collections::HashSet<&str> = messages
        .iter()
        .filter(|message| !message.id.is_empty() && is_protected(message))
        .map(|message| message.id.as_str())
        .collect();

    for message in messages.iter().filter(|message| !message.id.is_empty()) {
        let protected_now = is_protected(message);
        if let Some(previous) = map.by_raw.get(&message.id) {
            if previous == BLOCKED_REF
                && !protected_now
                && !sibling_protected.contains(message.id.as_str())
            {
                map.by_raw.remove(&message.id);
                migrated_ids.push(message.id.clone());
            } else {
                continue;
            }
        }
        if protected_now {
            map.by_raw
                .insert(message.id.clone(), BLOCKED_REF.to_owned());
            continue;
        }
        let Some((index, r#ref)) = free_ref(&map, cursor) else {
            continue;
        };
        cursor = index + 1;
        map.by_raw.insert(message.id.clone(), r#ref.clone());
        map.by_ref.insert(r#ref, message.id.clone());
        newly_assigned += 1;
    }

    AssignedRefs {
        map,
        next_index: cursor,
        newly_assigned,
        migrated_ids,
    }
}

/// The view's step that mints refs: protection is the host's flag, the
/// protected tools (with their results paired when a path pattern is
/// configured) and the latest-only tools. A ref that stops being blocked
/// keeps its message untagged until the next fold, so no quiet request
/// rewrites an old message.
pub fn assign_message_refs(state: &mut State, messages: &[Message], config: &Config) {
    let latest = collect_latest_protected(messages, config);
    let failed = collect_failed_compress_call_ids(messages);
    let has_path_patterns = config
        .protected_tools
        .iter()
        .any(|pattern| pattern.contains('/'));
    let path_protected =
        has_path_patterns.then(|| collect_protected_tool_call_ids(messages, config));
    let is_protected = |message: &Message| {
        let protected = match &path_protected {
            Some(call_ids) => is_message_protected_with_pairing(message, config, call_ids, &failed),
            None => is_message_protected(message, config, &failed),
        };
        protected || is_message_latest_protected(message, &latest)
    };
    let assigned = assign_refs(
        messages,
        &state.message_refs,
        highest_used_index(&state.message_refs) + 1,
        is_protected,
    );
    state.message_refs = assigned.map;
    for id in &assigned.migrated_ids {
        let Some(r#ref) = state.message_refs.by_raw.get(id) else {
            continue;
        };
        if r#ref != BLOCKED_REF && !state.render_deferred_refs.contains(r#ref) {
            state.render_deferred_refs.push(r#ref.clone());
        }
    }
}

fn free_ref(map: &RefMap, start: u32) -> Option<(u32, String)> {
    (start.max(MIN_INDEX)..=MAX_INDEX).find_map(|candidate| {
        let r#ref = index_to_ref(candidate)?;
        (!map.by_ref.contains_key(&r#ref)).then_some((candidate, r#ref))
    })
}
