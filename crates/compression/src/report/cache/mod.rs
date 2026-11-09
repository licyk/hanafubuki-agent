// Derived from billion-context (https://github.com/ranxianglei/billion-context),
// `kernel/src/cache-report.ts` of acp-kernel 0.0.189 at commit `4be21b08e`.
// Copyright (c) 2026 ranxianglei. MIT License with an additional attribution
// term (NOTICE).
// Modified for hanafubuki-agent: the caller passes the time the report is
// generated at, and the local time of day as an offset from UTC.

mod format;

pub use format::{format_cache_report, CacheDetail, FormatCacheReportOptions};

use crate::number::js_round;

/// One request's provider-reported usage.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CacheSample {
    /// Milliseconds since the Unix epoch.
    pub at: u64,
    /// Prompt tokens billed, cached ones included (Anthropic reports them
    /// apart; add them back).
    pub input: u64,
    pub cached: u64,
    pub output: Option<u64>,
}

/// A fold applied between two requests.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FoldEvent {
    pub at: u64,
    /// Tokens the fold removed from the view.
    pub tokens_compressed: u64,
    pub summary_tokens: Option<u64>,
    /// Where the post-fold view first differs, in tokens: about the prefix
    /// still cached after it. Only ever a bound, never part of the totals.
    pub first_fold_start_tokens: Option<u64>,
    pub view_before: Option<u64>,
    pub view_after: Option<u64>,
}

/// Prices per input token: cache writes `w`, cache reads `r`, output `q`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PriceProfile {
    pub w: f64,
    pub r: f64,
    pub q: f64,
}

impl Default for PriceProfile {
    fn default() -> Self {
        Self {
            w: 1.0,
            r: 0.1,
            q: 4.0,
        }
    }
}

/// A fold claims the cold requests after it until a warm one shows the
/// provider cached again, or this many were claimed.
pub const FOLD_CLAIM_WARM_HIT_PCT: f64 = 85.0;
pub const FOLD_CLAIM_COLD_ROUNDS: usize = 4;

/// One request's miss in three buckets that add up to it exactly.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SampleDecomposition {
    pub missed: u64,
    /// Appended since the previous request: fresh, not an invalidation.
    pub new_content: u64,
    /// Paid again because a fold rewrote the prefix.
    pub comp_repay: u64,
    /// The rest: provider expiry or a client rewrite, which cannot be told
    /// apart.
    pub ttl_repay: u64,
    /// The pending fold credited with `comp_repay`.
    pub fold_index: Option<usize>,
}

/// Splits a request's miss: new content up to the growth, then what the
/// earliest pending fold's divergence forced to be paid again, then the
/// rest.
pub fn decompose_sample(
    previous: Option<&CacheSample>,
    current: &CacheSample,
    pending_folds: &[FoldEvent],
) -> SampleDecomposition {
    let missed = current.input.saturating_sub(current.cached);
    let growth = previous.map_or(0, |previous| current.input.saturating_sub(previous.input));
    let new_content = missed.min(growth);
    let remainder = missed - new_content;
    let mut comp_repay = 0;
    let mut fold_index = None;
    if remainder > 0 && !pending_folds.is_empty() {
        let (owner, divergence) = pending_folds
            .iter()
            .enumerate()
            .map(|(index, fold)| {
                let divergence = fold
                    .first_fold_start_tokens
                    .or(fold.view_after)
                    .unwrap_or(0);
                (index, divergence)
            })
            .fold((0, u64::MAX), |best, candidate| {
                if candidate.1 < best.1 {
                    candidate
                } else {
                    best
                }
            });
        let structural_excess = current
            .input
            .saturating_sub(divergence)
            .saturating_sub(new_content);
        comp_repay = remainder.min(structural_excess);
        if comp_repay > 0 {
            fold_index = Some(owner);
        }
    }
    SampleDecomposition {
        missed,
        new_content,
        comp_repay,
        ttl_repay: remainder - comp_repay,
        fold_index,
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CacheTotals {
    pub requests: usize,
    pub input: u64,
    pub cached: u64,
    pub output: u64,
    pub hit_pct: f64,
    pub new_content: u64,
    pub comp_repay: u64,
    pub ttl_repay: u64,
    /// Input less cached less the three buckets: zero by construction.
    pub residual: i64,
    pub balanced: bool,
}

/// A fold's measured counters.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct FoldEconomicsInput {
    pub seq: usize,
    pub at: u64,
    /// Tokens removed.
    pub s: u64,
    /// Summary tokens.
    pub sigma: u64,
    pub v_prime: Option<u64>,
    /// The hit rate of the first request after it.
    pub h_pct: Option<f64>,
    /// The re-pay credited to it.
    pub t: u64,
    pub requests_after: usize,
    pub turns_to_next_fold: Option<usize>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct FoldEconomics {
    pub seq: usize,
    pub at: u64,
    pub s: u64,
    pub sigma: u64,
    pub v_prime: Option<u64>,
    pub h_pct: Option<f64>,
    pub t: u64,
    pub requests_after: usize,
    /// `(S − σ) × requests_after`: an estimate, outside the totals.
    pub saved_so_far: u64,
    pub turns_to_next_fold: Option<usize>,
    /// `T + σ − S`.
    pub net_token_delta: i64,
    /// `(w − r)·T + q·σ − r·S`, in input-token units.
    pub one_time_cost_units: f64,
    /// `(S − σ)·r`.
    pub per_turn_saving_units: f64,
    pub breakeven_turns: Option<f64>,
    /// The requests after it reached the breakeven point.
    pub paid_back: Option<bool>,
    /// The requests until the next fold reached it: a cadence signal.
    pub cadence_ok: Option<bool>,
}

pub fn compute_fold_economics(fold: &FoldEconomicsInput, price: &PriceProfile) -> FoldEconomics {
    let PriceProfile { w, r, q } = *price;
    let (s, sigma, t) = (fold.s as f64, fold.sigma as f64, fold.t as f64);
    let saving_per_turn = s - sigma;
    let one_time_cost = (w - r) * t + q * sigma - r * s;
    let per_turn_saving = saving_per_turn * r;
    let breakeven_turns = (per_turn_saving > 0.0).then(|| one_time_cost.max(0.0) / per_turn_saving);
    FoldEconomics {
        seq: fold.seq,
        at: fold.at,
        s: fold.s,
        sigma: fold.sigma,
        v_prime: fold.v_prime,
        h_pct: fold.h_pct,
        t: fold.t,
        requests_after: fold.requests_after,
        saved_so_far: fold.s.saturating_sub(fold.sigma) * fold.requests_after as u64,
        turns_to_next_fold: fold.turns_to_next_fold,
        net_token_delta: fold.t as i64 + fold.sigma as i64 - fold.s as i64,
        one_time_cost_units: round1(one_time_cost),
        per_turn_saving_units: round1(per_turn_saving),
        breakeven_turns,
        paid_back: breakeven_turns.map(|turns| fold.requests_after as f64 >= turns),
        cadence_ok: fold
            .turns_to_next_fold
            .zip(breakeven_turns)
            .map(|(turns, breakeven)| turns as f64 >= breakeven),
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct EconomicsSummary {
    pub folds: usize,
    pub gross_saved: u64,
    pub repay_cost: u64,
    pub summary_cost: u64,
    /// Gross saved less the re-pay and the summaries.
    pub net_tokens: i64,
    pub paid_back_count: usize,
    pub not_paid_back_count: usize,
    pub unobserved_count: usize,
}

pub fn summarize_fold_economics(folds: &[FoldEconomics]) -> EconomicsSummary {
    let gross_saved = folds.iter().map(|fold| fold.saved_so_far).sum();
    let repay_cost = folds.iter().map(|fold| fold.t).sum();
    let summary_cost = folds.iter().map(|fold| fold.sigma).sum();
    let count = |verdict: Option<bool>| {
        folds
            .iter()
            .filter(|fold| fold.paid_back == verdict)
            .count()
    };
    EconomicsSummary {
        folds: folds.len(),
        gross_saved,
        repay_cost,
        summary_cost,
        net_tokens: gross_saved as i64 - repay_cost as i64 - summary_cost as i64,
        paid_back_count: count(Some(true)),
        not_paid_back_count: count(Some(false)),
        unobserved_count: count(None),
    }
}

/// One request in the report.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CacheReportLine {
    pub seq: usize,
    pub at: u64,
    pub input: u64,
    pub cached: u64,
    pub output: u64,
    pub hit_pct: f64,
    pub missed: u64,
    pub new_content: u64,
    pub comp_repay: u64,
    pub ttl_repay: u64,
    /// The fold credited with the re-pay.
    pub fold_seq: Option<usize>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CacheReportOptions {
    pub price_profile: PriceProfile,
    /// The newest lines kept.
    pub max_lines: usize,
}

impl Default for CacheReportOptions {
    fn default() -> Self {
        Self {
            price_profile: PriceProfile::default(),
            max_lines: 512,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct CacheReport {
    pub generated_at: u64,
    pub profile: PriceProfile,
    pub totals: CacheTotals,
    pub economics: EconomicsSummary,
    pub folds: Vec<FoldEconomics>,
    pub lines: Vec<CacheReportLine>,
    /// Older lines left out of `lines`; the totals still count them.
    pub lines_omitted: usize,
}

/// The `acp_cache` report: every request's miss split into new content,
/// compression re-pay and the unattributed rest, so the totals reconcile
/// exactly, and each fold's economics. `now_ms` is when it is generated.
pub fn build_cache_report(
    samples: &[CacheSample],
    folds: &[FoldEvent],
    options: &CacheReportOptions,
    now_ms: u64,
) -> CacheReport {
    let mut samples = samples.to_vec();
    samples.sort_by_key(|sample| sample.at);
    let mut folds = folds.to_vec();
    folds.sort_by_key(|fold| (fold.at, fold.tokens_compressed));
    let max_lines = if options.max_lines > 0 {
        options.max_lines
    } else {
        512
    };
    let decompositions = decompose_all(&samples, &folds);

    let mut totals = CacheTotals {
        requests: samples.len(),
        ..CacheTotals::default()
    };
    for (sample, (decomposition, _)) in samples.iter().zip(&decompositions) {
        totals.input += sample.input;
        totals.cached += sample.cached;
        totals.output += sample.output.unwrap_or(0);
        totals.new_content += decomposition.new_content;
        totals.comp_repay += decomposition.comp_repay;
        totals.ttl_repay += decomposition.ttl_repay;
    }
    totals.hit_pct = round1(hit_pct(totals.input, totals.cached));
    totals.residual = totals.input as i64
        - totals.cached as i64
        - (totals.new_content + totals.comp_repay + totals.ttl_repay) as i64;
    totals.balanced = totals.residual == 0;

    let lines: Vec<CacheReportLine> = samples
        .iter()
        .zip(&decompositions)
        .enumerate()
        .map(
            |(index, (sample, (decomposition, fold_seq)))| CacheReportLine {
                seq: index + 1,
                at: sample.at,
                input: sample.input,
                cached: sample.cached,
                output: sample.output.unwrap_or(0),
                hit_pct: round1(hit_pct(sample.input, sample.cached)),
                missed: decomposition.missed,
                new_content: decomposition.new_content,
                comp_repay: decomposition.comp_repay,
                ttl_repay: decomposition.ttl_repay,
                fold_seq: *fold_seq,
            },
        )
        .collect();
    let lines_omitted = lines.len().saturating_sub(max_lines);

    let fold_economics: Vec<FoldEconomics> = folds
        .iter()
        .enumerate()
        .map(|(index, fold)| {
            let seq = index + 1;
            let after: Vec<&CacheSample> = samples
                .iter()
                .filter(|sample| sample.at > fold.at)
                .collect();
            let turns_to_next_fold = folds
                .get(index + 1)
                .map(|next| after.iter().filter(|sample| sample.at <= next.at).count());
            let input = FoldEconomicsInput {
                seq,
                at: fold.at,
                s: fold.tokens_compressed,
                sigma: fold.summary_tokens.unwrap_or(0),
                v_prime: fold.view_after,
                // The first request after the fold is the one that paid for it.
                h_pct: after
                    .first()
                    .map(|sample| round1(hit_pct(sample.input, sample.cached))),
                t: decompositions
                    .iter()
                    .filter(|(_, fold_seq)| *fold_seq == Some(seq))
                    .map(|(decomposition, _)| decomposition.comp_repay)
                    .sum(),
                requests_after: after.len(),
                turns_to_next_fold,
            };
            compute_fold_economics(&input, &options.price_profile)
        })
        .collect();

    CacheReport {
        generated_at: now_ms,
        profile: options.price_profile,
        totals,
        economics: summarize_fold_economics(&fold_economics),
        folds: fold_economics,
        lines: lines[lines_omitted..].to_vec(),
        lines_omitted,
    }
}

/// Each sample's decomposition and the fold (1-based) it credits. A fold
/// claims its first following sample whatever it shows, then every cold
/// one until a warm sample shows the provider cached again or it claimed
/// [`FOLD_CLAIM_COLD_ROUNDS`]; a warm sample retires every open fold.
fn decompose_all(
    samples: &[CacheSample],
    folds: &[FoldEvent],
) -> Vec<(SampleDecomposition, Option<usize>)> {
    let mut claims = vec![0usize; folds.len()];
    let mut retired = vec![false; folds.len()];
    let mut out = Vec::with_capacity(samples.len());
    for (index, sample) in samples.iter().enumerate() {
        let previous = index.checked_sub(1).map(|previous| &samples[previous]);
        let warm =
            sample.input > 0 && hit_pct(sample.input, sample.cached) >= FOLD_CLAIM_WARM_HIT_PCT;
        let mut pending = Vec::new();
        let mut pending_indexes = Vec::new();
        for (fold_index, fold) in folds.iter().enumerate() {
            if fold.at > sample.at {
                break;
            }
            if !retired[fold_index] && (claims[fold_index] == 0 || !warm) {
                pending.push(*fold);
                pending_indexes.push(fold_index);
            }
        }
        let decomposition = decompose_sample(previous, sample, &pending);
        // The kernel offsets the owner from the first pending fold, not
        // through the pending list, which differs once a fold between them
        // retired; kept as the kernel reports it.
        let fold_seq = decomposition
            .fold_index
            .map(|owner| pending_indexes[0] + owner + 1);
        out.push((decomposition, fold_seq));
        for fold_index in pending_indexes {
            claims[fold_index] += 1;
            if claims[fold_index] >= FOLD_CLAIM_COLD_ROUNDS {
                retired[fold_index] = true;
            }
        }
        if warm {
            for (fold_index, fold) in folds.iter().enumerate() {
                if fold.at <= sample.at {
                    retired[fold_index] = true;
                }
            }
        }
    }
    out
}

fn hit_pct(input: u64, cached: u64) -> f64 {
    if input > 0 {
        cached as f64 / input as f64 * 100.0
    } else {
        0.0
    }
}

fn round1(value: f64) -> f64 {
    js_round(value * 10.0) / 10.0
}
