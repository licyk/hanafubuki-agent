// Derived from billion-context (https://github.com/ranxianglei/billion-context),
// `kernel/src/unit-projection.ts` of acp-kernel 0.0.189 at commit
// `4be21b08e`. Copyright (c) 2026 ranxianglei. MIT License with an
// additional attribution term (NOTICE).
// Modified for hanafubuki-agent: the shared per-array context is always
// built, there are no host checkpoint carriers, and no message carries
// unrecoverable media.

mod carve;
mod integrity;
mod merge;
mod pairs;
mod segment;

use std::collections::{HashMap, HashSet};

pub use integrity::{compute_integrity_withdrawals, compute_turn_groups, IntegrityWithdrawals};
pub use pairs::{adjust_boundaries_for_reasoning_pairs, adjust_boundaries_for_tool_pairs};
pub use segment::{segment_groups, viable_ranges, SegmentItem, VIABLE_RANGE_MIN_TOKENS};

use crate::boundaries::{
    block_visible_in_range, index_by_message_id, resolve_boundaries_indexed, BoundaryError,
    BoundaryKind,
};
use crate::ids::OrderedIds;
use crate::model::{Config, Kind, Message, State};
use crate::protect::{
    collect_aged_compress_call_ids, collect_failed_compress_call_ids, collect_latest_protected,
    is_message_protected, last_user_anchor, LatestProtected, COMPRESS_EXCHANGE_KEEP_RECENT,
};
use crate::prune::is_summary_message_id;
use integrity::IntegrityPre;

/// Everything a projection derives from one `(messages, state, config)`:
/// build it once and project every unit against it.
pub struct ProjectionContext<'a> {
    messages: &'a [Message],
    state: &'a State,
    config: &'a Config,
    by_id: HashMap<&'a str, usize>,
    failed_compress_call_ids: HashSet<String>,
    latest: LatestProtected,
    aged_compress_call_ids: HashSet<String>,
    /// The latest-protected calls and every protected call but the aged
    /// `compress` ones.
    base_protected_call_ids: HashSet<String>,
    /// Call and result message ids by tool-call id (or message id).
    pair_by_call_id: HashMap<String, (String, String)>,
    tool_pair_extremes: HashMap<String, (usize, usize)>,
    anchor_message: Option<&'a Message>,
    integrity: IntegrityPre,
}

impl<'a> ProjectionContext<'a> {
    pub fn new(messages: &'a [Message], state: &'a State, config: &'a Config) -> Self {
        let failed_compress_call_ids = collect_failed_compress_call_ids(messages);
        let latest = collect_latest_protected(messages, config);
        let aged_compress_call_ids =
            collect_aged_compress_call_ids(messages, COMPRESS_EXCHANGE_KEEP_RECENT);
        let mut base_protected_call_ids = latest.call_ids.clone();
        for message in messages {
            let Some(call_id) = &message.tool_call_id else {
                continue;
            };
            if is_message_protected(message, config, &failed_compress_call_ids)
                && !aged_compress_call_ids.contains(call_id)
            {
                base_protected_call_ids.insert(call_id.clone());
            }
        }
        let mut pair_by_call_id: HashMap<String, (String, String)> = HashMap::new();
        for message in messages.iter().filter(|message| message.is_tool()) {
            let key = message
                .tool_call_id
                .clone()
                .unwrap_or_else(|| message.id.clone());
            let pair = pair_by_call_id.entry(key).or_default();
            if message.kind == Kind::ToolCall {
                pair.0 = message.id.clone();
            } else {
                pair.1 = message.id.clone();
            }
        }
        Self {
            messages,
            state,
            config,
            by_id: index_by_message_id(messages),
            failed_compress_call_ids,
            latest,
            aged_compress_call_ids,
            base_protected_call_ids,
            pair_by_call_id,
            tool_pair_extremes: pairs::tool_pair_extremes(messages),
            anchor_message: last_user_anchor(messages, &state.covered_message_ids()),
            integrity: IntegrityPre::new(messages),
        }
    }

    fn message(&self, id: &str) -> Option<&'a Message> {
        self.by_id
            .get(id)
            .and_then(|index| self.messages.get(*index))
    }

    fn ref_or_id(&self, id: &str) -> String {
        self.state
            .message_refs
            .by_raw
            .get(id)
            .cloned()
            .unwrap_or_else(|| id.to_owned())
    }
}

/// One candidate range and what surrounds it.
pub struct UnitRequest<'r> {
    pub start_ref: &'r str,
    pub end_ref: &'r str,
    /// The message ids active blocks covered before this unit.
    pub pre_existing_coverage: &'r HashSet<String>,
    /// The refs of the recent zone, if it is enforced.
    pub protected_refs: Option<&'r HashSet<String>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FatalKind {
    /// Entirely inside the protected zone: a no-op, not a failure.
    Zone,
    /// Folding would split a turn or a tool pair.
    Split,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Fatal {
    pub kind: FatalKind,
    pub message: String,
}

/// What a range folds, as the apply step would fold it.
#[derive(Clone, Debug, PartialEq)]
pub struct UnitProjection {
    /// Set when the apply step refuses the unit.
    pub fatal: Option<Fatal>,
    pub range_message_ids: Vec<String>,
    pub nested_block_ids: Vec<String>,
    pub consumed_block_ids: Vec<String>,
    pub effective_message_ids: Vec<String>,
    pub direct_message_ids: Vec<String>,
    /// The messages the new block folds itself.
    pub filtered_ids: Vec<String>,
    pub withdrawn_ids: Vec<String>,
    pub split_turn_count: usize,
    pub split_pair_count: usize,
    pub target_tier: u8,
    pub output_tier: u8,
    pub is_block_boundary: bool,
}

/// The one definition of what a range folds, shared by the apply step and
/// the recommendations: resolve the boundaries, widen for tool and
/// reasoning pairs, take in nested blocks, carve out protected tools and
/// the recent zone, and withdraw what would split a turn. Warnings are
/// pushed in the order the apply step reports them.
pub fn project_unit(
    context: &ProjectionContext<'_>,
    request: &UnitRequest<'_>,
    warnings: &mut Vec<String>,
) -> Result<UnitProjection, BoundaryError> {
    project_excluding(context, request, &HashSet::new(), warnings)
}

/// A projection under construction.
struct Draft {
    range_message_ids: Vec<String>,
    nested_block_ids: Vec<String>,
    consumed_block_ids: Vec<String>,
    effective: OrderedIds,
    direct: Vec<String>,
    filtered: Vec<String>,
    withdrawn: Vec<String>,
    split_turn_count: usize,
    split_pair_count: usize,
    target_tier: u8,
    output_tier: u8,
    is_block_boundary: bool,
    /// Nested blocks the merge ladder keeps active.
    excluded_nested_block_ids: Vec<String>,
}

impl Draft {
    fn finish(self, fatal: Option<Fatal>) -> UnitProjection {
        UnitProjection {
            fatal,
            range_message_ids: self.range_message_ids,
            nested_block_ids: self.nested_block_ids,
            consumed_block_ids: self.consumed_block_ids,
            effective_message_ids: self.effective.to_vec(),
            direct_message_ids: self.direct,
            filtered_ids: self.filtered,
            withdrawn_ids: self.withdrawn,
            split_turn_count: self.split_turn_count,
            split_pair_count: self.split_pair_count,
            target_tier: self.target_tier,
            output_tier: self.output_tier,
            is_block_boundary: self.is_block_boundary,
        }
    }
}

/// `excluded` are nested blocks the merge ladder keeps active: they are not
/// consumed, and as active blocks their coverage stays pre-existing.
fn project_excluding(
    context: &ProjectionContext<'_>,
    request: &UnitRequest<'_>,
    excluded: &HashSet<String>,
    warnings: &mut Vec<String>,
) -> Result<UnitProjection, BoundaryError> {
    let state = context.state;
    let resolved = resolve_boundaries_indexed(
        request.start_ref,
        request.end_ref,
        context.messages,
        state,
        &context.by_id,
    )?;
    let range_message_ids: Vec<String> =
        pairs::widened_range_ids(&resolved, context.messages, &context.tool_pair_extremes)
            .into_iter()
            .filter(|id| !is_summary_message_id(id))
            .collect();

    // Pair widening may pull in the anchor of another block.
    let mut nested_block_ids = resolved.nested_block_ids.clone();
    if range_message_ids.len() > resolved.message_ids.len() {
        let position = |id: Option<&String>, fallback| {
            id.and_then(|id| context.by_id.get(id.as_str()).copied())
                .unwrap_or(fallback)
        };
        let start = position(range_message_ids.first(), resolved.start_index);
        let end = position(range_message_ids.last(), resolved.end_index);
        for block in state.active_blocks() {
            if !nested_block_ids.contains(&block.block_id)
                && block_visible_in_range(block, &context.by_id, start, end)
            {
                nested_block_ids.push(block.block_id.clone());
            }
        }
    }

    let is_block_boundary = resolved.boundary_kind == BoundaryKind::Block;
    let target_tier = target_tier(state, &nested_block_ids, is_block_boundary);
    let output_tier = if is_block_boundary {
        (target_tier + 1).min(3)
    } else {
        1
    };
    let excluded_nested_block_ids: Vec<String> = nested_block_ids
        .iter()
        .filter(|id| excluded.contains(*id))
        .cloned()
        .collect();
    let consumed_block_ids: Vec<String> = nested_block_ids
        .iter()
        .filter(|id| !excluded.contains(*id))
        .filter(|id| {
            state
                .block(id)
                .is_some_and(|block| block.active && block.tier == target_tier)
        })
        .cloned()
        .collect();

    let mut effective: OrderedIds = range_message_ids.iter().map(String::as_str).collect();
    for block in consumed_block_ids.iter().filter_map(|id| state.block(id)) {
        for id in &block.effective_message_ids {
            effective.insert(id);
        }
    }
    let direct: Vec<String> = effective
        .iter()
        .filter(|id| !request.pre_existing_coverage.contains(*id))
        .map(str::to_owned)
        .collect();
    let filtered = carve::filter_protected_tool_messages(&direct, context);
    // Protected tool messages must leave the coverage too, or the block would
    // hide them.
    if filtered.len() < direct.len() {
        let kept: HashSet<&String> = filtered.iter().collect();
        for id in direct.iter().filter(|id| !kept.contains(id)) {
            effective.remove(id);
        }
    }

    let mut draft = Draft {
        range_message_ids,
        nested_block_ids,
        consumed_block_ids,
        effective,
        direct,
        filtered,
        withdrawn: Vec::new(),
        split_turn_count: 0,
        split_pair_count: 0,
        target_tier,
        output_tier,
        is_block_boundary,
        excluded_nested_block_ids,
    };
    if let Some(fatal) = carve::carve_protected_zone(&mut draft, context, request, warnings) {
        return Ok(draft.finish(Some(fatal)));
    }
    merge::integrity_gate(draft, context, request, excluded, warnings)
}

/// A message range is tier 1; a block range takes the lowest tier among
/// its nested blocks.
fn target_tier(state: &State, nested_block_ids: &[String], is_block_boundary: bool) -> u8 {
    if !is_block_boundary || nested_block_ids.is_empty() {
        return 1;
    }
    nested_block_ids
        .iter()
        .filter_map(|id| state.block(id))
        .map(|block| block.tier)
        .fold(3, u8::min)
}
