// Derived from billion-context (https://github.com/ranxianglei/billion-context),
// `applySingleRange`, `applyRefolds` and the summary checks of
// `kernel/src/compress.ts` of acp-kernel 0.0.189 at commit `4be21b08e`.
// Copyright (c) 2026 ranxianglei. MIT License with an additional attribution
// term (NOTICE).
// Modified for hanafubuki-agent: the creation time is the caller's, and a
// summary carries no MEDIA REFS section yet.

use std::collections::HashSet;

use super::refold::{evaluate_refold, RefoldDecision};
use super::RangeSpec;
use crate::model::{Block, CompressLimits, Config, Generation, Message, State};
use crate::project::{project_unit, FatalKind, ProjectionContext, UnitRequest};
use crate::refs::{highest_used_index, index_to_ref};
use crate::render::{count_message_tokens, default_count_tokens};
use crate::text::utf16_len;

pub(super) struct SingleRange<'a> {
    pub(super) spec: &'a RangeSpec,
    pub(super) messages: &'a [Message],
    pub(super) run_id: String,
    pub(super) config: &'a Config,
    pub(super) zone: &'a HashSet<String>,
    pub(super) pre_existing: &'a HashSet<String>,
    pub(super) now_ms: u64,
}

#[derive(Default)]
pub(super) struct SingleOutcome {
    pub(super) tokens: u64,
    pub(super) warnings: Vec<String>,
    /// The blocks updated in place instead of a new one.
    pub(super) refolded: Option<Vec<String>>,
    /// Legal, but nothing folded.
    pub(super) noop: bool,
}

impl SingleOutcome {
    fn noop(warnings: Vec<String>) -> Self {
        Self {
            warnings,
            noop: true,
            ..Self::default()
        }
    }
}

/// Folds one resolved range into a new block, or updates restored blocks
/// in place; a range with nothing to fold is a no-op with its reason.
pub(super) fn apply_single_range(
    state: &mut State,
    range: SingleRange<'_>,
) -> Result<SingleOutcome, String> {
    let spec = range.spec;
    let mut warnings = Vec::new();
    let projection = {
        let context = ProjectionContext::new(range.messages, state, range.config);
        let request = UnitRequest {
            start_ref: &spec.start_ref,
            end_ref: &spec.end_ref,
            pre_existing_coverage: range.pre_existing,
            protected_refs: Some(range.zone),
        };
        project_unit(&context, &request, &mut warnings).map_err(|error| error.message)?
    };
    if let Some(fatal) = projection.fatal {
        // The newest tail the model just produced has nothing it may fold.
        if fatal.kind == FatalKind::Zone {
            return Ok(SingleOutcome::noop(vec![format!(
                "Range {}..{} folded nothing — it lies entirely inside the protected zone (the last {} messages and/or the most recent user message), which always stays visible. Nothing failed; do not resend this range. Session highest allocated ref is {} — target older ranges only.",
                spec.start_ref,
                spec.end_ref,
                range.config.preserve_recent_messages,
                highest_ref(state),
            )]));
        }
        return Err(fatal.message);
    }

    // A message range whose content blocks already own adds nothing: a new
    // block would be an empty rewrite reported as success, and the model
    // would repeat it forever.
    let consumed = &projection.consumed_block_ids;
    if !projection.is_block_boundary && projection.filtered_ids.is_empty() && !consumed.is_empty() {
        if let RefoldDecision::Refold(blocks) =
            evaluate_refold(state, &spec.start_ref, &spec.end_ref)
        {
            apply_refolds(state, spec, &range.run_id, range.config, &blocks)?;
            return Ok(SingleOutcome {
                warnings,
                refolded: Some(blocks),
                ..SingleOutcome::default()
            });
        }
        let (first, last) = (&consumed[0], &consumed[consumed.len() - 1]);
        return Ok(SingleOutcome::noop(vec![format!(
            "Range {}..{} folded nothing — every message in it is already covered by active block(s) {}. Nothing failed and nothing needed redoing; do not resend this range. To rewrite or merge those blocks, reference them by block ID ({first}..{last}) instead.",
            spec.start_ref,
            spec.end_ref,
            consumed.join(", "),
        )]));
    }

    validate_summary_length(spec, &range.config.compress)?;

    if projection.filtered_ids.is_empty() && consumed.is_empty() {
        warnings.push(format!(
            "Range {}..{} folded nothing — all of its content is protected or filtered (protected tool calls/results, media, or the recent-message zone), which always stays visible. Nothing failed; do not resend this range. Session highest allocated ref is {}.",
            spec.start_ref,
            spec.end_ref,
            highest_ref(state),
        ));
        return Ok(SingleOutcome::noop(warnings));
    }

    let mut compressed_tokens: u64 = projection
        .filtered_ids
        .iter()
        .filter_map(|id| range.messages.iter().find(|message| &message.id == id))
        .map(count_message_tokens)
        .sum();
    compressed_tokens += consumed
        .iter()
        .filter_map(|id| state.block(id))
        .map(|block| default_count_tokens(&block.summary))
        .sum::<u64>();

    let block_id = state.allocate_block_id();
    state.blocks.push(Block {
        block_id,
        run_id: range.run_id,
        tier: projection.output_tier,
        topic: spec.topic.clone(),
        summary: spec.summary.clone(),
        direct_message_ids: projection.filtered_ids,
        effective_message_ids: projection.effective_message_ids,
        direct_block_ids: consumed.clone(),
        compressed_tokens,
        created_at: range.now_ms,
        survived_count: 0,
        generation: Generation::Young,
        active: true,
        expanded: false,
        restored_inline: false,
        compress_call_id: spec.compress_call_id.clone(),
        start_ref: Some(spec.start_ref.clone()),
        end_ref: Some(spec.end_ref.clone()),
    });
    for id in &projection.consumed_block_ids {
        if let Some(block) = state.block_mut(id) {
            block.active = false;
        }
    }
    Ok(SingleOutcome {
        tokens: compressed_tokens,
        warnings,
        ..SingleOutcome::default()
    })
}

/// Updates restored blocks in place: same id, coverage and tier; the new
/// summary (and topic), a new run id, no longer restored.
pub(super) fn apply_refolds(
    state: &mut State,
    spec: &RangeSpec,
    run_id: &str,
    config: &Config,
    block_ids: &[String],
) -> Result<(), String> {
    validate_summary_length(spec, &config.compress)?;
    for block in state
        .blocks
        .iter_mut()
        .filter(|block| block_ids.contains(&block.block_id))
    {
        block.summary = spec.summary.clone();
        if spec.topic.is_some() {
            block.topic = spec.topic.clone();
        }
        block.run_id = run_id.to_owned();
        block.restored_inline = false;
    }
    Ok(())
}

/// The summary is not empty and fits its limits, in UTF-16 code units of
/// its trimmed text; `summary_max_chars` overrides the maximum.
pub(super) fn validate_summary_length(
    spec: &RangeSpec,
    limits: &CompressLimits,
) -> Result<(), String> {
    let length = utf16_len(spec.summary.trim());
    if length == 0 {
        return Err(
            "Summary is empty — provide a meaningful summary of the compressed range.".to_owned(),
        );
    }
    let minimum = limits.min_summary_length;
    if minimum > 0 && length < minimum {
        return Err(format!(
            "Summary too short ({length} chars, min {minimum}). The summary must capture the compressed range's key information."
        ));
    }
    let maximum = spec.summary_max_chars.unwrap_or(limits.max_summary_length);
    if maximum > 0 && length > maximum {
        return Err(format!(
            "Summary too long ({length} chars, max {maximum}). Strip noise — keep critical paths, decisions, errors, and code references. Or pass summaryMaxChars to increase the limit — don't lose critical info just to fit."
        ));
    }
    Ok(())
}

fn highest_ref(state: &State) -> String {
    index_to_ref(highest_used_index(&state.message_refs)).unwrap_or_else(|| "none".to_owned())
}
