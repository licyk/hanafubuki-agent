// Derived from billion-context (https://github.com/ranxianglei/billion-context),
// `kernel/src/refold.ts` and `activeAncestorIds` of `kernel/src/decompress.ts`
// of acp-kernel 0.0.189 at commit `4be21b08e`. Copyright (c) 2026
// ranxianglei. MIT License with an additional attribution term (NOTICE).

use crate::boundaries::{parse_boundary, resolve_block_span, BoundaryKind};
use crate::model::State;

/// Whether a range over folded content updates blocks in place.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RefoldDecision {
    /// The blocks to update.
    Refold(Vec<String>),
    /// Why not, by block (`blocked by bN (not restored)`, `partially
    /// covered bN`); empty when nothing in the span is folded.
    Blocked(Vec<String>),
}

/// A range over folded content updates its blocks in place (same id, new
/// summary) when every active block in the span is restored inline, with
/// its active ancestors, and lies wholly inside it.
pub fn evaluate_refold(state: &State, start_ref: &str, end_ref: &str) -> RefoldDecision {
    let Some((low, high)) = requested_span(state, start_ref, end_ref) else {
        return RefoldDecision::Blocked(Vec::new());
    };
    let mut reasons: Vec<String> = Vec::new();
    let mut blocks = Vec::new();
    let mut reason = |text: String| {
        if !reasons.contains(&text) {
            reasons.push(text);
        }
    };
    for block in state.active_blocks() {
        let Some((start, end)) = resolve_block_span(block, &state.message_refs.by_raw) else {
            reason(format!("partially covered {}", block.block_id));
            continue;
        };
        let (Some(own_low), Some(own_high)) = (parse_boundary(&start), parse_boundary(&end)) else {
            continue;
        };
        let (own_low, own_high) = (own_low.numeric_id, own_high.numeric_id);
        if own_high < low || own_low > high {
            continue;
        }
        if !block.restored_inline {
            reason(format!("blocked by {} (not restored)", block.block_id));
            continue;
        }
        // Part of the block would keep its stale summary.
        if own_low < low || own_high > high {
            reason(format!("partially covered {}", block.block_id));
            continue;
        }
        let blockers: Vec<String> = active_ancestor_ids(state, &block.block_id)
            .into_iter()
            .filter(|ancestor| {
                !state
                    .block(ancestor)
                    .is_some_and(|ancestor| ancestor.restored_inline)
            })
            .collect();
        if !blockers.is_empty() {
            for blocker in blockers {
                reason(format!("blocked by {blocker} (not restored)"));
            }
            continue;
        }
        blocks.push(block.block_id.clone());
    }
    if !reasons.is_empty() {
        return RefoldDecision::Blocked(reasons);
    }
    if blocks.is_empty() {
        return RefoldDecision::Blocked(Vec::new());
    }
    RefoldDecision::Refold(blocks)
}

/// The ref-number span of a range; a block ref spans its block.
fn requested_span(state: &State, start_ref: &str, end_ref: &str) -> Option<(u32, u32)> {
    let mut numbers = Vec::new();
    for r#ref in [start_ref, end_ref] {
        let boundary = parse_boundary(r#ref)?;
        if boundary.kind == BoundaryKind::Message {
            numbers.push(boundary.numeric_id);
            continue;
        }
        let block = state.block(&format!("b{}", boundary.numeric_id))?;
        let (start, end) = resolve_block_span(block, &state.message_refs.by_raw)?;
        numbers.push(parse_boundary(&start)?.numeric_id);
        numbers.push(parse_boundary(&end)?.numeric_id);
    }
    Some((*numbers.iter().min()?, *numbers.iter().max()?))
}

/// The active blocks above `block_id`: those that consumed it, and so on up.
pub(crate) fn active_ancestor_ids(state: &State, block_id: &str) -> Vec<String> {
    let mut ancestors = Vec::new();
    let mut visited = vec![block_id.to_owned()];
    let mut frontier = vec![block_id.to_owned()];
    while !frontier.is_empty() {
        let mut next = Vec::new();
        for block in &state.blocks {
            if visited.contains(&block.block_id) {
                continue;
            }
            if !block
                .direct_block_ids
                .iter()
                .any(|id| frontier.contains(id))
            {
                continue;
            }
            visited.push(block.block_id.clone());
            if block.active {
                ancestors.push(block.block_id.clone());
            }
            next.push(block.block_id.clone());
        }
        frontier = next;
    }
    ancestors
}
