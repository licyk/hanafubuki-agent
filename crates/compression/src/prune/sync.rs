// Derived from billion-context (https://github.com/ranxianglei/billion-context),
// `kernel/src/sync.ts` of acp-kernel 0.0.189 at commit `4be21b08e`.
// Copyright (c) 2026 ranxianglei. MIT License with an additional attribution
// term (NOTICE).
// Modified for hanafubuki-agent: the state is updated in place, ids are
// compared exactly and there are no ledger intervals.

use std::collections::HashSet;

use super::summary_message_id;
use crate::model::{Message, State};

/// Re-derives which blocks are active from the messages the host holds: a
/// consumed or expanded block is inactive; any other block is active while
/// one of its messages (or its rendered summary) is present, and comes back
/// when they reappear. The token snapshot keeps only refs still present.
/// Returns the blocks deactivated because their messages are gone.
pub fn sync_blocks(messages: &[Message], state: &mut State) -> Vec<String> {
    let present: HashSet<&str> = messages.iter().map(|message| message.id.as_str()).collect();

    let live_refs: HashSet<&String> = messages
        .iter()
        .filter_map(|message| state.message_refs.by_raw.get(&message.id))
        .collect();
    if state.token_snapshot.len() != live_refs.len() {
        let snapshot = std::mem::take(&mut state.token_snapshot);
        state.token_snapshot = snapshot
            .into_iter()
            .filter(|(r#ref, _)| live_refs.contains(r#ref))
            .collect();
    }

    let consumed: HashSet<String> = state
        .blocks
        .iter()
        .flat_map(|block| block.direct_block_ids.iter().cloned())
        .collect();
    let mut deactivated = Vec::new();
    for block in &mut state.blocks {
        if consumed.contains(&block.block_id) || block.expanded {
            block.active = false;
            continue;
        }
        let still_present = block
            .effective_message_ids
            .iter()
            .any(|id| present.contains(id.as_str()))
            || present.contains(summary_message_id(&block.block_id).as_str());
        block.active = still_present;
        if !still_present {
            deactivated.push(block.block_id.clone());
        }
    }
    deactivated
}
