// Derived from billion-context (https://github.com/ranxianglei/billion-context),
// the minimum-range gate of `applyCompression` in `kernel/src/compress.ts`
// of acp-kernel 0.0.189 at commit `4be21b08e`. Copyright (c) 2026
// ranxianglei. MIT License with an additional attribution term (NOTICE).

use super::classify::{covering_block_ids, Resolution};
use super::refold::RefoldDecision;
use super::{ApplyInput, ApplyOutcome, ApplyResult, Batch};
use crate::boundaries::{BoundaryKind, Endpoint};
use crate::model::{Config, State};
use crate::refs::{highest_used_index, index_to_ref};
use crate::text::utf16_len;

/// Rejects a batch whose message ranges span fewer than
/// `min_compress_range` characters, with the reason the model can act on:
/// unknown refs, dead refs, content already folded, or too little of it. A
/// pure resubmit of folded ranges is a no-op instead. `None` lets the batch
/// on.
pub(super) fn size_gate(
    input: &ApplyInput<'_>,
    state: &State,
    batch: &Batch,
) -> Option<ApplyOutcome> {
    let config = input.config;
    let minimum = config.compress.min_compress_range;
    if minimum == 0 || input.ranges.is_empty() {
        return None;
    }
    let mut total_chars = 0;
    let mut has_block_range = false;
    let mut counted = 0;
    for (index, resolution) in batch.resolutions.iter().enumerate() {
        let Some(resolved) = resolution.resolved().filter(|_| !batch.skipped[index]) else {
            continue;
        };
        if resolved.boundary_kind == BoundaryKind::Block {
            has_block_range = true;
            continue;
        }
        counted += 1;
        for id in &resolved.message_ids {
            if let Some(message) = input.messages.iter().find(|message| &message.id == id) {
                total_chars += utf16_len(&message.text);
            }
        }
    }
    if batch.all_refold || has_block_range || total_chars >= minimum {
        return None;
    }

    let consumed: Vec<usize> = (0..input.ranges.len())
        .filter(|index| batch.resolutions[*index].is_consumed())
        .collect();
    let dangling: Vec<String> = consumed
        .iter()
        .flat_map(|index| batch.dangling[*index].iter().cloned())
        .collect();
    let Some(mut message) = gate_message(
        input,
        state,
        batch,
        &consumed,
        &dangling,
        total_chars,
        counted,
    ) else {
        // Every range failed to resolve: the per-range errors name the real
        // cause, where "too small" would mislead.
        return Some(ApplyOutcome {
            state: input.state.clone(),
            result: ApplyResult {
                errors: batch.classification_errors.clone(),
                ..ApplyResult::default()
            },
        });
    };
    let reversal_notes: Vec<&str> = batch
        .resolutions
        .iter()
        .enumerate()
        .filter(|(index, _)| !batch.skipped[*index])
        .filter_map(|(_, resolution)| resolution.resolved()?.reversed_note.as_deref())
        .collect();
    if !reversal_notes.is_empty() {
        message = format!("{message} {}", reversal_notes.join(" "));
    }

    // Resending ranges that are all folded already is an idempotent retry,
    // not a failure.
    let pure_resubmit =
        counted == 0 && !consumed.is_empty() && dangling.is_empty() && batch.unknown_count == 0;
    if pure_resubmit {
        return Some(ApplyOutcome {
            state: state.clone(),
            result: ApplyResult {
                warnings: vec![format!(
                    "{message} Nothing failed and nothing needed redoing — do not resend these ranges."
                )],
                noop: true,
                ..ApplyResult::default()
            },
        });
    }
    let mut errors = vec![message];
    errors.extend(batch.classification_errors.iter().cloned());
    Some(ApplyOutcome {
        state: state.clone(),
        result: ApplyResult {
            errors,
            ..ApplyResult::default()
        },
    })
}

fn gate_message(
    input: &ApplyInput<'_>,
    state: &State,
    batch: &Batch,
    consumed: &[usize],
    dangling: &[String],
    total_chars: usize,
    counted: usize,
) -> Option<String> {
    let ranges = input.ranges;
    let diagnostics = ref_gate_diagnostics(state, ranges.len(), batch.unknown_count);
    if batch.resolvable_count == 0 && consumed.is_empty() && batch.unknown_count > 0 {
        let mut endpoints: Vec<String> = Vec::new();
        for (spec, resolution) in ranges.iter().zip(&batch.resolutions) {
            let Resolution::Unknown(error) = resolution else {
                continue;
            };
            let endpoint = match error.endpoint {
                Some(Endpoint::End) => format!("endId={}", spec.end_ref),
                _ => format!("startId={}", spec.start_ref),
            };
            if !endpoints.contains(&endpoint) {
                endpoints.push(endpoint);
            }
        }
        let failing = if endpoints.is_empty() {
            String::new()
        } else {
            format!(", failing endpoints: {}", endpoints.join(", "))
        };
        return Some(format!(
            "None of the {} requested range(s) resolved — every ref is unknown to this session{failing}. Refs are per-session snapshots, assigned once when a message is first rendered; no compress reassigns them. An unknown ref is usually a stale artifact from an earlier generation (switching model or upstream mid-conversation starts a fresh session whose refs restart at m00001, and a native-compaction rebase resets them the same way) or a typo — but a number at or below the highest allocated ref may also simply be an unallocated gap: the ref space is sparse. {diagnostics} Do not probe for valid numbers. Run acp_status, then call the compress tool again using only the refs it reports.",
            ranges.len(),
        ));
    }
    if let Some(first) = consumed.first().map(|index| &ranges[*index]) {
        if !dangling.is_empty() {
            return Some(format!(
                "Requested range(s) cannot be anchored (e.g. {}..{}): refs {} are known to this session but no longer back any visible or folded message — the client rewrote or dropped those messages (an edit reissues a new ref; host-native compaction or a bulk history rewrite drops them outright), and no active block covers them. These refs are now recorded as DEAD: no range that includes them can compress now or later, whatever the neighbors. Do not retry this range in any form — run acp_status and target only the live refs it reports. {diagnostics}",
                first.start_ref,
                first.end_ref,
                dangling.join(", "),
            ));
        }
        let covering = covering_block_ids(state, first);
        let cover_detail = match covering.as_slice() {
            [] => "its refs no longer point to directly compressible content (stale block ref(s) distilled or consumed by higher-tier blocks)".to_owned(),
            [only] => format!("its content is already summarized in active block(s) {only} — use search_context or decompress {only} if you need details from it"),
            many => format!("its content is already summarized in active block(s) {}", many.join(", ")),
        };
        let reasons = blocked_reasons(batch, consumed.iter().copied());
        return Some(format!(
            "Requested range(s) already compressed (e.g. {}..{}) — {cover_detail}{}. Nothing new to compress in that window. {diagnostics} Continue the task, or run acp_status and target one of the CURRENT compressible ranges it reports.{}",
            first.start_ref,
            first.end_ref,
            refold_suffix(&reasons),
            tier_action_hint(input.config, state),
        ));
    }
    if counted > 0 {
        let ok_ranges = (0..ranges.len()).filter(|index| {
            !batch.skipped[*index] && batch.resolutions[*index].resolved().is_some()
        });
        let reasons = blocked_reasons(batch, ok_ranges);
        return Some(format!(
            "Total compressible content too small ({total_chars} chars across {counted} range(s), min {}). Combine more messages into your range(s) to meet the threshold.{}",
            input.config.compress.min_compress_range,
            refold_suffix(&reasons),
        ));
    }
    None
}

/// The refold blockers of the given ranges, each once.
fn blocked_reasons(batch: &Batch, ranges: impl Iterator<Item = usize>) -> Vec<String> {
    let mut reasons: Vec<String> = Vec::new();
    for index in ranges {
        let Some(RefoldDecision::Blocked(blocked)) = &batch.refold[index] else {
            continue;
        };
        for reason in blocked {
            if !reasons.contains(reason) {
                reasons.push(reason.clone());
            }
        }
    }
    reasons
}

fn refold_suffix(reasons: &[String]) -> String {
    if reasons.is_empty() {
        return String::new();
    }
    format!(
        " Refold blocked: {}. Restore the affected block(s) inline (decompress with inline:true), then recompressing the same range updates them in place",
        reasons.join("; ")
    )
}

fn ref_gate_diagnostics(state: &State, requested: usize, unknown: usize) -> String {
    let highest = highest_used_index(&state.message_refs);
    let highest_ref = index_to_ref(highest).unwrap_or_else(|| "none".to_owned());
    format!(
        "[diagnostics: session highest allocated ref={highest_ref} (the ref space is sparse — numbers at or below it are NOT necessarily allocated, so probing numbers will not find a valid one; use only refs acp_status lists), unknown ranges in request={unknown}/{requested}, session history={} compression(s), {} block(s)]",
        state.stats.compression_count,
        state.blocks.len(),
    )
}

/// Block-range advice, only when distillation is due.
fn tier_action_hint(config: &Config, state: &State) -> String {
    if !config.tiers.enabled {
        return String::new();
    }
    let of_tier = |tier: u8| -> Vec<&str> {
        state
            .active_blocks()
            .filter(|block| block.tier == tier)
            .map(|block| block.block_id.as_str())
            .collect()
    };
    let hint = |blocks: &[&str], from: u8, to: u8| {
        match (blocks.first(), blocks.last()) {
        (Some(first), Some(last)) => format!(
            " Tier {} is actionable now: compress({{ content: [{{ startId: \"{first}\", endId: \"{last}\", summary: \"...\", topic: \"...\" }}] }}) merges those tier-{from} blocks into one tier-{to} block.",
            if to == 3 { "condensation" } else { "distillation" },
        ),
        _ => String::new(),
    }
    };
    let tier2 = of_tier(2);
    if tier2.len() >= config.tiers.tier3_trigger {
        return hint(&tier2, 2, 3);
    }
    let tier1 = of_tier(1);
    if tier1.len() >= config.tiers.tier2_trigger {
        return hint(&tier1, 1, 2);
    }
    String::new()
}
