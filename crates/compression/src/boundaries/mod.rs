// Derived from billion-context (https://github.com/ranxianglei/billion-context),
// `kernel/src/boundaries.ts` of acp-kernel 0.0.189 at commit `4be21b08e`.
// Copyright (c) 2026 ranxianglei. MIT License with an additional attribution
// term (NOTICE).
// Modified for hanafubuki-agent: a failed resolution is an error value, and
// there are no content-store retrieval messages to leave out.

mod block_map;

use std::collections::HashMap;

pub use block_map::{active_block_spans, format_created_blocks, resolve_block_span, BlockSpan};

use crate::model::{Block, Message, State};
use crate::prune::{is_rendered_summary, summary_message_id};
use crate::refs::{highest_used_index, parse_padded_index, ref_to_index, BLOCKED_REF};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BoundaryKind {
    Message,
    Block,
}

/// A parsed range endpoint: `mNNNNN` or `bN`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Boundary {
    pub kind: BoundaryKind,
    pub numeric_id: u32,
    /// Trimmed and lowercased.
    pub raw: String,
}

pub fn parse_boundary(r#ref: &str) -> Option<Boundary> {
    let raw = r#ref.trim().to_lowercase();
    if let Some(numeric_id) = raw.strip_prefix('m').and_then(parse_padded_index) {
        return Some(Boundary {
            kind: BoundaryKind::Message,
            numeric_id,
            raw,
        });
    }
    let digits = raw.strip_prefix('b')?;
    if digits.is_empty() || digits.len() > 9 || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let numeric_id: u32 = digits.parse().ok().filter(|id| *id >= 1)?;
    Some(Boundary {
        kind: BoundaryKind::Block,
        numeric_id,
        raw,
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Endpoint {
    Start,
    End,
}

impl Endpoint {
    fn label(self) -> &'static str {
        match self {
            Self::Start => "startId",
            Self::End => "endId",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BoundaryErrorKind {
    /// Not `mNNNNN` or `bN`.
    Invalid,
    /// It never existed in this session.
    Unknown,
    /// It exists, but its message is folded or gone.
    Consumed,
}

/// Why a range endpoint cannot be anchored; the message is written for the
/// model.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BoundaryError {
    pub kind: BoundaryErrorKind,
    pub endpoint: Option<Endpoint>,
    pub message: String,
}

impl BoundaryError {
    fn new(kind: BoundaryErrorKind, endpoint: Endpoint, message: String) -> Self {
        Self {
            kind,
            endpoint: Some(endpoint),
            message,
        }
    }
}

/// A range resolved against a message array.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedRange {
    pub start_index: usize,
    pub end_index: usize,
    /// Every message id in the span but rendered summaries.
    pub message_ids: Vec<String>,
    /// The active blocks visible in the span.
    pub nested_block_ids: Vec<String>,
    pub boundary_kind: BoundaryKind,
    /// Why an endpoint was moved, for the model.
    pub snapped_boundaries: Vec<String>,
    pub reversed_note: Option<String>,
}

/// Message id to its last index in `messages`.
pub(crate) fn index_by_message_id(messages: &[Message]) -> HashMap<&str, usize> {
    messages
        .iter()
        .enumerate()
        .map(|(index, message)| (message.id.as_str(), index))
        .collect()
}

/// The positions ranges resolve against, built once per message array and
/// state: each message id's last index, and where each active block shows.
pub(crate) struct MessageIndex<'a> {
    pub(crate) by_id: HashMap<&'a str, usize>,
    /// Each active block, in state order, with the positions of its rendered
    /// summary and of its earliest visible message.
    active_blocks: Vec<(&'a Block, Option<usize>, Option<usize>)>,
}

impl<'a> MessageIndex<'a> {
    pub(crate) fn new(messages: &'a [Message], state: &'a State) -> Self {
        let by_id = index_by_message_id(messages);
        let active_blocks = state
            .active_blocks()
            .map(|block| {
                let summary = by_id
                    .get(summary_message_id(&block.block_id).as_str())
                    .copied();
                let earliest = earliest_index_of_ids(&block.effective_message_ids, &by_id);
                (block, summary, earliest)
            })
            .collect();
        Self {
            by_id,
            active_blocks,
        }
    }

    /// The active blocks in `start..=end`, in state order. A block is in a
    /// range when its rendered summary or its earliest visible message is:
    /// the first user message survives a fold with its summary placed
    /// before it, outside a range that still holds the message.
    pub(crate) fn blocks_in_range(
        &self,
        start: usize,
        end: usize,
    ) -> impl Iterator<Item = &'a Block> + '_ {
        let within = move |position: Option<usize>| {
            position.is_some_and(|position| (start..=end).contains(&position))
        };
        self.active_blocks
            .iter()
            .filter(move |(_, summary, earliest)| within(*summary) || within(*earliest))
            .map(|(block, _, _)| *block)
    }
}

/// Resolves `start..end` to indices of `messages`: a message ref to its
/// message (or the active block now covering it), a block ref to the
/// block's visible anchor. Reversed endpoints are swapped with a note; an
/// endpoint on a hidden orphan `compress` call, a dead ref or (for the end)
/// a ref above the highest allocated one snaps to its visible neighbour.
pub fn resolve_boundaries(
    start_ref: &str,
    end_ref: &str,
    messages: &[Message],
    state: &State,
) -> Result<ResolvedRange, BoundaryError> {
    resolve_boundaries_indexed(
        start_ref,
        end_ref,
        messages,
        state,
        &MessageIndex::new(messages, state),
    )
}

pub(crate) fn resolve_boundaries_indexed(
    start_ref: &str,
    end_ref: &str,
    messages: &[Message],
    state: &State,
    message_index: &MessageIndex<'_>,
) -> Result<ResolvedRange, BoundaryError> {
    let index = &message_index.by_id;
    let (Some(start), Some(end)) = (parse_boundary(start_ref), parse_boundary(end_ref)) else {
        return Err(BoundaryError {
            kind: BoundaryErrorKind::Invalid,
            endpoint: None,
            message: format!(
                "Invalid boundary ref(s): startId=\"{start_ref}\", endId=\"{end_ref}\". Use mNNNNN or bN."
            ),
        });
    };
    let mut snapped_boundaries = Vec::new();
    let (mut start_index, snapped) = resolve_anchor(&start, state, index, Endpoint::Start)?;
    snapped_boundaries.extend(snapped);
    let (mut end_index, snapped) = resolve_anchor(&end, state, index, Endpoint::End)?;
    snapped_boundaries.extend(snapped);

    let mut reversed_note = None;
    if start_index > end_index {
        // The direction is notational, but the model must see that its input
        // was rewritten.
        std::mem::swap(&mut start_index, &mut end_index);
        reversed_note = Some(format!(
            "note: refs were given reversed ({}→{}), normalized to {}..{}",
            start.raw, end.raw, end.raw, start.raw
        ));
    }

    let message_ids = messages[start_index..=end_index]
        .iter()
        .filter(|message| !is_rendered_summary(message))
        .map(|message| message.id.clone())
        .collect();
    let boundary_kind = if start.kind == BoundaryKind::Block || end.kind == BoundaryKind::Block {
        BoundaryKind::Block
    } else {
        BoundaryKind::Message
    };
    let mut nested_block_ids: Vec<String> = Vec::new();
    for block in message_index.blocks_in_range(start_index, end_index) {
        if !nested_block_ids.contains(&block.block_id) {
            nested_block_ids.push(block.block_id.clone());
        }
    }

    Ok(ResolvedRange {
        start_index,
        end_index,
        message_ids,
        nested_block_ids,
        boundary_kind,
        snapped_boundaries,
        reversed_note,
    })
}

fn resolve_anchor(
    boundary: &Boundary,
    state: &State,
    index: &HashMap<&str, usize>,
    endpoint: Endpoint,
) -> Result<(usize, Option<String>), BoundaryError> {
    match boundary.kind {
        BoundaryKind::Message => resolve_message_anchor(boundary, state, index, endpoint),
        BoundaryKind::Block => resolve_block_anchor(boundary, state, index, endpoint),
    }
}

fn resolve_message_anchor(
    boundary: &Boundary,
    state: &State,
    index: &HashMap<&str, usize>,
    endpoint: Endpoint,
) -> Result<(usize, Option<String>), BoundaryError> {
    let label = endpoint.label();
    let raw = &boundary.raw;
    let padded = padded_ref(boundary.numeric_id);
    let by_ref = &state.message_refs.by_ref;
    let Some(raw_id) = by_ref.get(raw).or_else(|| by_ref.get(&padded)) else {
        return unallocated_anchor(boundary, state, index, endpoint);
    };
    if let Some(found) = index.get(raw_id.as_str()) {
        return Ok((*found, None));
    }
    if let Some(owner) = active_owner_anchor(state, std::slice::from_ref(raw_id), index) {
        return Ok((owner, Some(format!(
            "{label}=\"{raw}\" refers to a message already compressed into an active block — anchored to the active block covering it instead."
        ))));
    }
    if state.hidden_orphan_refs.contains(&padded) {
        if let Some(neighbor) =
            snap_to_neighbor_visible(state, index, boundary.numeric_id, endpoint)
        {
            let direction = match endpoint {
                Endpoint::Start => "the next visible message after it",
                Endpoint::End => "the previous visible message before it",
            };
            return Ok((neighbor, Some(format!(
                "{label}=\"{raw}\" is a hidden orphan compress call (no matching block) — snapped to {direction} instead."
            ))));
        }
        return Err(BoundaryError::new(BoundaryErrorKind::Consumed, endpoint, format!(
            "{label}=\"{raw}\" is a hidden orphan compress call with no adjacent visible message to anchor to — run acp_status for current refs."
        )));
    }
    Err(BoundaryError::new(
        BoundaryErrorKind::Consumed,
        endpoint,
        format!(
        "{label}=\"{raw}\" not found in visible context (likely consumed by an existing block)."
    ),
    ))
}

/// A message ref with no `by_ref` entry: a dead ref snaps to a visible
/// neighbour; an end above the highest allocated ref clamps down to it.
fn unallocated_anchor(
    boundary: &Boundary,
    state: &State,
    index: &HashMap<&str, usize>,
    endpoint: Endpoint,
) -> Result<(usize, Option<String>), BoundaryError> {
    let label = endpoint.label();
    let raw = &boundary.raw;
    let padded = padded_ref(boundary.numeric_id);
    if state.dead_refs.contains(&padded) {
        if let Some(neighbor) =
            snap_to_neighbor_visible(state, index, boundary.numeric_id, endpoint)
        {
            return Ok((neighbor, Some(format!(
                "{label}=\"{raw}\" no longer backs any visible message (its content was pruned or removed by the client) — anchored to the nearest visible message instead, so the range folds slightly less than requested."
            ))));
        }
        return Err(BoundaryError::new(BoundaryErrorKind::Consumed, endpoint, format!(
            "{label}=\"{raw}\" is a dead ref (its message was pruned or removed by the client) and no adjacent visible message exists to anchor to — nothing left to fold here. run acp_status for live refs."
        )));
    }
    // Refs are minted in order and never pre-allocated: a number above the
    // highest cannot exist yet, while a gap below it is a sparse-space hole.
    let highest = highest_used_index(&state.message_refs);
    if boundary.numeric_id > highest {
        let highest_ref = if highest > 0 {
            padded_ref(highest)
        } else {
            "none".to_owned()
        };
        // Aiming the end past the newest tail is the most common compress
        // failure on long sessions: it folds up to the last visible message.
        if endpoint == Endpoint::End {
            if let Some(neighbor) =
                snap_to_neighbor_visible(state, index, boundary.numeric_id, endpoint)
            {
                return Ok((neighbor, Some(format!(
                    "{label}=\"{raw}\" is above this session's highest allocated ref ({highest_ref}) — clamped to it (refs are minted in order and never pre-allocated, so higher numbers do not exist yet). The range folds up to the last visible message."
                ))));
            }
        }
        return Err(BoundaryError::new(BoundaryErrorKind::Unknown, endpoint, format!(
            "{label}=\"{raw}\" is above this session's highest allocated ref ({highest_ref}) — refs are minted in order and never pre-allocated, so it cannot exist in this session (stale artifact from an earlier generation, or a typo). run acp_status for current refs."
        )));
    }
    Err(BoundaryError::new(BoundaryErrorKind::Unknown, endpoint, format!(
        "{label}=\"{raw}\" was never allocated in this session or was pruned after allocation — the ref space is sparse, so a number at or below the highest is not necessarily present; this is not a cross-session ref. run acp_status for current refs."
    )))
}

fn resolve_block_anchor(
    boundary: &Boundary,
    state: &State,
    index: &HashMap<&str, usize>,
    endpoint: Endpoint,
) -> Result<(usize, Option<String>), BoundaryError> {
    let label = endpoint.label();
    let id = format!("b{}", boundary.numeric_id);
    let Some(block) = state.block(&id) else {
        return Err(BoundaryError::new(BoundaryErrorKind::Unknown, endpoint, format!(
            "{label}=\"{id}\" does not exist in this session (typo or wrong session) — run acp_status for current refs."
        )));
    };
    if block.active {
        if let Some(anchor) = visible_block_anchor(block, index) {
            return Ok((anchor, None));
        }
    }
    if let Some(owner) = active_owner_anchor(state, &block.effective_message_ids, index) {
        return Ok((owner, Some(format!(
            "{label}=\"{id}\" was consumed by a higher-tier block — anchored to the active block covering its content instead."
        ))));
    }
    let message = if block.active {
        format!("{label}=\"{id}\" is an active block but none of its content (raw messages or rendered summary) is visible in the current context — run acp_status to verify.")
    } else {
        format!("{label}=\"{id}\" not found in visible context (block distilled/consumed by a higher-tier block).")
    };
    Err(BoundaryError::new(
        BoundaryErrorKind::Consumed,
        endpoint,
        message,
    ))
}

/// The visible anchor of the active block that inherited `owned_ids` by
/// consuming another block. A block that folded a message directly does not
/// qualify: the caller gets the "already compressed" guidance instead of a
/// silent snap that would turn a retry into a same-tier duplicate.
fn active_owner_anchor(
    state: &State,
    owned_ids: &[String],
    index: &HashMap<&str, usize>,
) -> Option<usize> {
    if owned_ids.is_empty() {
        return None;
    }
    state
        .active_blocks()
        .filter(|block| {
            let inherited: std::collections::HashSet<&str> = block
                .direct_block_ids
                .iter()
                .filter_map(|child| state.block(child))
                .flat_map(|child| child.effective_message_ids.iter().map(String::as_str))
                .collect();
            owned_ids.iter().any(|id| inherited.contains(id.as_str()))
        })
        .filter_map(|block| visible_block_anchor(block, index))
        .min()
}

/// The visible message nearest to ref number `ref_number`: after it for a
/// start, before it for an end. Refs are never recycled, so nearest by
/// number is nearest by position.
fn snap_to_neighbor_visible(
    state: &State,
    index: &HashMap<&str, usize>,
    ref_number: u32,
    endpoint: Endpoint,
) -> Option<usize> {
    let mut best: Option<(u32, usize)> = None;
    for (raw_id, position) in index {
        let Some(ref_text) = state.message_refs.by_raw.get(*raw_id) else {
            continue;
        };
        if ref_text == BLOCKED_REF {
            continue;
        }
        let Some(r#ref) = ref_to_index(ref_text) else {
            continue;
        };
        let better = match endpoint {
            Endpoint::Start => r#ref > ref_number && best.is_none_or(|(known, _)| r#ref < known),
            Endpoint::End => r#ref < ref_number && best.is_none_or(|(known, _)| r#ref > known),
        };
        if better {
            best = Some((r#ref, *position));
        }
    }
    best.map(|(_, position)| position)
}

fn padded_ref(index: u32) -> String {
    format!("m{index:05}")
}

/// A block's rendered summary if the view has one, else the earliest of its
/// messages the view still holds.
pub(crate) fn visible_block_anchor(block: &Block, index: &HashMap<&str, usize>) -> Option<usize> {
    index
        .get(summary_message_id(&block.block_id).as_str())
        .copied()
        .or_else(|| earliest_index_of_ids(&block.effective_message_ids, index))
}

pub(crate) fn earliest_index_of_ids(ids: &[String], index: &HashMap<&str, usize>) -> Option<usize> {
    ids.iter()
        .filter_map(|id| index.get(id.as_str()).copied())
        .min()
}
