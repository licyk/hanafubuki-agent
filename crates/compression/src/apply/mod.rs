// Derived from billion-context (https://github.com/ranxianglei/billion-context),
// `applyCompression` of `kernel/src/compress.ts` of acp-kernel 0.0.189 at
// commit `4be21b08e`. Copyright (c) 2026 ranxianglei. MIT License with an
// additional attribution term (NOTICE).
// Modified for hanafubuki-agent: the caller passes the time, and Rust
// strings need no lone-surrogate scrub.

mod classify;
mod gate;
mod refold;
mod single;

use std::collections::HashSet;

pub use refold::evaluate_refold;
pub use refold::RefoldDecision;

use crate::model::{Config, Message, State};
use crate::protect::compute_protected_refs;
use crate::refs::ref_to_index;
use classify::{range_error, track_dead_refs, Resolution};
use single::{apply_refolds, apply_single_range, validate_summary_length, SingleRange};

/// One range of a `compress` call.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RangeSpec {
    /// `mNNNNN` or `bN`.
    pub start_ref: String,
    pub end_ref: String,
    pub summary: String,
    pub topic: Option<String>,
    /// The id of the `compress` tool call that carries the summary.
    pub compress_call_id: Option<String>,
    /// Overrides the configured maximum summary length.
    pub summary_max_chars: Option<usize>,
}

pub struct ApplyInput<'a> {
    pub ranges: &'a [RangeSpec],
    /// The full history, not a pruned view.
    pub messages: &'a [Message],
    pub state: &'a State,
    pub config: &'a Config,
    /// The recent zone's refs; computed from `messages` when `None`.
    pub protected_refs: Option<&'a HashSet<String>>,
    /// Milliseconds since the Unix epoch, recorded on new blocks.
    pub now_ms: u64,
}

/// What a call did; its texts are written for the model. Zero blocks with
/// no error is a no-op (`noop`), which the host reports with a
/// `[Compression no-op` receipt.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ApplyResult {
    pub blocks_created: usize,
    pub tokens_compressed: u64,
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
    pub notes: Vec<String>,
    pub noop: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ApplyOutcome {
    pub state: State,
    pub result: ApplyResult,
}

/// The batch's ranges as classified once, before anything folds.
struct Batch {
    resolutions: Vec<Resolution>,
    /// Ranges overlapping an earlier one, skipped.
    skipped: Vec<bool>,
    /// The refold verdict of each range over folded content.
    refold: Vec<Option<RefoldDecision>>,
    /// Each range's dead refs.
    dangling: Vec<Vec<String>>,
    /// Every range refolds in place.
    all_refold: bool,
    resolvable_count: usize,
    unknown_count: usize,
    classification_errors: Vec<String>,
}

/// Applies one `compress` call all or nothing: every range is resolved
/// first, overlaps are skipped, a batch too small or with a bad summary is
/// rejected before anything folds, and a range failing while folding
/// rejects the whole batch. On success the nudge baselines, the truncation
/// latch, the deferred tags and the view latches reset, since a fold is the
/// one moment the view may be rewritten.
pub fn apply_compression(input: &ApplyInput<'_>) -> ApplyOutcome {
    let mut state = input.state.clone();
    let config = input.config;
    let computed_zone;
    let zone = match input.protected_refs {
        Some(zone) => zone,
        None => {
            computed_zone = compute_protected_refs(input.messages, input.state, config);
            &computed_zone
        }
    };
    let pre_existing: HashSet<String> = state
        .covered_message_ids()
        .into_iter()
        .map(str::to_owned)
        .collect();
    let mut warnings = Vec::new();
    let batch = classify_batch(input, &mut state, &pre_existing, &mut warnings);

    if let Some(rejected) = gate::size_gate(input, &state, &batch) {
        return rejected;
    }
    if let Some(rejected) = length_precheck(input, &state, &batch) {
        return rejected;
    }

    // Folding a range can depend on the one before it, so the loop keeps the
    // state it started from: any failure returns it, and nothing folds.
    let pre_loop_state = state.clone();
    let warnings_base = warnings.len();
    let pre_fold_blocks: HashSet<String> = state
        .blocks
        .iter()
        .map(|block| block.block_id.clone())
        .collect();
    let mut run_id: Option<String> = None;
    let mut result = ApplyResult::default();
    let mut apply_errors = Vec::new();
    for (index, spec) in input.ranges.iter().enumerate() {
        if batch.skipped[index] {
            continue;
        }
        match &batch.resolutions[index] {
            Resolution::Consumed => {
                if let Some(RefoldDecision::Refold(blocks)) = &batch.refold[index] {
                    let run = run_id
                        .get_or_insert_with(|| state.allocate_run_id())
                        .clone();
                    match apply_refolds(&mut state, spec, &run, config, blocks) {
                        Ok(()) => result.blocks_created += blocks.len(),
                        Err(error) => apply_errors.push(range_error(spec, &error)),
                    }
                    continue;
                }
                warnings.push(skipped_consumed_warning(spec, &batch, index));
            }
            Resolution::Unknown(error) | Resolution::Invalid(error) => {
                result.errors.push(range_error(spec, &error.message));
            }
            Resolution::Ok(resolved) => {
                warnings.extend(resolved.snapped_boundaries.iter().cloned());
                if let Some(note) = &resolved.reversed_note {
                    result.notes.push(note.clone());
                }
                let run = run_id
                    .get_or_insert_with(|| state.allocate_run_id())
                    .clone();
                let range = SingleRange {
                    spec,
                    messages: input.messages,
                    run_id: run,
                    config,
                    zone,
                    pre_existing: &pre_existing,
                    now_ms: input.now_ms,
                };
                match apply_single_range(&mut state, range) {
                    Ok(outcome) => {
                        result.blocks_created += match (&outcome.refolded, outcome.noop) {
                            (_, true) => 0,
                            (Some(refolded), false) => refolded.len(),
                            (None, false) => 1,
                        };
                        result.tokens_compressed += outcome.tokens;
                        warnings.extend(outcome.warnings);
                    }
                    Err(error) => apply_errors.push(range_error(spec, &error)),
                }
            }
        }
    }

    if !apply_errors.is_empty() {
        let mut errors = vec![format!(
            "Batch rejected before folding any range — {} of {} requested range(s) failed during application: {}. Nothing was folded, so no prefix was rewritten. Fix or drop the offending range(s), then resend the whole batch.",
            apply_errors.len(),
            input.ranges.len(),
            apply_errors.join(" | "),
        )];
        errors.extend(batch.classification_errors.iter().cloned());
        warnings.truncate(warnings_base);
        return ApplyOutcome {
            state: pre_loop_state,
            result: ApplyResult {
                errors,
                warnings,
                ..ApplyResult::default()
            },
        };
    }

    state.stats.compression_count += result.blocks_created as u64;
    state.stats.tokens_compressed += result.tokens_compressed;
    if result.blocks_created > 0 {
        reset_after_fold(&mut state, &pre_fold_blocks, result.tokens_compressed);
    }
    result.warnings = warnings;
    result.noop = result.blocks_created == 0 && result.errors.is_empty();
    ApplyOutcome { state, result }
}

fn classify_batch(
    input: &ApplyInput<'_>,
    state: &mut State,
    pre_existing: &HashSet<String>,
    warnings: &mut Vec<String>,
) -> Batch {
    let resolutions: Vec<Resolution> = input
        .ranges
        .iter()
        .map(|spec| Resolution::of(spec, input.messages, state))
        .collect();
    let mut classification_errors = Vec::new();
    let (mut resolvable_count, mut unknown_count) = (0, 0);
    for (spec, resolution) in input.ranges.iter().zip(&resolutions) {
        match resolution {
            Resolution::Ok(_) => resolvable_count += 1,
            Resolution::Unknown(error) => {
                unknown_count += 1;
                classification_errors.push(range_error(spec, &error.message));
            }
            Resolution::Invalid(error) => {
                classification_errors.push(range_error(spec, &error.message))
            }
            Resolution::Consumed => {}
        }
    }
    // A dead ref backs no visible or folded message: every later receipt
    // says so instead of inviting another doomed retry.
    let dangling = track_dead_refs(state, input.messages, input.ranges, &resolutions);

    // Refolds are judged before the size gate, so a pure refold never trips
    // it.
    let refold: Vec<Option<RefoldDecision>> = input
        .ranges
        .iter()
        .zip(&resolutions)
        .map(|(spec, resolution)| {
            resolution
                .is_refold_candidate(pre_existing)
                .then(|| refold::evaluate_refold(state, &spec.start_ref, &spec.end_ref))
        })
        .collect();
    let all_refold = !input.ranges.is_empty()
        && unknown_count == 0
        && refold
            .iter()
            .all(|decision| matches!(decision, Some(RefoldDecision::Refold(_))));

    // Overlapping ranges: the earliest wins, the later ones are skipped.
    let mut spans: Vec<(usize, usize, usize)> = resolutions
        .iter()
        .enumerate()
        .filter_map(|(index, resolution)| {
            let resolved = resolution.resolved()?;
            Some((resolved.start_index, resolved.end_index, index))
        })
        .collect();
    spans.sort_by_key(|(start, _, _)| *start);
    let mut skipped = vec![false; input.ranges.len()];
    let mut accepted_max: Option<usize> = None;
    for (start, end, index) in spans {
        if accepted_max.is_some_and(|max| start <= max) {
            skipped[index] = true;
            let spec = &input.ranges[index];
            warnings.push(format!(
                "Skipped range ({}..{}) — overlaps an earlier range in the batch; the earlier range takes precedence. Keep ranges disjoint.",
                spec.start_ref, spec.end_ref
            ));
            continue;
        }
        accepted_max = Some(accepted_max.map_or(end, |max| max.max(end)));
    }

    Batch {
        resolutions,
        skipped,
        refold,
        dangling,
        all_refold,
        resolvable_count,
        unknown_count,
        classification_errors,
    }
}

/// Summary lengths depend on nothing but the summary, so they are checked
/// for every range that would fold before any does: a retry then costs one
/// rewritten prefix, not two.
fn length_precheck(input: &ApplyInput<'_>, state: &State, batch: &Batch) -> Option<ApplyOutcome> {
    let failures: Vec<String> = input
        .ranges
        .iter()
        .enumerate()
        .filter(|(index, _)| !batch.skipped[*index])
        .filter(|(index, _)| {
            matches!(batch.resolutions[*index], Resolution::Ok(_))
                || (batch.resolutions[*index].is_consumed()
                    && matches!(batch.refold[*index], Some(RefoldDecision::Refold(_))))
        })
        .filter_map(|(_, spec)| {
            validate_summary_length(spec, &input.config.compress)
                .err()
                .map(|error| range_error(spec, &error))
        })
        .collect();
    if failures.is_empty() {
        return None;
    }
    let mut errors = vec![format!(
        "Batch rejected before folding any range — {} of {} requested range(s) failed summary-length validation: {}. Nothing was folded, so no prefix was rewritten. Fix it one way or the other and resend: shorten or split each offending range's summary to fit its cap, then resend the whole batch; or drop the offending range(s) and resend only the rest that validate.",
        failures.len(),
        input.ranges.len(),
        failures.join(" | "),
    )];
    errors.extend(batch.classification_errors.iter().cloned());
    Some(ApplyOutcome {
        state: state.clone(),
        result: ApplyResult {
            errors,
            ..ApplyResult::default()
        },
    })
}

fn skipped_consumed_warning(spec: &RangeSpec, batch: &Batch, index: usize) -> String {
    let dangling = &batch.dangling[index];
    if !dangling.is_empty() {
        return format!(
            "Skipped range ({}..{}) — its refs {} are DEAD (the client rewrote or dropped those messages); this range can never compress, do not retry it.",
            spec.start_ref,
            spec.end_ref,
            dangling.join(", "),
        );
    }
    let detail = match &batch.refold[index] {
        Some(RefoldDecision::Blocked(reasons)) if !reasons.is_empty() => {
            format!(": {}", reasons.join("; "))
        }
        _ => " (messages consumed by existing block(s))".to_owned(),
    };
    format!(
        "Skipped range ({}..{}) — already compressed{detail}; nothing to compress.",
        spec.start_ref, spec.end_ref
    )
}

/// After a fold: where the orphan sweep starts, fresh nudge baselines, the
/// tier-1 harvest-stall verdict, and the latches released.
fn reset_after_fold(state: &mut State, pre_fold_blocks: &HashSet<String>, tokens_compressed: u64) {
    let new_blocks = || {
        state
            .blocks
            .iter()
            .filter(|block| !pre_fold_blocks.contains(&block.block_id))
    };
    // The lowest ref the fold covered, never a block ref's digits: sweeping
    // from a guess would rewrite the cached prefix.
    let fold_start = new_blocks()
        .flat_map(|block| &block.effective_message_ids)
        .filter_map(|id| state.message_refs.by_raw.get(id))
        .filter_map(|r#ref| ref_to_index(r#ref))
        .min();
    let distilled = new_blocks().any(|block| block.tier > 1);
    if let Some(fold_start) = fold_start {
        state.pending_orphan_sweep_min_ref = Some(
            state
                .pending_orphan_sweep_min_ref
                .map_or(fold_start, |pending| pending.min(fold_start)),
        );
    }
    let nudge = &mut state.nudge;
    nudge.last_per_message_nudge_tokens = 0;
    nudge.last_nudge_shown_tokens = 0;
    nudge.last_shown_by_tier.clear();
    nudge.absorbed_debt = 0;
    nudge.absorb_count = 0;
    // A tier-1 fold taking less than half of what the last tier-1 nudge
    // offered means that channel is not draining.
    if distilled {
        nudge.t1_harvest_stall = false;
        nudge.last_t1_fire_pending = None;
    } else if let Some(pending) = nudge.last_t1_fire_pending.filter(|pending| *pending > 0) {
        nudge.t1_harvest_stall = (tokens_compressed as f64) < STALL_HARVEST_RATIO * pending as f64;
        nudge.last_t1_fire_pending = None;
    }
    state.terminal_streak = 0;
    state.emergency_truncate_latch.clear();
    state.render_deferred_refs.clear();
    state.view_latches.clear();
}

/// The share of a tier-1 nudge's offer a fold must take for the tier-1
/// channel to count as draining.
pub(crate) const STALL_HARVEST_RATIO: f64 = 0.5;
