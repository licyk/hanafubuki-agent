//! The hierarchical compression state as the history keeps it: a chain of
//! links, one in each runtime checkpoint that changed the state. A snapshot
//! starts the chain; each delta after it carries only what grew or changed
//! (refs are never reissued and blocks never deleted, so the state is
//! append-mostly), and a new snapshot is written once the deltas since the
//! last one outweigh it, so reading the chain stays proportional to the
//! state.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use hanafubuki_agent_compression::{Block, LocalTime, RefMap, State, BLOCKED_REF, REF_FLOOR_ID};
use serde::{Deserialize, Serialize};

use super::AgentCompressionState;

/// One link of a session's compression chain.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AgentCompressionUpdate {
    /// The whole state, which starts a chain. Its ref map is written one
    /// way (`by_raw`); the other is derived when it is read.
    Snapshot(Box<AgentCompressionState>),
    /// What changed since the chain's previous link.
    Delta(Box<AgentCompressionDelta>),
    /// The state ended: a session that left `hierarchical` compacted its
    /// blocks away.
    Dropped,
    /// A link that could not be read. Never written: the chain is broken
    /// until its next snapshot.
    Unreadable,
}

/// What a link adds to the state before it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct AgentCompressionDelta {
    /// The link's place after its snapshot, which is 0.
    pub seq: u64,
    /// The ref bindings (message id to ref) minted or changed.
    pub refs: BTreeMap<String, String>,
    /// The blocks made, and those changed other than in their age, whole.
    pub blocks: Vec<Block>,
    /// The age of the blocks that only aged (every active block ages one
    /// step per request).
    pub survived: BTreeMap<String, u32>,
    /// The token counts first rendered.
    pub token_snapshot: BTreeMap<String, u64>,
    /// The refs whose token count is no longer kept.
    pub token_snapshot_removed: Vec<String>,
    /// The messages first stamped (stamps are never changed or removed).
    pub ts_stamps: BTreeMap<String, LocalTime>,
    /// Every other field of the state, when one of them changed; its
    /// blocks, refs, token counts and stamps are empty.
    pub rest: Option<Box<AgentCompressionState>>,
}

/// A chain read link by link, oldest first.
#[derive(Clone, Debug, Default)]
pub struct AgentCompressionChain {
    state: Option<AgentCompressionState>,
    seq: u64,
    broken: bool,
}

impl AgentCompressionChain {
    pub fn push(&mut self, link: AgentCompressionUpdate) {
        match link {
            AgentCompressionUpdate::Snapshot(state) => {
                let mut state = *state;
                state.message_refs.by_ref = derived_by_ref(&state.message_refs);
                *self = Self {
                    state: Some(state),
                    ..Self::default()
                };
            }
            AgentCompressionUpdate::Dropped => *self = Self::default(),
            AgentCompressionUpdate::Delta(delta) if !self.broken => {
                let applied = match self.state.as_mut() {
                    Some(state) if delta.seq == self.seq + 1 => apply(state, *delta),
                    Some(_) => Err("a link is missing before it"),
                    None => Err("no snapshot precedes it"),
                };
                match applied {
                    Ok(()) => self.seq += 1,
                    Err(reason) => self.break_off(reason),
                }
            }
            AgentCompressionUpdate::Delta(_) => {}
            AgentCompressionUpdate::Unreadable => self.break_off("a link could not be read"),
        }
    }

    /// The state the chain holds; none without one or once it broke.
    pub fn state(&self) -> Option<&AgentCompressionState> {
        self.state.as_ref().filter(|_| !self.broken)
    }

    pub fn into_state(self) -> Option<AgentCompressionState> {
        self.state.filter(|_| !self.broken)
    }

    fn break_off(&mut self, reason: &str) {
        if !self.broken {
            log::warn!(
                "Agent session compression chain broken: {reason}; the state is rebuilt from the history"
            );
        }
        self.broken = true;
        self.state = None;
    }
}

/// The chain's last link as the kernel wrote it: the state it holds and
/// what decides whether the next link is a delta or a snapshot.
#[derive(Clone, Debug)]
pub(crate) struct CompressionChainHead {
    state: Arc<AgentCompressionState>,
    seq: u64,
    snapshot_bytes: usize,
    delta_bytes: usize,
}

/// The link that takes a chain at `head` to `current`, if anything
/// changed, and the head after it. With no head (nothing written yet, or
/// written by another run) the link is a snapshot, and so it is when the
/// change is not one a delta carries or the deltas would outweigh the
/// snapshot.
pub(crate) fn next_link(
    head: Option<&CompressionChainHead>,
    current: Option<&Arc<AgentCompressionState>>,
) -> (Option<AgentCompressionUpdate>, Option<CompressionChainHead>) {
    let Some(current) = current else {
        return (head.map(|_| AgentCompressionUpdate::Dropped), None);
    };
    if let Some(head) = head {
        if Arc::ptr_eq(&head.state, current) {
            return (None, Some(head.clone()));
        }
        if let Some(delta) = diff(&head.state, current, head.seq + 1) {
            let moved = CompressionChainHead {
                state: current.clone(),
                ..head.clone()
            };
            if delta == empty_delta(delta.seq) {
                return (None, Some(moved));
            }
            let bytes = encoded_len(&delta);
            if head.delta_bytes + bytes <= head.snapshot_bytes {
                return (
                    Some(AgentCompressionUpdate::Delta(Box::new(delta))),
                    Some(CompressionChainHead {
                        seq: head.seq + 1,
                        delta_bytes: head.delta_bytes + bytes,
                        ..moved
                    }),
                );
            }
        }
    }
    let mut snapshot = (**current).clone();
    snapshot.message_refs.by_ref.clear();
    let snapshot_bytes = encoded_len(&snapshot);
    (
        Some(AgentCompressionUpdate::Snapshot(Box::new(snapshot))),
        Some(CompressionChainHead {
            state: current.clone(),
            seq: 0,
            snapshot_bytes,
            delta_bytes: 0,
        }),
    )
}

/// What changed from `base` to `current`, or none when a binding, a block or
/// a stamp went away or changed, or the blocks were reordered: only a
/// snapshot carries that.
fn diff(
    base: &AgentCompressionState,
    current: &AgentCompressionState,
    seq: u64,
) -> Option<AgentCompressionDelta> {
    let mut delta = empty_delta(seq);
    let mut kept = 0;
    for (id, r#ref) in &current.message_refs.by_raw {
        if let Some(before) = base.message_refs.by_raw.get(id) {
            kept += 1;
            if before == r#ref {
                continue;
            }
        }
        delta.refs.insert(id.clone(), r#ref.clone());
    }
    if kept != base.message_refs.by_raw.len() || current.blocks.len() < base.blocks.len() {
        return None;
    }
    for (before, now) in base.blocks.iter().zip(&current.blocks) {
        if before.block_id != now.block_id {
            return None;
        }
        if before == now {
            continue;
        }
        if same_but_age(before, now) {
            delta
                .survived
                .insert(now.block_id.clone(), now.survived_count);
        } else {
            delta.blocks.push(now.clone());
        }
    }
    delta
        .blocks
        .extend(current.blocks[base.blocks.len()..].iter().cloned());
    for (r#ref, tokens) in &current.token_snapshot {
        if base.token_snapshot.get(r#ref) != Some(tokens) {
            delta.token_snapshot.insert(r#ref.clone(), *tokens);
        }
    }
    delta.token_snapshot_removed = base
        .token_snapshot
        .keys()
        .filter(|r#ref| !current.token_snapshot.contains_key(*r#ref))
        .cloned()
        .collect();
    let mut stamps_kept = 0;
    for (id, stamp) in &current.ts_stamps {
        match base.ts_stamps.get(id) {
            Some(before) if before == stamp => stamps_kept += 1,
            Some(_) => return None,
            None => {
                delta.ts_stamps.insert(id.clone(), *stamp);
            }
        }
    }
    if stamps_kept != base.ts_stamps.len() {
        return None;
    }
    let rest = rest_of(current);
    if rest != rest_of(base) {
        delta.rest = Some(Box::new(rest));
    }
    Some(delta)
}

/// `delta` applied to `state`. A delta naming a block or a token count the
/// state does not have belongs to another chain.
fn apply(
    state: &mut AgentCompressionState,
    delta: AgentCompressionDelta,
) -> Result<(), &'static str> {
    for (id, r#ref) in delta.refs {
        if let Some(before) = state.message_refs.by_raw.insert(id.clone(), r#ref.clone()) {
            if before != BLOCKED_REF {
                state.message_refs.by_ref.remove(&before);
            }
        }
        if r#ref != BLOCKED_REF && id != REF_FLOOR_ID {
            state.message_refs.by_ref.insert(r#ref, id);
        }
    }
    let mut positions: HashMap<String, usize> = state
        .blocks
        .iter()
        .enumerate()
        .map(|(index, block)| (block.block_id.clone(), index))
        .collect();
    for block in delta.blocks {
        match positions.get(&block.block_id) {
            Some(&index) => state.blocks[index] = block,
            None => {
                positions.insert(block.block_id.clone(), state.blocks.len());
                state.blocks.push(block);
            }
        }
    }
    for (block_id, survived) in delta.survived {
        let index = positions
            .get(&block_id)
            .ok_or("it ages a block the state does not have")?;
        state.blocks[*index].survived_count = survived;
    }
    state.token_snapshot.extend(delta.token_snapshot);
    for r#ref in delta.token_snapshot_removed {
        state
            .token_snapshot
            .remove(&r#ref)
            .ok_or("it removes a token count the state does not have")?;
    }
    state.ts_stamps.extend(delta.ts_stamps);
    if let Some(rest) = delta.rest {
        let State {
            blocks,
            message_refs,
            token_snapshot,
            ts_stamps,
            ..
        } = std::mem::take(state);
        *state = State {
            blocks,
            message_refs,
            token_snapshot,
            ts_stamps,
            ..*rest
        };
    }
    Ok(())
}

/// The ref map's `by_ref` from its `by_raw`: every numbered binding but the
/// fork's ref floor, which names no message.
fn derived_by_ref(refs: &RefMap) -> BTreeMap<String, String> {
    refs.by_raw
        .iter()
        .filter(|(id, r#ref)| *r#ref != BLOCKED_REF && *id != REF_FLOOR_ID)
        .map(|(id, r#ref)| (r#ref.clone(), id.clone()))
        .collect()
}

fn empty_delta(seq: u64) -> AgentCompressionDelta {
    AgentCompressionDelta {
        seq,
        ..AgentCompressionDelta::default()
    }
}

/// Every field of `state` but the four that grow with the window.
fn rest_of(state: &AgentCompressionState) -> AgentCompressionState {
    // Listed in full, so a new field of the state is placed on purpose.
    let State {
        blocks: _,
        message_refs: _,
        token_snapshot: _,
        ts_stamps: _,
        nudge,
        stats,
        terminal_streak,
        hidden_orphan_refs,
        swept_orphan_call_ids,
        pending_orphan_sweep_min_ref,
        emergency_truncate_latch,
        render_deferred_refs,
        view_latches,
        view_latch_config_digest,
        dead_refs,
        next_block_id,
        next_run_id,
        absorbed,
        rules,
        next_rule_id,
    } = state;
    State {
        blocks: Vec::new(),
        message_refs: RefMap::default(),
        token_snapshot: BTreeMap::new(),
        ts_stamps: BTreeMap::new(),
        nudge: nudge.clone(),
        stats: stats.clone(),
        terminal_streak: *terminal_streak,
        hidden_orphan_refs: hidden_orphan_refs.clone(),
        swept_orphan_call_ids: swept_orphan_call_ids.clone(),
        pending_orphan_sweep_min_ref: *pending_orphan_sweep_min_ref,
        emergency_truncate_latch: emergency_truncate_latch.clone(),
        render_deferred_refs: render_deferred_refs.clone(),
        view_latches: view_latches.clone(),
        view_latch_config_digest: view_latch_config_digest.clone(),
        dead_refs: dead_refs.clone(),
        next_block_id: *next_block_id,
        next_run_id: *next_run_id,
        absorbed: absorbed.clone(),
        rules: rules.clone(),
        next_rule_id: *next_rule_id,
    }
}

/// `a` and `b` differ at most in their age.
fn same_but_age(a: &Block, b: &Block) -> bool {
    let Block {
        block_id,
        run_id,
        tier,
        topic,
        summary,
        direct_message_ids,
        effective_message_ids,
        direct_block_ids,
        compressed_tokens,
        created_at,
        survived_count: _,
        generation,
        active,
        expanded,
        restored_inline,
        compress_call_id,
        start_ref,
        end_ref,
    } = a;
    *block_id == b.block_id
        && *run_id == b.run_id
        && *tier == b.tier
        && *topic == b.topic
        && *summary == b.summary
        && *direct_message_ids == b.direct_message_ids
        && *effective_message_ids == b.effective_message_ids
        && *direct_block_ids == b.direct_block_ids
        && *compressed_tokens == b.compressed_tokens
        && *created_at == b.created_at
        && *generation == b.generation
        && *active == b.active
        && *expanded == b.expanded
        && *restored_inline == b.restored_inline
        && *compress_call_id == b.compress_call_id
        && *start_ref == b.start_ref
        && *end_ref == b.end_ref
}

fn encoded_len<T: Serialize>(value: &T) -> usize {
    serde_json::to_vec(value).map_or(0, |encoded| encoded.len())
}

#[cfg(test)]
mod tests;
