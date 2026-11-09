// Derived from billion-context (https://github.com/ranxianglei/billion-context),
// `src/fork-adoption.ts` (`planForkAdoption`, `applyForkAdoption`) and
// `reserveRefsThrough` in `src/session.ts` at commit `4be21b08e`.
// Copyright (c) 2026 ranxianglei. MIT License with an additional attribution
// term (NOTICE).
// Modified for hanafubuki-agent: one call plans and seeds a fresh state;
// every ref of a present message is kept (the kernel's `seedAllRefs`) and
// blocks are always adopted; a present message's first-sight stamp rides
// along (billion-context's fork clones its store rows, which carry the time);
// no content store, block contents or lineage metadata do.

use std::collections::{BTreeSet, HashMap, HashSet};

use crate::model::{Block, State};
use crate::refs::{highest_used_index, index_to_ref, BLOCKED_REF};

/// The raw id that holds the parent's highest ref in a fork's ref map, so
/// a fresh message never takes a number the parent gave a message the fork
/// dropped. It names no message.
pub const REF_FLOOR_ID: &str = "fork:ref-floor";

/// The state a fork of `parent` starts with, given the ids of the messages
/// its history holds. A block is adopted, active, when every message it
/// covers is present, unless it was expanded or another block consumed it;
/// a tier block brings the blocks it consumed as the inactive records they
/// are. Every present message keeps its ref, its frozen token count and its
/// first-sight stamp, and fresh refs number above the parent's. The orphan-hiding, truncation,
/// render-deferral and view latches carry over, so the shared history is
/// sent as before; the nudge, the stats, the streak, absorbed records and
/// rules start fresh. The parent is not changed.
pub fn adopt_fork_state(parent: &State, present: &HashSet<&str>) -> State {
    let by_id: HashMap<&str, &Block> = parent
        .blocks
        .iter()
        .map(|block| (block.block_id.as_str(), block))
        .collect();
    let consumed: HashSet<&str> = parent
        .blocks
        .iter()
        .flat_map(|block| block.direct_block_ids.iter().map(String::as_str))
        .collect();
    let adopted: HashSet<&str> = parent
        .blocks
        .iter()
        .filter(|block| !block.expanded && !consumed.contains(block.block_id.as_str()))
        .filter(|block| {
            !block.effective_message_ids.is_empty()
                && block
                    .effective_message_ids
                    .iter()
                    .all(|id| present.contains(id.as_str()))
        })
        .map(|block| block.block_id.as_str())
        .collect();
    let mut closure = BTreeSet::new();
    let mut queue: Vec<&str> = adopted.iter().copied().collect();
    while let Some(id) = queue.pop() {
        if closure.insert(id) {
            if let Some(block) = by_id.get(id) {
                queue.extend(block.direct_block_ids.iter().map(String::as_str));
            }
        }
    }

    let mut state = State {
        blocks: parent
            .blocks
            .iter()
            .filter(|block| closure.contains(block.block_id.as_str()))
            .map(|block| Block {
                active: adopted.contains(block.block_id.as_str()),
                ..block.clone()
            })
            .collect(),
        next_block_id: parent.next_block_id.max(1),
        next_run_id: parent.next_run_id.max(1),
        hidden_orphan_refs: parent.hidden_orphan_refs.clone(),
        swept_orphan_call_ids: parent.swept_orphan_call_ids.clone(),
        emergency_truncate_latch: parent.emergency_truncate_latch.clone(),
        render_deferred_refs: parent.render_deferred_refs.clone(),
        view_latches: parent.view_latches.clone(),
        view_latch_config_digest: parent.view_latch_config_digest.clone(),
        ..State::default()
    };
    state.ts_stamps = parent
        .ts_stamps
        .iter()
        .filter(|(id, _)| present.contains(id.as_str()))
        .map(|(id, stamp)| (id.clone(), *stamp))
        .collect();
    for (id, r#ref) in &parent.message_refs.by_raw {
        if !present.contains(id.as_str()) {
            continue;
        }
        state.message_refs.by_raw.insert(id.clone(), r#ref.clone());
        if r#ref != BLOCKED_REF {
            state.message_refs.by_ref.insert(r#ref.clone(), id.clone());
            if let Some(&tokens) = parent.token_snapshot.get(r#ref) {
                state.token_snapshot.insert(r#ref.clone(), tokens);
            }
        }
    }
    let highest = highest_used_index(&parent.message_refs);
    if highest > highest_used_index(&state.message_refs) {
        if let Some(r#ref) = index_to_ref(highest) {
            state
                .message_refs
                .by_raw
                .insert(REF_FLOOR_ID.to_owned(), r#ref);
        }
    }
    state
}
