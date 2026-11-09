// Derived from billion-context (https://github.com/ranxianglei/billion-context),
// `processTurn` and the pipeline of `kernel/src/compress.ts` and
// `kernel/src/pipeline.ts` of acp-kernel 0.0.189 at commit `4be21b08e`.
// Copyright (c) 2026 ranxianglei. MIT License with an additional attribution
// term (NOTICE).
// Modified for hanafubuki-agent: the steps run as one function in the
// kernel's order. Ids are the host's stable ones, so there is no live-id
// reconciliation; timestamps and carrier relocation are not ported; there
// is no separate apply view, since the host passes the full history; the
// content store and the filters come with the call.

use std::collections::HashSet;

use crate::absorb::{append_absorb_prompts, hide_absorbed_messages};
use crate::crush::apply_crush_to_messages;
use crate::filter::{apply_message_filters, MessageFilter};
use crate::latch::{apply_view_latch, view_latch_config_digest};
use crate::model::{Config, Message, State};
use crate::nudge::{decide_nudge, stamp_nudge, NudgeDecision, NudgeInput, TokenCountSource};
use crate::number::format_k;
use crate::prune::{hide_consumed_compress_calls, prune, sync_blocks};
use crate::recommend::recommend;
use crate::refs::{assign_message_refs, highest_used_index, index_to_ref};
use crate::render::{render_refs, RenderStrategy};
use crate::store::{store_large_results, ContentStore};
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

/// What the opt-in transforms need beyond a [`TurnInput`].
#[derive(Default)]
pub struct TurnExtras<'a> {
    /// The session's content store, as the previous turn returned it.
    pub content_store: ContentStore,
    /// The host's message filters, run while `config.message_filters` is
    /// on.
    pub filters: &'a [&'a dyn MessageFilter],
}

/// [`process_turn_with`] with an empty content store and no filters: for a
/// host whose content store is off.
pub fn process_turn(input: TurnInput<'_>) -> TurnOutput {
    process_turn_with(input, TurnExtras::default()).0
}

/// Projects the full history into the view for one request: mints refs,
/// syncs and prunes the blocks, runs the opt-in transforms that are on
/// (the content store, absorbed-pair hiding, crush, the absorb demand, the
/// filters), hides consumed `compress` calls, replays the view latch,
/// recommends ranges, decides the nudge, truncates as a last resort and
/// renders the ref tags. Bytes already sent change only at a fold, an
/// absorption or a change of the transforms' settings. Returns the content
/// store to persist with the state.
pub fn process_turn_with(
    input: TurnInput<'_>,
    extras: TurnExtras<'_>,
) -> (TurnOutput, ContentStore) {
    let TurnExtras {
        mut content_store,
        filters,
    } = extras;
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

    let pre_transform = view.clone();
    let view = transform(
        view,
        &mut state,
        config,
        token_count,
        &mut content_store,
        filters,
    );
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
    let output = TurnOutput {
        messages,
        state,
        nudge,
        status_line,
        terminal_escape: emergency.terminal_escape,
        truncation_skipped: emergency.truncation_skipped,
    };
    (output, content_store)
}

/// The opt-in transforms, in the kernel's order: the content store first
/// (a placeholder is never absorbed), crush before the absorb demand (which
/// decides on the crushed size).
fn transform(
    view: Vec<Message>,
    state: &mut State,
    config: &Config,
    token_count: u64,
    content_store: &mut ContentStore,
    filters: &[&dyn MessageFilter],
) -> Vec<Message> {
    let stored = store_large_results(view, state, content_store, config);
    state.stats.stored_count += stored.stored_count as u64;
    let mut view = hide_absorbed_messages(stored.messages, state);
    if config.crush.enabled && config.absorb.enabled {
        view = apply_crush_to_messages(view, config, token_count).messages;
    }
    view = append_absorb_prompts(view, state, config, token_count).messages;
    if config.message_filters.enabled && !filters.is_empty() {
        view = apply_message_filters(view, &config.message_filters, filters).messages;
    }
    view
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
