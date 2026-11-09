// Derived from billion-context (https://github.com/ranxianglei/billion-context),
// `kernel/src/block-map.ts` of acp-kernel 0.0.189 at commit `4be21b08e`.
// Copyright (c) 2026 ranxianglei. MIT License with an additional attribution
// term (NOTICE).

use std::collections::BTreeMap;

use crate::model::{Block, State};
use crate::refs::{ordered_ref_pair, BLOCKED_REF};

/// An active block's ref span, for display.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BlockSpan {
    pub block_id: String,
    pub tier: u8,
    pub start_ref: String,
    pub end_ref: String,
}

/// A block's span: its requested refs when both are message refs (in
/// ascending order), else the lowest and highest refs of what it covers.
pub fn resolve_block_span(
    block: &Block,
    by_raw: &BTreeMap<String, String>,
) -> Option<(String, String)> {
    if let (Some(start), Some(end)) = (block.start_ref.as_deref(), block.end_ref.as_deref()) {
        if is_message_ref(start) && is_message_ref(end) {
            let (start, end) = ordered_ref_pair(start, end);
            return Some((start.to_owned(), end.to_owned()));
        }
    }
    let mut refs: Vec<&String> = block
        .effective_message_ids
        .iter()
        .filter_map(|id| by_raw.get(id))
        .filter(|r#ref| *r#ref != BLOCKED_REF)
        .collect();
    refs.sort_by_key(|r#ref| ref_number(r#ref));
    Some(((*refs.first()?).clone(), (*refs.last()?).clone()))
}

pub fn active_block_spans(state: &State) -> Vec<BlockSpan> {
    state
        .active_blocks()
        .filter_map(|block| {
            let (start_ref, end_ref) = resolve_block_span(block, &state.message_refs.by_raw)?;
            Some(BlockSpan {
                block_id: block.block_id.clone(),
                tier: block.tier,
                start_ref,
                end_ref,
            })
        })
        .collect()
}

/// `blocks: b3=m00044–m00097, b4=m00103–m00123`, so the model learns its
/// blocks from the kernel rather than from its own arguments; empty when
/// there are none.
pub fn format_created_blocks(state: &State, blocks: &[Block]) -> String {
    let parts: Vec<String> = blocks
        .iter()
        .map(
            |block| match resolve_block_span(block, &state.message_refs.by_raw) {
                Some((start, end)) => format!("{}={start}–{end}", block.block_id),
                None => block.block_id.clone(),
            },
        )
        .collect();
    if parts.is_empty() {
        String::new()
    } else {
        format!("blocks: {}", parts.join(", "))
    }
}

fn is_message_ref(r#ref: &str) -> bool {
    r#ref.strip_prefix('m').is_some_and(|digits| {
        !digits.is_empty() && digits.bytes().all(|byte| byte.is_ascii_digit())
    })
}

/// The first run of digits in a ref, as the kernel sorts spans.
fn ref_number(r#ref: &str) -> u64 {
    let digits: String = r#ref
        .chars()
        .skip_while(|character| !character.is_ascii_digit())
        .take_while(char::is_ascii_digit)
        .collect();
    digits.parse().unwrap_or(0)
}
