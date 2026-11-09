// Derived from billion-context (https://github.com/ranxianglei/billion-context),
// `processTurn` and the pipeline of `kernel/src/compress.ts` and
// `kernel/src/pipeline.ts` of acp-kernel 0.0.189 at commit `4be21b08e`.
// Copyright (c) 2026 ranxianglei. MIT License with an additional attribution
// term (NOTICE).
// Modified for hanafubuki-agent: the steps run as one function in the
// kernel's order. Ids are the host's stable ones, so there is no live-id
// reconciliation; the opt-in steps (timestamps, the content store, absorb,
// crush, filters, carrier relocation) are not ported yet; there is no
// separate apply view, since the host passes the full history.

use std::collections::HashSet;

use crate::latch::{apply_view_latch, view_latch_config_digest};
use crate::model::{Config, Message, State};
use crate::nudge::{decide_nudge, stamp_nudge, NudgeDecision, NudgeInput, TokenCountSource};
use crate::number::format_k;
use crate::prune::{hide_consumed_compress_calls, prune, sync_blocks};
use crate::recommend::recommend;
use crate::refs::{assign_message_refs, highest_used_index, index_to_ref};
use crate::render::{render_refs, RenderStrategy};
use crate::truncate::{emergency_truncate, TerminalEscape};

/// One request's input.
pub struct TurnInput<'a> {
    /// The full, unpruned history of the window, with the host's stable ids.
    pub messages: &'a [Message],
    pub state: State,
    pub config: &'a Config,
    /// The context's size in tokens: the provider's last usage when there
    /// is one.
    pub token_count: u64,
    pub token_count_source: Option<TokenCountSource>,
    pub render: RenderStrategy,
}

/// The view to send and what goes with it.
#[derive(Clone, Debug, PartialEq)]
pub struct TurnOutput {
    /// The view: covered messages out, summaries in, live `compress` calls
    /// compacted, ref tags in, clips latched.
    pub messages: Vec<Message>,
    /// The state to persist.
    pub state: State,
    pub nudge: NudgeDecision,
    /// When a compression round is live but its nudge is not shown this
    /// request: the highest ref and the offered ranges, so the model never
    /// guesses refs. The host appends it, like a nudge, as an ephemeral
    /// trailing user message.
    pub status_line: Option<String>,
    /// Compression and truncation are exhausted over the threshold; the
    /// host surfaces it once.
    pub terminal_escape: Option<TerminalEscape>,
    /// The emergency truncation ran over its threshold and reclaimed
    /// nothing.
    pub truncation_skipped: Option<String>,
}

/// Projects the full history into the view for one request: mints refs,
/// syncs and prunes the blocks, hides consumed `compress` calls, replays
/// the view latch, recommends ranges, decides the nudge, truncates as a
/// last resort and renders the ref tags. Bytes already sent change only at
/// a fold.
pub fn process_turn(input: TurnInput<'_>) -> TurnOutput {
    let TurnInput {
        messages: original,
        mut state,
        config,
        token_count,
        token_count_source,
        render,
    } = input;

    assign_message_refs(&mut state, original, config);
    sync_blocks(original, &mut state);
    state.advance_survival(config.promotion_threshold);
    let view = prune(original, &state);

    // The opt-in transforms (P10: the content store, absorb-hide, crush,
    // absorb-prompt, filters) run between this snapshot and the latch.
    let pre_transform = view.clone();
    let hidden = hide_consumed_compress_calls(&state, &view);
    hidden.commit(&mut state);
    let view = apply_view_latch(
        &pre_transform,
        hidden.messages,
        &mut state,
        &view_latch_config_digest(config),
    );

    let recommendation = recommend(&view, original, &state, config);
    let nudge = decide_nudge(&NudgeInput {
        view: &view,
        state: &state,
        config,
        recommendation: &recommendation,
        token_count,
        token_count_source,
    });
    stamp_nudge(&mut state.nudge, &nudge, token_count);
    let emergency = emergency_truncate(view, &mut state, config, token_count, &nudge);
    let messages = render_refs(&emergency.messages, &mut state, render);

    lift_dead_refs(&mut state, original);
    let status_line = status_line(&state, &nudge);
    TurnOutput {
        messages,
        state,
        nudge,
        status_line,
        terminal_escape: emergency.terminal_escape,
        truncation_skipped: emergency.truncation_skipped,
    }
}

/// A dead ref whose message came back into the history is alive again.
fn lift_dead_refs(state: &mut State, messages: &[Message]) {
    if state.dead_refs.is_empty() {
        return;
    }
    let present: HashSet<&str> = messages.iter().map(|message| message.id.as_str()).collect();
    let by_ref = &state.message_refs.by_ref;
    state.dead_refs.retain(|r#ref| {
        !by_ref
            .get(r#ref)
            .is_some_and(|id| present.contains(id.as_str()))
    });
}

/// Between nudges a round can stay armed for many requests, and a model
/// that sees no current refs guesses beyond the highest one. The line is
/// shown when ranges are offered but the nudge is not: for an armed tier
/// held back by its cadence, or once any nudge was shown. A fresh session
/// below every threshold stays silent.
fn status_line(state: &State, nudge: &NudgeDecision) -> Option<String> {
    let banner_shown =
        state.nudge.last_nudge_shown_tokens > 0 || !state.nudge.last_shown_by_tier.is_empty();
    let ranges = &nudge.compressible_ranges;
    if nudge.should_inject || !(nudge.tier.is_some() || banner_shown) || ranges.is_empty() {
        return None;
    }
    let highest = highest_used_index(&state.message_refs);
    let highest = index_to_ref(highest)
        .filter(|_| highest > 0)
        .unwrap_or_else(|| "none".to_owned());
    let offered: Vec<String> = ranges
        .iter()
        .take(3)
        .map(|range| {
            format!(
                "{}–{} ({})",
                range.start_ref,
                range.end_ref,
                format_k(range.tokens as f64)
            )
        })
        .collect();
    let extra = if ranges.len() > 3 {
        format!(", +{} more", ranges.len() - 3)
    } else {
        String::new()
    };
    Some(format!(
        "[ACP status: highest allocated ref {highest} — refs above it are not minted yet, never guess them; compressible now: {}{extra}; the newest messages stay protected. Use these refs directly; acp_status only if you need the full list.]",
        offered.join(", ")
    ))
}
