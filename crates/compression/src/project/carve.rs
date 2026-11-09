// Derived from billion-context (https://github.com/ranxianglei/billion-context),
// the protected-tool filter and the recent-zone carve of
// `kernel/src/unit-projection.ts` of acp-kernel 0.0.189 at commit
// `4be21b08e`. Copyright (c) 2026 ranxianglei. MIT License with an
// additional attribution term (NOTICE).

use std::collections::HashSet;

use super::{Draft, Fatal, FatalKind, ProjectionContext, UnitRequest};
use crate::model::Kind;
use crate::protect::{is_message_latest_protected, is_message_protected};
use crate::refs::BLOCKED_REF;
use crate::text::{clamp_prefix, collapse_whitespace};

/// `ids` without the protected tool calls and their results, which stay
/// visible. Failed `compress` exchanges and all but the newest `compress`
/// exchanges are not protected here.
pub(super) fn filter_protected_tool_messages(
    ids: &[String],
    context: &ProjectionContext<'_>,
) -> Vec<String> {
    let mut protected_call_ids = context.base_protected_call_ids.clone();
    let mut removed: HashSet<&str> = HashSet::new();
    let is_exempt = |call_id: Option<&String>| {
        call_id.is_some_and(|call_id| context.aged_compress_call_ids.contains(call_id))
    };
    for id in ids {
        let Some(message) = context.message(id) else {
            continue;
        };
        let protected =
            is_message_protected(message, context.config, &context.failed_compress_call_ids)
                && !is_exempt(message.tool_call_id.as_ref());
        if protected || is_message_latest_protected(message, &context.latest) {
            removed.insert(id);
            if let Some(call_id) = &message.tool_call_id {
                protected_call_ids.insert(call_id.clone());
            }
        }
    }
    for id in ids {
        if removed.contains(id.as_str()) {
            continue;
        }
        let Some(message) = context.message(id) else {
            continue;
        };
        if message.kind == Kind::ToolResult
            && message
                .tool_call_id
                .as_ref()
                .is_some_and(|call_id| protected_call_ids.contains(call_id))
        {
            removed.insert(id);
        }
    }
    ids.iter()
        .filter(|id| !removed.contains(id.as_str()))
        .cloned()
        .collect()
}

/// The recent zone is advisory when folding: its messages leave the fold
/// and stay visible, and the rest folds. A protected message inside the
/// range ends the fold there, so no summary lands above the question it
/// answers. Returns the fatal when nothing is left.
pub(super) fn carve_protected_zone(
    draft: &mut Draft,
    context: &ProjectionContext<'_>,
    request: &UnitRequest<'_>,
    warnings: &mut Vec<String>,
) -> Option<Fatal> {
    let zone = request.protected_refs?;
    let by_raw = &context.state.message_refs.by_raw;
    let hit: Vec<String> = draft
        .filtered
        .iter()
        .filter(|id| by_raw.get(*id).is_some_and(|r#ref| zone.contains(r#ref)))
        .cloned()
        .collect();
    if hit.is_empty() {
        return None;
    }
    let hit_set: HashSet<&String> = hit.iter().collect();

    let mut truncated_after = 0;
    if let Some(first) = draft.filtered.iter().position(|id| hit_set.contains(id)) {
        let truncatable = draft.filtered.len() - first - hit.len();
        // Truncate only when what is kept still folds something and no tool
        // pair straddles the cut; otherwise only carve the zone out.
        let dropped: HashSet<&String> = draft.filtered[first..].iter().collect();
        let strands_pair = draft.filtered[..first].iter().any(|id| {
            let Some(message) = context.message(id).filter(|message| message.is_tool()) else {
                return false;
            };
            let key = message.tool_call_id.as_ref().unwrap_or(&message.id);
            context
                .pair_by_call_id
                .get(key)
                .is_some_and(|(call, result)| {
                    [call, result]
                        .into_iter()
                        .any(|half| !half.is_empty() && half != id && dropped.contains(half))
                })
        });
        if truncatable > 0 && first > 0 && !strands_pair {
            truncated_after = truncatable;
            for id in draft.filtered.split_off(first) {
                draft.effective.remove(&id);
            }
        }
    }
    draft.filtered.retain(|id| !hit_set.contains(id));
    for id in &hit {
        draft.effective.remove(id);
    }
    let hit_refs: Vec<&str> = hit
        .iter()
        .filter_map(|id| by_raw.get(id).map(String::as_str))
        .collect();

    if draft.filtered.is_empty() && draft.consumed_block_ids.is_empty() {
        return Some(zone_fatal(draft, context, request, &hit_refs));
    }
    let anchor_note = context
        .anchor_message
        .and_then(|anchor| Some((anchor, by_raw.get(&anchor.id)?)))
        .filter(|(_, r#ref)| *r#ref != BLOCKED_REF)
        .map(|(anchor, anchor_ref)| {
            let text = collapse_whitespace(&anchor.text);
            format!(
                " Last-user anchor: {anchor_ref} \"{}\".",
                clamp_prefix(&text, 40)
            )
        })
        .unwrap_or_default();
    let truncation = if truncated_after > 0 {
        format!(" Range truncated at that boundary — {truncated_after} later message(s) (including replies to the protected message) stay visible too, so no summary can land above the question it answers (#3953).")
    } else {
        String::new()
    };
    warnings.push(format!(
        "Excluded {} protected message(s) {} from compression range (recent/last-user zone) — they stay visible outside the new block; do not target them in another compress call.{anchor_note}{truncation}",
        hit.len(),
        hit_refs.join(", "),
    ));
    None
}

/// Nothing is left once the zone is carved out: a no-op, unless the merge
/// ladder excluded every nested block, which stays an error.
fn zone_fatal(
    draft: &Draft,
    context: &ProjectionContext<'_>,
    request: &UnitRequest<'_>,
    hit_refs: &[&str],
) -> Fatal {
    if !draft.excluded_nested_block_ids.is_empty() {
        return Fatal {
            kind: FatalKind::Split,
            message: format!(
                "Merging {} cannot proceed: excluding {} (their coverage would strand a live turn-integrity conflict) leaves nothing left to fold in {}..{}. Fold the stranded visible partner(s) first with a plain message-range compress so the turn closes, or retry once those block(s) can fold whole.",
                draft.nested_block_ids.join(", "),
                draft.excluded_nested_block_ids.join(", "),
                request.start_ref,
                request.end_ref,
            ),
        };
    }
    Fatal {
        kind: FatalKind::Zone,
        message: format!(
            "Range is entirely within the protected zone (the last {} messages and/or the most recent user message): {}. Adjust startId/endId to older messages.",
            context.config.preserve_recent_messages,
            hit_refs.join(", "),
        ),
    }
}
