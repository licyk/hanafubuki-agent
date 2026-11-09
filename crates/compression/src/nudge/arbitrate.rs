// Derived from billion-context (https://github.com/ranxianglei/billion-context),
// `decideNudge` of `kernel/src/compress.ts` of acp-kernel 0.0.189 at commit
// `4be21b08e`. Copyright (c) 2026 ranxianglei. MIT License with an
// additional attribution term (NOTICE).

use super::{NudgeInput, Tiers};
use crate::model::{Config, State};
use crate::number::{format_number, js_round};

/// Everything a decision compares, computed once.
pub(super) struct Gauges {
    usage_pct: i64,
    pub(super) over_limit: bool,
    pub(super) emergency: bool,
    pressure: bool,
    pub(super) pressure_deferred: bool,
    estimate_hold_streak: u32,
    max_estimate_hold_rounds: u32,
    pub(super) min_pressure_benefit: u64,
    pub(super) growth_floor: f64,
    pub(super) growth: i64,
    thresholds: [u64; 3],
    pub(super) max_pending: u64,
    t1_harvest_stall: bool,
    first_sight: bool,
    absorbed_debt: u64,
    absorb_count: u32,
    repeat_absorbs: bool,
    growth_ready: bool,
}

impl Gauges {
    pub(super) fn new(
        input: &NudgeInput<'_>,
        usage: f64,
        nudge_growth_tokens: u64,
        growth_reference: u64,
        estimate_hold_streak: u32,
        tiers: &Tiers,
    ) -> Self {
        let NudgeInput {
            config,
            state,
            token_count,
            ..
        } = *input;
        let nudge = &config.nudge;
        let limit = config.model_context_limit;
        let over_limit = usage >= nudge.max_context_limit_pct;
        let emergency = usage >= nudge.emergency_threshold_pct;
        let pressure = over_limit || emergency;
        // An estimated count overstates what is billed, so the pressure band
        // waits for a measured one, but only for so many estimated requests:
        // a provider that never reports usage must still be nudged.
        let pressure_deferred = pressure
            && estimate_hold_streak > 0
            && nudge.require_usage_sample_for_pressure
            && estimate_hold_streak <= nudge.max_estimate_hold_rounds;
        let tier2_threshold =
            js_round(nudge_growth_tokens as f64 * nudge.tier2_growth_multiplier) as u64;
        let thresholds = [
            nudge.tier_growth_tokens.t1.unwrap_or(nudge_growth_tokens),
            nudge.tier_growth_tokens.t2.unwrap_or(tier2_threshold),
            nudge.tier_growth_tokens.t3.unwrap_or(tier2_threshold),
        ];
        let max_pending = tiers.pending.iter().copied().max().unwrap_or(0);
        let growth_floor = (nudge.min_growth_floor as f64)
            .max(nudge.min_growth_ratio * nudge_growth_tokens as f64);
        let growth = token_count as i64 - growth_reference as i64;
        let shown = state.nudge.last_nudge_shown_tokens;
        let baseline = state.nudge.last_per_message_nudge_tokens;
        // A session arriving with a ready backlog has no growth to show; the
        // floor paces work within a backlog, it must not keep it from
        // draining. An ignored nudge keeps `shown` set, so this never fires
        // again on a session that does not answer.
        let first_sight = shown == 0
            && baseline == 0
            && usage >= nudge.min_context_limit_pct
            && max_pending >= nudge_growth_tokens;
        // Growth skipped by repeated aborts counts as growth once a second
        // abort shows the session is abort-heavy; one abort stays quiet.
        let absorbed_debt = state.nudge.absorbed_debt;
        let absorb_count = state.nudge.absorb_count;
        let repeat_absorbs = absorb_count >= 2;
        let payable_debt = if repeat_absorbs { absorbed_debt } else { 0 };
        let backlog_drain = repeat_absorbs
            && absorbed_debt > 0
            && shown == 0
            && max_pending >= 2 * thresholds[0]
            && usage >= nudge.min_context_limit_pct;
        let growth_ready =
            first_sight || backlog_drain || (growth + payable_debt as i64) as f64 >= growth_floor;
        Self {
            usage_pct: js_round(usage * 100.0) as i64,
            over_limit,
            emergency,
            pressure,
            pressure_deferred,
            estimate_hold_streak,
            max_estimate_hold_rounds: nudge.max_estimate_hold_rounds,
            min_pressure_benefit: nudge
                .min_pressure_benefit_tokens
                .unwrap_or_else(|| 5000.max(js_round(limit as f64 * 0.01) as u64)),
            growth_floor,
            growth,
            thresholds,
            max_pending,
            t1_harvest_stall: config.tiers.enabled && state.nudge.t1_harvest_stall,
            first_sight,
            absorbed_debt,
            absorb_count,
            repeat_absorbs,
            growth_ready,
        }
    }

    fn threshold(&self, tier: u8) -> u64 {
        self.thresholds[usize::from(tier - 1)]
    }

    fn label(&self) -> &'static str {
        if self.emergency {
            "EMERGENCY"
        } else {
            "OVER-LIMIT"
        }
    }

    fn floor(&self) -> String {
        format_number(self.growth_floor)
    }
}

pub(super) struct Verdict {
    pub(super) tier: u8,
    pub(super) reason: String,
}

/// The tier to nudge, if any.
pub(super) fn arbitrate(
    gauges: &Gauges,
    tiers: &Tiers,
    state: &State,
    config: &Config,
    token_count: u64,
) -> Option<Verdict> {
    let (tier, mut reason, stall_escalated) = if gauges.pressure {
        pressure_tier(gauges, tiers, config)?
    } else if gauges.growth_ready {
        growth_tier(gauges, tiers, state, config, token_count)?
    } else {
        return None;
    };
    if gauges.first_sight {
        reason.push_str(" [first-sight mass]");
    }
    if stall_escalated {
        reason.push_str(" [T1 harvest stall]");
    }
    Some(Verdict { tier, reason })
}

/// The pressure band's candidate with the most pending tokens. A tier-1
/// channel shown not to drain leaves the running when a distillation tier
/// can pay the minimum benefit.
fn pressure_best(gauges: &Gauges, tiers: &Tiers, config: &Config) -> (Option<u8>, u64) {
    let mut candidates = Vec::new();
    if !(gauges.t1_harvest_stall
        && tiers.pending(2).max(tiers.pending(3)) >= gauges.min_pressure_benefit)
    {
        candidates.push(1);
    }
    if config.tiers.enabled {
        candidates.extend([2, 3]);
    }
    let mut best = None;
    let mut best_pending = 0;
    for tier in candidates {
        if tiers.pending(tier) > best_pending {
            best_pending = tiers.pending(tier);
            best = Some(tier);
        }
    }
    (best, best_pending)
}

fn pressure_tier(gauges: &Gauges, tiers: &Tiers, config: &Config) -> Option<(u8, String, bool)> {
    let (best, pending) = pressure_best(gauges, tiers, config);
    // Below the minimum benefit a fold reclaims almost nothing, and its
    // success would only re-arm the same nudge.
    let best =
        best.filter(|_| pending >= gauges.min_pressure_benefit && !gauges.pressure_deferred)?;
    let (label, usage) = (gauges.label(), gauges.usage_pct);
    let reason = if best == 1 {
        format!("{label} T1: max effective pending {pending}, usage {usage}%")
    } else {
        format!(
            "{label} T{best} distill: max pending {pending} (T1 effective {}, T2 {}, T3 {}), usage {usage}%",
            tiers.pending(1),
            tiers.pending(2),
            tiers.pending(3),
        )
    };
    Some((best, reason, gauges.t1_harvest_stall && best != 1))
}

fn growth_tier(
    gauges: &Gauges,
    tiers: &Tiers,
    state: &State,
    config: &Config,
    token_count: u64,
) -> Option<(u8, String, bool)> {
    let cadence_met = |tier: u8| {
        let last = state
            .nudge
            .last_shown_by_tier
            .get(&tier)
            .copied()
            .unwrap_or(0);
        last == 0 || token_count as f64 - last as f64 >= gauges.growth_floor
    };
    let (t1, t2, t3) = (tiers.pending(1), tiers.pending(2), tiers.pending(3));
    let (t2_count, t3_count) = (tiers.targets(2).len(), tiers.targets(3).len());
    let usage = gauges.usage_pct;
    let derived = |tier: u8| {
        let set = match tier {
            2 => config.nudge.tier_growth_tokens.t2,
            _ => config.nudge.tier_growth_tokens.t3,
        };
        if set.is_none() {
            " (1.5x)"
        } else {
            ""
        }
    };
    let (threshold2, threshold3) = (gauges.threshold(2), gauges.threshold(3));
    let t3_mass = || {
        format!(
            "T3 condense ready: {t3_count} tier-2 blocks ({t3} tokens) >= {threshold3}{}",
            derived(3)
        )
    };
    let t2_mass = || {
        format!(
            "T2 distill ready: {t2_count} tier-1 blocks ({t2} tokens) >= {threshold2}{}",
            derived(2)
        )
    };

    // A tier-1 channel shown not to drain lets a mass-qualified
    // distillation tier go first, without having to outweigh tier 1.
    if gauges.t1_harvest_stall {
        if t3 >= threshold3 && t3 > t2 && cadence_met(3) {
            return Some((
                3,
                format!("{} and > T2 {t2}, usage {usage}%", t3_mass()),
                true,
            ));
        }
        if t2 >= threshold2 && cadence_met(2) {
            return Some((2, format!("{}, usage {usage}%", t2_mass()), true));
        }
    }
    if t1 >= gauges.threshold(1) {
        let reason = format!(
            "T1 effective {t1} >= {}, growth {}, usage {usage}%",
            gauges.threshold(1),
            gauges.growth
        );
        return Some((1, reason, false));
    }
    let tiers_on = config.tiers.enabled;
    let t2_count_ready = t2_count >= config.tiers.tier2_trigger;
    let t3_count_ready = t3_count >= config.tiers.tier3_trigger;
    if tiers_on && (t2_count_ready || (t2 >= threshold2 && t2 > t1)) {
        let reason = if t2_count_ready {
            format!(
                "T2 distill ready: {t2_count} tier-1 blocks >= tier2Trigger {} ({t2} tokens), usage {usage}%",
                config.tiers.tier2_trigger
            )
        } else {
            format!("{} and > T1 effective {t1}, usage {usage}%", t2_mass())
        };
        return cadence_met(2).then_some((2, reason, false));
    }
    if tiers_on && (t3_count_ready || (t3 >= threshold3 && t3 > t2 && t3 > t1)) {
        let reason = if t3_count_ready {
            format!(
                "T3 condense ready: {t3_count} tier-2 blocks >= tier3Trigger {} ({t3} tokens), usage {usage}%",
                config.tiers.tier3_trigger
            )
        } else {
            format!(
                "{} and > T2 {t2} and > T1 effective {t1}, usage {usage}%",
                t3_mass()
            )
        };
        return cadence_met(3).then_some((3, reason, false));
    }
    None
}

/// Why nothing was nudged: the actual blocking condition, not a template.
pub(super) fn idle_reason(
    gauges: &Gauges,
    tiers: &Tiers,
    state: &State,
    config: &Config,
    token_count: u64,
) -> String {
    let (t1, t2, t3) = (tiers.pending(1), tiers.pending(2), tiers.pending(3));
    let (label, usage) = (gauges.label(), gauges.usage_pct);
    if gauges.pressure_deferred {
        return format!(
            "{label}: usage {usage}% but the meter reading is estimate-grade (no billing sample landed this cycle, max pending {}) — pressure injection held (estimate round {} of {}) until a usage-grade reading decides or the estimate-hold backstop elapses; hard-overflow backstop unaffected",
            gauges.max_pending, gauges.estimate_hold_streak, gauges.max_estimate_hold_rounds,
        );
    }
    if gauges.pressure {
        let (_, best_pending) = pressure_best(gauges, tiers, config);
        return if best_pending == 0 {
            format!(
                "{label}: usage {usage}% but no tier has effective compressible content (T1 effective {t1}, T2 {t2}, T3 {t3}) — nudge suppressed to avoid offering ranges below minCompressRange"
            )
        } else {
            format!(
                "{label}: usage {usage}% but max pending {best_pending} < min benefit {} tokens (T1 effective {t1}, T2 {t2}, T3 {t3}) — suppressed: rewriting below the benefit floor reclaims almost nothing while usage stays high; truncate.threshold remains the safety valve",
                gauges.min_pressure_benefit
            )
        };
    }

    let eligible: Vec<u8> = [1, 2, 3]
        .into_iter()
        .filter(|tier| config.tiers.enabled || *tier == 1)
        .collect();
    let count = |tier: u8| tiers.targets(tier).len();
    let count_ready = |tier: u8| match tier {
        2 => count(2) >= config.tiers.tier2_trigger,
        3 => count(3) >= config.tiers.tier3_trigger,
        _ => false,
    };
    let over = |tier: u8| tiers.pending(tier) >= gauges.threshold(tier);
    let mut ready: Vec<String> = eligible
        .iter()
        .filter(|tier| over(**tier))
        .map(|tier| format!("T{tier} {}", tiers.pending(*tier)))
        .collect();
    ready.extend(
        eligible
            .iter()
            .filter(|tier| !over(**tier) && count_ready(**tier))
            .map(|tier| format!("T{tier} {} blocks (count)", count(*tier))),
    );
    let blocked: Vec<String> = eligible
        .iter()
        .filter(|tier| {
            let last = state
                .nudge
                .last_shown_by_tier
                .get(tier)
                .copied()
                .unwrap_or(0);
            (over(**tier) || count_ready(**tier))
                && last > 0
                && (token_count as f64 - last as f64) < gauges.growth_floor
        })
        .map(|tier| format!("T{tier} (cadence)"))
        .collect();

    let min_threshold = gauges.thresholds.iter().copied().min().unwrap_or(0);
    let mut parts = Vec::new();
    if gauges.max_pending < min_threshold {
        parts.push(format!(
            "max compressible {} < threshold {min_threshold}",
            gauges.max_pending
        ));
    }
    if (gauges.growth as f64) < gauges.growth_floor {
        let (growth, debt, floor) = (gauges.growth, gauges.absorbed_debt, gauges.floor());
        parts.push(if debt > 0 && !gauges.repeat_absorbs {
            format!("growth {growth} + absorbed {debt} held (single abort, #5293) < floor {floor}")
        } else if debt > 0 {
            format!(
                "growth {growth} + absorbed {debt} across {} aborts < floor {floor}",
                gauges.absorb_count
            )
        } else {
            format!("growth {growth} < floor {floor}")
        });
    }
    if parts.is_empty() {
        parts.push(format!(
            "max compressible {}, growth {}",
            gauges.max_pending, gauges.growth
        ));
    }
    let mut reason = parts.join("; ");
    if !ready.is_empty() {
        reason.push_str(&format!(", ready: {}", ready.join(", ")));
    }
    if !blocked.is_empty() {
        reason.push_str(&format!(", blocked: {}", blocked.join(", ")));
    }
    reason
}
