// Derived from billion-context (https://github.com/ranxianglei/billion-context),
// `decideNudge` and the nudge-inject step of `kernel/src/compress.ts` of
// acp-kernel 0.0.189 at commit `4be21b08e`. Copyright (c) 2026 ranxianglei.
// MIT License with an additional attribution term (NOTICE).

mod arbitrate;
mod text;

pub use text::{format_ranges, render_nudge_text, NudgeVoice, RenderedNudge, ONE_CALL_HINT};

use crate::boundaries::{active_block_spans, BlockSpan};
use crate::model::{Block, Config, Message, NudgeConfig, NudgeState, Role, State};
use crate::number::js_round;
use crate::prune::SUMMARY_HEADER;
use crate::recommend::{CompressibleRange, ProtectedRange, Recommendation};
use crate::render::{count_message_tokens, default_count_tokens};
use arbitrate::{arbitrate, idle_reason, Gauges};

/// Where the request's token count came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TokenCountSource {
    /// The provider's reported usage, or an anchor derived from it.
    Usage,
    /// A local estimate, which runs high against what is billed.
    Estimate,
}

/// Whether to ask the model to compress this request, and what to offer.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct NudgeDecision {
    pub should_inject: bool,
    /// Why, or what held it back, for diagnostics and the status panel.
    pub reason: String,
    pub compressible_ranges: Vec<CompressibleRange>,
    pub protected_ranges: Vec<ProtectedRange>,
    /// Where the active blocks stand, so the model can check a block
    /// boundary without calling `acp_status`.
    pub active_block_spans: Vec<BlockSpan>,
    /// For a tier-2 or tier-3 nudge, the blocks to distil.
    pub tier_target_blocks: Vec<Block>,
    pub context_usage: f64,
    /// The tier nudged; `None` when nothing is.
    pub tier: Option<u8>,
    pub breakdown: NudgeBreakdown,
    pub context_breakdown: Option<ContextBreakdown>,
}

/// The numbers a decision was made from.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct NudgeBreakdown {
    pub usage: f64,
    /// Tokens since the growth reference; negative after a drop.
    pub growth: i64,
    pub growth_reference: u64,
    pub effective_threshold: u64,
    pub nudge_growth_tokens: u64,
    pub growth_floor: f64,
    pub has_pending_nudge: bool,
    pub over_limit: bool,
    pub emergency_override: bool,
    /// The pressure band waited for a measured token count.
    pub pressure_deferred_no_sample: bool,
    /// Consecutive requests whose count was an estimate, this one included.
    pub estimate_hold_streak: u32,
    pub min_pressure_benefit: u64,
    pub pending_t1: u64,
    pub pending_t2: u64,
    pub pending_t3: u64,
    /// Compared with the minimum benefit to detect the terminal floor.
    pub max_pending: u64,
}

/// The view's tokens by kind.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ContextBreakdown {
    pub system: u64,
    pub tool: u64,
    pub summaries: u64,
    pub code: u64,
    pub text: u64,
    pub total: u64,
    pub growth: i64,
}

/// What a nudge is decided from.
pub struct NudgeInput<'a> {
    /// The view as the model will see it.
    pub view: &'a [Message],
    pub state: &'a State,
    pub config: &'a Config,
    pub recommendation: &'a Recommendation,
    pub token_count: u64,
    /// `None` keeps the pressure band from waiting for a measured count.
    pub token_count_source: Option<TokenCountSource>,
}

/// Each tier's pending tokens and the blocks it would distil: tier 1 the
/// recommended ranges that clear the minimum range, tier 2 the summaries of
/// the active tier-1 blocks, tier 3 those of the active tier-2 blocks.
pub(crate) struct Tiers {
    pub(crate) pending: [u64; 3],
    pub(crate) targets: [Vec<Block>; 3],
}

impl Tiers {
    fn new(state: &State, recommendation: &Recommendation, min_compress_range: usize) -> Self {
        let t1 = recommendation
            .recommended_ranges
            .iter()
            .filter(|range| {
                min_compress_range == 0 || range.effective_chars() >= min_compress_range
            })
            .map(|range| range.tokens)
            .sum();
        let of_tier = |tier: u8| -> Vec<Block> {
            state
                .active_blocks()
                .filter(|block| block.tier == tier)
                .cloned()
                .collect()
        };
        let summary_tokens = |blocks: &[Block]| -> u64 {
            blocks
                .iter()
                .map(|block| default_count_tokens(&block.summary))
                .sum()
        };
        let (tier1, tier2) = (of_tier(1), of_tier(2));
        Self {
            pending: [t1, summary_tokens(&tier1), summary_tokens(&tier2)],
            targets: [Vec::new(), tier1, tier2],
        }
    }

    pub(crate) fn pending(&self, tier: u8) -> u64 {
        self.pending[usize::from(tier - 1)]
    }

    pub(crate) fn targets(&self, tier: u8) -> &[Block] {
        &self.targets[usize::from(tier - 1)]
    }
}

/// Decides the nudge. In the pressure band (from `max_context_limit_pct`)
/// the tier with the most pending tokens is nudged if it reclaims at least
/// the minimum benefit, unless the count is an estimate and a measured one
/// is awaited. Otherwise a nudge needs the context to have grown by the
/// growth floor since the last one (or a ready backlog on first sight), and
/// then tier 1 goes first when its pending clears its threshold; tiers 2
/// and 3 go when they clear theirs and outweigh the lower tiers, each at
/// its own cadence. A tier-1 channel that answered its last nudge with
/// less than half of what it offered lets the distillation tiers go first.
pub fn decide_nudge(input: &NudgeInput<'_>) -> NudgeDecision {
    let NudgeInput {
        config,
        state,
        token_count,
        ..
    } = *input;
    let nudge = &config.nudge;
    let limit = config.model_context_limit;
    let usage = if limit > 0 {
        token_count as f64 / limit as f64
    } else {
        0.0
    };
    let nudge_growth_tokens = adaptive_growth(limit, nudge);
    let estimate = input.token_count_source == Some(TokenCountSource::Estimate);
    let estimate_hold_streak = if estimate {
        state.nudge.estimate_hold_streak + 1
    } else {
        0
    };
    let baseline = state.nudge.last_per_message_nudge_tokens;
    let shown = state.nudge.last_nudge_shown_tokens;
    let growth_reference = if shown > 0 {
        shown
    } else if baseline > 0 {
        baseline
    } else {
        token_count
    };
    let tiers = Tiers::new(
        state,
        input.recommendation,
        config.compress.min_compress_range,
    );
    let gauges = Gauges::new(
        input,
        usage,
        nudge_growth_tokens,
        growth_reference,
        estimate_hold_streak,
        &tiers,
    );

    let (tier, reason) = match arbitrate(&gauges, &tiers, state, config, token_count) {
        Some(verdict) => (Some(verdict.tier), verdict.reason),
        None => (
            None,
            idle_reason(&gauges, &tiers, state, config, token_count),
        ),
    };
    let growth = gauges.growth;
    NudgeDecision {
        should_inject: tier.is_some(),
        reason,
        compressible_ranges: input.recommendation.recommended_ranges.clone(),
        protected_ranges: input.recommendation.context_ranges.protected.clone(),
        active_block_spans: active_block_spans(state),
        tier_target_blocks: tier.map_or_else(Vec::new, |tier| tiers.targets(tier).to_vec()),
        context_usage: usage,
        tier,
        breakdown: NudgeBreakdown {
            usage,
            growth,
            growth_reference,
            effective_threshold: if shown > 0 {
                nudge_growth_tokens / 2
            } else {
                nudge_growth_tokens
            },
            nudge_growth_tokens,
            growth_floor: gauges.growth_floor,
            has_pending_nudge: shown > 0,
            over_limit: gauges.over_limit,
            emergency_override: gauges.emergency,
            pressure_deferred_no_sample: gauges.pressure_deferred,
            estimate_hold_streak,
            min_pressure_benefit: gauges.min_pressure_benefit,
            pending_t1: tiers.pending(1),
            pending_t2: tiers.pending(2),
            pending_t3: tiers.pending(3),
            max_pending: gauges.max_pending,
        },
        context_breakdown: Some(context_breakdown(input.view, token_count, growth)),
    }
}

/// The growth step: the window times `growth_ratio`, clamped to
/// `growth_floor..=growth_cap`.
fn adaptive_growth(limit: u64, nudge: &NudgeConfig) -> u64 {
    if limit == 0 {
        return nudge.growth_floor;
    }
    let scaled = js_round(limit as f64 * nudge.growth_ratio) as u64;
    nudge.growth_cap.min(nudge.growth_floor.max(scaled))
}

fn context_breakdown(view: &[Message], total: u64, growth: i64) -> ContextBreakdown {
    let mut breakdown = ContextBreakdown {
        total,
        growth,
        ..ContextBreakdown::default()
    };
    for message in view {
        let tokens = count_message_tokens(message);
        let bucket = if message.text.starts_with(SUMMARY_HEADER) {
            &mut breakdown.summaries
        } else if message.is_tool() {
            &mut breakdown.tool
        } else if message.role == Role::System {
            &mut breakdown.system
        } else if message.text.contains("```") {
            &mut breakdown.code
        } else {
            &mut breakdown.text
        };
        *bucket += tokens;
    }
    breakdown
}

/// The nudge step's bookkeeping after a decision. Every reference is a
/// low-water mark: it may follow the count down, never up, so growth is
/// always measured from the latest valley and a compression settling over
/// several requests corrects itself. A nudge shown stamps its count and its
/// tier's cadence, and a tier-1 nudge banks what it offered, which the next
/// fold is measured against.
pub(crate) fn stamp_nudge(nudge: &mut NudgeState, decision: &NudgeDecision, token_count: u64) {
    nudge.estimate_hold_streak = decision.breakdown.estimate_hold_streak;
    if nudge.last_per_message_nudge_tokens == 0 || token_count < nudge.last_per_message_nudge_tokens
    {
        nudge.last_per_message_nudge_tokens = token_count;
    }
    if nudge.last_nudge_shown_tokens > 0 && token_count < nudge.last_nudge_shown_tokens {
        nudge.last_nudge_shown_tokens = token_count;
    }
    for stamp in nudge.last_shown_by_tier.values_mut() {
        *stamp = (*stamp).min(token_count);
    }
    if decision.should_inject {
        nudge.last_nudge_shown_tokens = token_count;
        if let Some(tier) = decision.tier {
            nudge.last_shown_by_tier.insert(tier, token_count);
            if tier == 1 {
                nudge.last_t1_fire_pending = Some(decision.breakdown.pending_t1);
            }
        }
    }
}
