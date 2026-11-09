// Derived from billion-context (https://github.com/ranxianglei/billion-context),
// the range classification and dead-ref tracking of `applyCompression` in
// `kernel/src/compress.ts` of acp-kernel 0.0.189 at commit `4be21b08e`.
// Copyright (c) 2026 ranxianglei. MIT License with an additional attribution
// term (NOTICE).

use std::collections::HashSet;

use super::RangeSpec;
use crate::boundaries::{
    parse_boundary, resolve_boundaries, BoundaryError, BoundaryErrorKind, BoundaryKind,
    ResolvedRange,
};
use crate::model::{Message, State};
use crate::refs::index_to_ref;

/// One range resolved once: it goes on to the fold, or its refs exist but
/// are folded, never existed, or do not parse.
pub(super) enum Resolution {
    Ok(ResolvedRange),
    Consumed,
    Unknown(BoundaryError),
    Invalid(BoundaryError),
}

impl Resolution {
    pub(super) fn of(spec: &RangeSpec, messages: &[Message], state: &State) -> Self {
        match resolve_boundaries(&spec.start_ref, &spec.end_ref, messages, state) {
            Ok(resolved) => Self::Ok(resolved),
            Err(error) => match error.kind {
                BoundaryErrorKind::Consumed => Self::Consumed,
                BoundaryErrorKind::Unknown => Self::Unknown(error),
                BoundaryErrorKind::Invalid => Self::Invalid(error),
            },
        }
    }

    pub(super) fn resolved(&self) -> Option<&ResolvedRange> {
        match self {
            Self::Ok(resolved) => Some(resolved),
            _ => None,
        }
    }

    pub(super) fn is_consumed(&self) -> bool {
        matches!(self, Self::Consumed)
    }

    /// A range whose content is already folded: it may refold in place.
    pub(super) fn is_refold_candidate(&self, pre_existing: &HashSet<String>) -> bool {
        match self {
            Self::Consumed => true,
            Self::Ok(resolved) => {
                resolved.boundary_kind != BoundaryKind::Block
                    && resolved
                        .message_ids
                        .iter()
                        .all(|id| pre_existing.contains(id))
            }
            Self::Unknown(_) | Self::Invalid(_) => false,
        }
    }
}

pub(super) fn range_error(spec: &RangeSpec, message: &str) -> String {
    format!("range {}..{}: {message}", spec.start_ref, spec.end_ref)
}

/// The message refs of a consumed range that back no visible or folded
/// message: the client rewrote or dropped those messages.
pub(super) fn dangling_message_refs(
    state: &State,
    messages: &[Message],
    spec: &RangeSpec,
) -> Vec<String> {
    let visible: HashSet<&str> = messages.iter().map(|message| message.id.as_str()).collect();
    let mut dangling = Vec::new();
    for r#ref in [&spec.start_ref, &spec.end_ref] {
        let Some(boundary) =
            parse_boundary(r#ref).filter(|boundary| boundary.kind == BoundaryKind::Message)
        else {
            continue;
        };
        let padded = index_to_ref(boundary.numeric_id).unwrap_or_default();
        let by_ref = &state.message_refs.by_ref;
        let Some(raw_id) = by_ref.get(&boundary.raw).or_else(|| by_ref.get(&padded)) else {
            // A tombstone proves the number was allocated here.
            if state.dead_refs.contains(&boundary.raw) || state.dead_refs.contains(&padded) {
                dangling.push(padded);
            }
            continue;
        };
        if visible.contains(raw_id.as_str()) {
            continue;
        }
        let covered = state
            .active_blocks()
            .any(|block| block.effective_message_ids.contains(raw_id));
        if !covered {
            dangling.push(boundary.raw);
        }
    }
    dangling
}

/// The active blocks whose coverage holds a boundary of the range, a stale
/// block ref resolving to the block that now owns its content.
pub(super) fn covering_block_ids(state: &State, spec: &RangeSpec) -> Vec<String> {
    let mut found: Vec<String> = Vec::new();
    for r#ref in [&spec.start_ref, &spec.end_ref] {
        let Some(boundary) = parse_boundary(r#ref) else {
            continue;
        };
        let raw_ids: Vec<String> = match boundary.kind {
            BoundaryKind::Message => {
                let padded = index_to_ref(boundary.numeric_id).unwrap_or_default();
                let by_ref = &state.message_refs.by_ref;
                by_ref
                    .get(&boundary.raw)
                    .or_else(|| by_ref.get(&padded))
                    .cloned()
                    .into_iter()
                    .collect()
            }
            BoundaryKind::Block => state
                .block(&format!("b{}", boundary.numeric_id))
                .map(|block| block.effective_message_ids.clone())
                .unwrap_or_default(),
        };
        for candidate in state.active_blocks() {
            if raw_ids
                .iter()
                .any(|id| candidate.effective_message_ids.contains(id))
                && !found.contains(&candidate.block_id)
            {
                found.push(candidate.block_id.clone());
            }
        }
    }
    found.sort_by_key(|id| numeric_block_id(id));
    found
}

fn numeric_block_id(id: &str) -> u64 {
    id.strip_prefix('b')
        .filter(|digits| !digits.is_empty() && digits.bytes().all(|byte| byte.is_ascii_digit()))
        .and_then(|digits| digits.parse().ok())
        .unwrap_or(0)
}

/// Records the dead refs of the consumed ranges, lifting a tombstone whose
/// message is back in view; returns each consumed range's dangling refs.
pub(super) fn track_dead_refs(
    state: &mut State,
    messages: &[Message],
    ranges: &[RangeSpec],
    resolutions: &[Resolution],
) -> Vec<Vec<String>> {
    let visible: HashSet<&str> = messages.iter().map(|message| message.id.as_str()).collect();
    let mut dead: Vec<String> = state
        .dead_refs
        .iter()
        .filter(|r#ref| {
            !state
                .message_refs
                .by_ref
                .get(*r#ref)
                .is_some_and(|raw_id| visible.contains(raw_id.as_str()))
        })
        .cloned()
        .collect();
    let mut dangling_by_spec = Vec::with_capacity(ranges.len());
    for (spec, resolution) in ranges.iter().zip(resolutions) {
        let dangling = if resolution.is_consumed() {
            dangling_message_refs(state, messages, spec)
        } else {
            Vec::new()
        };
        for r#ref in &dangling {
            if !dead.contains(r#ref) {
                dead.push(r#ref.clone());
            }
        }
        dangling_by_spec.push(dangling);
    }
    let mut unique: Vec<String> = Vec::new();
    for r#ref in dead {
        if !unique.contains(&r#ref) {
            unique.push(r#ref);
        }
    }
    unique.sort_by_key(|r#ref| {
        r#ref
            .strip_prefix('m')
            .unwrap_or(r#ref)
            .parse::<u64>()
            .unwrap_or(u64::MAX)
    });
    state.dead_refs = unique;
    dangling_by_spec
}
