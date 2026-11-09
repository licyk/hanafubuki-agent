// Derived from billion-context (https://github.com/ranxianglei/billion-context),
// tests of `kernel/tests` of acp-kernel 0.0.189 at commit `4be21b08e`.
// Copyright (c) 2026 ranxianglei. MIT License with an additional attribution
// term (NOTICE).
// Ports `cache-report.test.ts`; the full report texts were checked against
// the kernel's own output (TZ=UTC).

use hanafubuki_agent_compression::{
    acp_tools, build_cache_report, compute_fold_economics, decompose_sample, format_cache_report,
    CacheDetail, CacheReport, CacheReportOptions, CacheSample, FoldEconomicsInput, FoldEvent,
    FormatCacheReportOptions, PriceProfile, ACP_CACHE_TOOL_DESCRIPTION, ACP_CACHE_TOOL_NAME,
    ACP_READONLY_TOOLS, ACP_TOOL_NAMES, FOLD_CLAIM_COLD_ROUNDS,
};

/// 2026-09-15T10:00:00Z.
const T0: u64 = 1_789_466_400_000;
const MIN: u64 = 60_000;
const HALF_MIN: u64 = 30_000;

fn sample(minute: u64, input: u64, cached: u64) -> CacheSample {
    CacheSample {
        at: T0 + minute * MIN,
        input,
        cached,
        output: Some(0),
    }
}

fn fold(at: u64, tokens_compressed: u64, first_fold_start_tokens: u64) -> FoldEvent {
    FoldEvent {
        at,
        tokens_compressed,
        first_fold_start_tokens: Some(first_fold_start_tokens),
        ..FoldEvent::default()
    }
}

fn report(samples: &[CacheSample], folds: &[FoldEvent]) -> CacheReport {
    build_cache_report(samples, folds, &CacheReportOptions::default(), T0)
}

fn text(report: &CacheReport, label: Option<&str>, detail: CacheDetail) -> String {
    format_cache_report(
        report,
        label,
        &FormatCacheReportOptions {
            detail,
            utc_offset_seconds: 0,
        },
    )
}

fn round1(value: f64) -> f64 {
    (value * 10.0 + 0.5).floor() / 10.0
}

#[test]
fn the_identity_holds_for_arbitrary_inputs() {
    let mut seed: u64 = 42;
    let mut rand = || {
        seed = (seed * 1_103_515_245 + 12_345) % 2_147_483_648;
        seed as f64 / 2_147_483_648.0
    };
    let mut samples = Vec::new();
    let mut folds = Vec::new();
    let mut t = 0u64;
    for _ in 0..300 {
        t += 1 + (rand() * 5.0).floor() as u64;
        let input = (5_000.0 + rand() * 90_000.0).floor() as u64;
        let cached = (rand() * (input + 1) as f64).floor() as u64;
        samples.push(CacheSample {
            at: T0 + t * MIN,
            input,
            cached,
            output: None,
        });
        if rand() < 0.2 {
            let tokens = (5_000.0 + rand() * 50_000.0).floor() as u64;
            let start = (rand() * 20_000.0).floor() as u64;
            folds.push(fold(T0 + t * MIN - HALF_MIN, tokens, start));
        }
    }
    let report = report(&samples, &folds);
    assert!(report.totals.balanced);
    assert_eq!(report.totals.residual, 0);
    let totals = &report.totals;
    assert_eq!(
        totals.input,
        totals.cached + totals.new_content + totals.comp_repay + totals.ttl_repay
    );
    for line in &report.lines {
        assert_eq!(
            line.missed,
            line.new_content + line.comp_repay + line.ttl_repay
        );
        assert_eq!(line.missed, line.input.saturating_sub(line.cached));
    }
}

#[test]
fn a_pure_append_session_is_new_content_and_its_cold_start_is_residual() {
    let samples = [
        sample(0, 1000, 0),
        sample(1, 2000, 1000),
        sample(2, 3000, 2000),
    ];
    let report = report(&samples, &[]);
    assert_eq!(report.totals.new_content, 2000);
    assert_eq!(report.totals.comp_repay, 0);
    assert_eq!(report.totals.ttl_repay, 1000);
    assert!(report.totals.balanced);
}

#[test]
fn a_cold_cache_without_folds_lands_in_the_unattributed_bucket() {
    let report = report(&[sample(0, 1000, 1000), sample(30, 1000, 400)], &[]);
    let last = report.lines.last().unwrap();
    assert_eq!(last.missed, 600);
    assert_eq!(last.new_content, 0);
    assert_eq!(last.comp_repay, 0);
    assert_eq!(last.ttl_repay, 600);
}

#[test]
fn a_fold_cliff_credits_the_structural_excess_to_the_fold() {
    let samples = [sample(0, 10_000, 9_000), sample(1, 6_000, 1_000)];
    let folds = [FoldEvent {
        view_before: Some(10_000),
        view_after: Some(5_500),
        ..fold(T0 + HALF_MIN, 5_000, 3_000)
    }];
    let report = report(&samples, &folds);
    let line = &report.lines[1];
    assert_eq!(line.missed, 5_000);
    assert_eq!(line.new_content, 0);
    assert_eq!(line.comp_repay, 3_000);
    assert_eq!(line.ttl_repay, 2_000);
    assert_eq!(line.fold_seq, Some(1));
    let fold = &report.folds[0];
    assert_eq!(fold.t, 3_000);
    assert_eq!(fold.h_pct, Some(round1(1_000.0 / 6_000.0 * 100.0)));
    assert!(fold.breakeven_turns.is_some_and(|turns| turns > 0.0));
}

#[test]
fn the_earliest_divergence_wins_among_folds_in_one_window() {
    let samples = [sample(0, 10_000, 9_000), sample(1, 7_000, 2_000)];
    let folds = [
        fold(T0 + 18_000, 2_000, 5_000),
        fold(T0 + 42_000, 3_000, 2_000),
    ];
    let report = report(&samples, &folds);
    let line = &report.lines[1];
    assert_eq!(line.comp_repay, 5_000);
    assert_eq!(line.ttl_repay, 0);
    assert_eq!(line.fold_seq, Some(2));
    assert_eq!(report.folds[1].t, 5_000);
}

#[test]
fn a_fold_without_a_later_request_is_paid_back_and_its_cadence_unobserved() {
    let report = report(
        &[sample(0, 10_000, 9_000)],
        &[fold(T0 + HALF_MIN, 5_000, 3_000)],
    );
    let fold = &report.folds[0];
    assert_eq!(fold.h_pct, None);
    assert_eq!(fold.t, 0);
    assert_eq!(fold.paid_back, Some(true));
    assert_eq!(fold.cadence_ok, None);
    assert_eq!(report.economics.paid_back_count, 1);
    assert_eq!(report.economics.unobserved_count, 0);
}

#[test]
fn a_fold_saving_nothing_has_no_verdict() {
    let samples = [sample(0, 70_000, 60_000), sample(1, 22_000, 5_000)];
    let folds = [FoldEvent {
        summary_tokens: Some(3_000),
        ..fold(T0 + HALF_MIN, 2_000, 5_000)
    }];
    let report = report(&samples, &folds);
    let fold = &report.folds[0];
    assert_eq!(fold.breakeven_turns, None);
    assert_eq!(fold.paid_back, None);
    assert_eq!(fold.cadence_ok, None);
    assert_eq!(report.economics.unobserved_count, 1);
}

fn big_fold() -> FoldEvent {
    FoldEvent {
        summary_tokens: Some(2_000),
        view_before: Some(70_000),
        view_after: Some(20_000),
        ..fold(T0 + HALF_MIN, 50_000, 5_000)
    }
}

/// From 70K/60K, a request every minute after the fold growing by 2K.
fn recovering(count: u64) -> Vec<CacheSample> {
    let mut samples = vec![sample(0, 70_000, 60_000), sample(1, 22_667, 7_000)];
    for minute in 2..count {
        let input = 24_000 + (minute - 2) * 2_000;
        samples.push(sample(minute, input, input - 2_000));
    }
    samples
}

#[test]
fn the_payback_verdict_uses_the_whole_post_fold_window() {
    let report = report(&recovering(2), &[big_fold()]);
    let fold = &report.folds[0];
    assert_eq!(fold.s, 50_000);
    assert!(fold.t > 0);
    assert!(fold
        .breakeven_turns
        .is_some_and(|turns| turns > 0.0 && turns < 10.0));
    assert_eq!(fold.turns_to_next_fold, None);
    assert_eq!(fold.paid_back, Some(false));
    assert_eq!(fold.cadence_ok, None);
    assert_eq!(report.economics.not_paid_back_count, 1);
}

#[test]
fn the_last_fold_is_paid_back_after_enough_requests() {
    let report = report(&recovering(6), &[big_fold()]);
    let fold = &report.folds[0];
    assert_eq!(fold.turns_to_next_fold, None);
    assert_eq!(fold.requests_after, 5);
    assert!(fold
        .breakeven_turns
        .is_some_and(|turns| fold.requests_after as f64 >= turns));
    assert_eq!(fold.paid_back, Some(true));
    assert_eq!(fold.cadence_ok, None);
}

#[test]
fn a_back_to_back_fold_is_paid_back_but_its_cadence_is_not_ok() {
    let folds = [
        big_fold(),
        FoldEvent {
            summary_tokens: Some(800),
            ..fold(T0 + MIN + HALF_MIN, 10_000, 3_000)
        },
    ];
    let report = report(&recovering(10), &folds);
    let first = &report.folds[0];
    assert_eq!(first.turns_to_next_fold, Some(1));
    assert_eq!(first.requests_after, 9);
    assert_eq!(first.paid_back, Some(true));
    assert_eq!(first.cadence_ok, Some(false));
    assert!(report.economics.paid_back_count >= 1);
}

#[test]
fn the_economics_summary_aggregates_the_folds() {
    let samples = [
        sample(0, 70_000, 60_000),
        sample(1, 22_000, 5_000),
        sample(2, 24_000, 22_000),
        sample(3, 26_000, 24_000),
    ];
    let folds = [FoldEvent {
        summary_tokens: Some(2_000),
        ..fold(T0 + HALF_MIN, 50_000, 5_000)
    }];
    let economics = report(&samples, &folds).economics;
    assert_eq!(economics.folds, 1);
    assert_eq!(economics.summary_cost, 2_000);
    assert_eq!(economics.gross_saved, (50_000 - 2_000) * 3);
    assert_eq!(
        economics.net_tokens,
        economics.gross_saved as i64 - economics.repay_cost as i64 - economics.summary_cost as i64
    );
    assert_eq!(
        economics.paid_back_count + economics.not_paid_back_count + economics.unobserved_count,
        1
    );
}

#[test]
fn line_retention_keeps_the_newest_and_the_totals_keep_everything() {
    let samples: Vec<CacheSample> = (0..10).map(|i| sample(i, 1_000 + i * 100, 0)).collect();
    let options = CacheReportOptions {
        max_lines: 3,
        ..CacheReportOptions::default()
    };
    let report = build_cache_report(&samples, &[], &options, T0);
    assert_eq!(report.lines.len(), 3);
    assert_eq!(report.lines_omitted, 7);
    assert_eq!(report.lines[0].seq, 8);
    assert_eq!(report.totals.requests, 10);
    assert_eq!(
        report.totals.input,
        samples.iter().map(|sample| sample.input).sum::<u64>()
    );
}

#[test]
fn without_pending_folds_the_whole_remainder_is_unattributed() {
    let decomposition = decompose_sample(
        Some(&sample(0, 10_000, 9_000)),
        &sample(1, 10_500, 4_000),
        &[],
    );
    assert_eq!(decomposition.new_content, 500);
    assert_eq!(decomposition.comp_repay, 0);
    assert_eq!(decomposition.ttl_repay, 6_000);
    assert_eq!(decomposition.fold_index, None);
}

#[test]
fn a_hit_beyond_the_divergence_still_clamps_cleanly() {
    let decomposition = decompose_sample(
        Some(&sample(0, 10_000, 9_000)),
        &sample(1, 6_000, 4_000),
        &[fold(T0 + HALF_MIN, 5_000, 3_000)],
    );
    assert_eq!(decomposition.missed, 2_000);
    assert_eq!(decomposition.comp_repay, 2_000);
    assert_eq!(decomposition.ttl_repay, 0);
}

fn cliff() -> CacheReport {
    let folds = [FoldEvent {
        view_after: Some(5_500),
        ..fold(T0 + HALF_MIN, 5_000, 3_000)
    }];
    report(&[sample(0, 10_000, 9_000), sample(1, 6_000, 1_000)], &folds)
}

#[test]
fn the_formatter_prints_the_identity_line_and_every_section() {
    let text = text(&cliff(), Some("sess-1"), CacheDetail::Summary);
    for section in [
        "ACP CACHE REPORT (sess-1)",
        "GRAND LEDGER",
        "identity check   OK",
        "FOLD ECONOMICS",
        "LINE ITEMS",
    ] {
        assert!(text.contains(section), "{section} in\n{text}");
    }
    assert!(text.contains("PAID BACK") || text.contains('?'));
}

#[test]
fn the_unattributed_bucket_is_named_honestly() {
    for detail in [CacheDetail::Summary, CacheDetail::Full] {
        let text = text(&cliff(), Some("s392"), detail);
        assert!(text.contains("upstream-ttl-or-client-rewrite (unattributed)"));
        assert!(text.contains("kernel cannot distinguish"));
        assert!(!text.contains("ttl/other"));
    }
    assert!(ACP_CACHE_TOOL_DESCRIPTION.contains("upstream-ttl-or-client-rewrite"));
    assert!(!ACP_CACHE_TOOL_DESCRIPTION.contains("TTL expiry"));
}

#[test]
fn acp_cache_is_a_read_only_core_tool() {
    assert_eq!(ACP_CACHE_TOOL_NAME, "acp_cache");
    assert!(acp_tools().iter().any(|tool| tool.name == "acp_cache"));
    assert!(ACP_TOOL_NAMES.contains(&"acp_cache"));
    assert!(ACP_READONLY_TOOLS.contains(&"acp_cache"));
    assert!(!ACP_CACHE_TOOL_DESCRIPTION.contains("loop"));
}

#[test]
fn fold_economics_follow_the_default_price_formulas() {
    let input = FoldEconomicsInput {
        seq: 1,
        at: 1_000_000,
        s: 50_000,
        sigma: 2_000,
        h_pct: Some(33.0),
        t: 17_000,
        requests_after: 21,
        turns_to_next_fold: Some(21),
        ..FoldEconomicsInput::default()
    };
    let economics = compute_fold_economics(&input, &PriceProfile::default());
    assert_eq!(economics.one_time_cost_units, 18_300.0);
    assert_eq!(economics.per_turn_saving_units, 4_800.0);
    assert!((economics.breakeven_turns.unwrap() - 18_300.0 / 4_800.0).abs() < 1e-9);
    assert_eq!(economics.paid_back, Some(true));
    assert_eq!(economics.cadence_ok, Some(true));
    assert_eq!(economics.saved_so_far, 48_000 * 21);
    assert_eq!(economics.net_token_delta, 17_000 + 2_000 - 50_000);

    // The next fold came at once, long after this one paid back.
    let immediate = compute_fold_economics(
        &FoldEconomicsInput {
            turns_to_next_fold: Some(0),
            ..input
        },
        &PriceProfile::default(),
    );
    assert_eq!(immediate.paid_back, Some(true));
    assert_eq!(immediate.cadence_ok, Some(false));
    assert_eq!(immediate.saved_so_far, 48_000 * 21);
}

/// Thirty healthy requests and ten small folds.
fn healthy_with_folds() -> CacheReport {
    let samples: Vec<CacheSample> = (0..30).map(|i| sample(i, 100_000, 98_500)).collect();
    let folds: Vec<FoldEvent> = (0..10)
        .map(|f| FoldEvent {
            summary_tokens: Some(500),
            ..fold(T0 + (f * 3 + 2) * MIN + HALF_MIN, 5_000 + f, 3_000)
        })
        .collect();
    report(&samples, &folds)
}

#[test]
fn the_summary_is_the_default_with_verdicts_and_anomalous_lines_only() {
    let text = text(&healthy_with_folds(), Some("s1"), CacheDetail::default());
    assert!(text.contains("[summary — detail:\"full\" for every fold & line]"));
    assert!(text.contains("→ HEALTHY)"));
    assert!(text
        .lines()
        .any(|line| line.contains(" PAID BACK / ") && line.ends_with(" unobserved")));
    assert!(text.contains("more folds omitted"));
    assert!(text.contains("no anomalies (30 requests, median hit 98.5%)"));
    assert!(!text.contains("10:00:00"), "no per-request rows:\n{text}");
}

#[test]
fn the_summary_shows_anomalous_lines() {
    let mut samples: Vec<CacheSample> = (0..20).map(|i| sample(i, 100_000, 99_000)).collect();
    samples.push(sample(120, 100_000, 500));
    samples.extend((0..5).map(|i| sample(121 + i, 100_000, 99_000)));
    let text = text(&report(&samples, &[]), Some("s2"), CacheDetail::Summary);
    assert!(text.contains("LINE ITEMS (anomalies: hit<85% or miss≥5000):"));
    let anomalies = text
        .lines()
        .filter(|line| line.contains("    0.5%  "))
        .count();
    assert_eq!(anomalies, 1);
    assert!(text.contains("lines omitted (median hit 99.0%"));
}

#[test]
fn the_full_detail_lists_every_fold_and_every_line() {
    let text = text(&healthy_with_folds(), Some("s3"), CacheDetail::Full);
    assert!(!text.contains("[summary"));
    let fold_rows = text
        .lines()
        .filter(|line| line.contains(" S=500") && line.starts_with("  #"))
        .count();
    assert_eq!(fold_rows, 10);
    let line_rows = text
        .lines()
        .filter(|line| line.contains("  98.5%  "))
        .count();
    assert_eq!(line_rows, 30);
    assert!(text.contains("identity check   OK"));
}

#[test]
fn the_summary_lists_every_fold_when_there_are_few() {
    let samples = [
        sample(0, 100_000, 99_000),
        sample(1, 100_000, 99_000),
        sample(10, 100_000, 10_000),
    ];
    let folds = [FoldEvent {
        summary_tokens: Some(500),
        ..fold(T0 + HALF_MIN, 20_000, 3_000)
    }];
    let text = text(&report(&samples, &folds), Some("s4"), CacheDetail::Summary);
    assert!(text
        .lines()
        .any(|line| line.starts_with("  #1 ") && line.contains("S=20.0K") && line.contains(" → ")));
    assert!(!text.contains("more folds omitted"));
}

fn head_fold() -> FoldEvent {
    FoldEvent {
        view_before: Some(100_000),
        view_after: Some(50_500),
        ..fold(T0 + HALF_MIN, 50_000, 3_000)
    }
}

#[test]
fn a_fold_claims_its_cold_tail_until_a_warm_line_retires_it() {
    let samples = [
        sample(0, 100_000, 99_000),
        sample(1, 51_000, 5_000),
        sample(2, 55_000, 5_100),
        sample(3, 60_000, 42_000),
        sample(4, 65_000, 60_000),
        sample(5, 66_000, 30_000),
    ];
    let report = report(&samples, &[head_fold()]);
    let lines = &report.lines;
    assert_eq!(lines[1].fold_seq, Some(1));
    assert_eq!(lines[1].comp_repay, 46_000);
    assert_eq!(lines[2].fold_seq, Some(1));
    assert_eq!(lines[2].comp_repay, 45_900);
    assert_eq!(lines[2].ttl_repay, 0);
    assert_eq!(lines[3].fold_seq, Some(1));
    assert_eq!(lines[3].comp_repay, 13_000);
    assert_eq!(lines[4].fold_seq, None);
    assert_eq!(lines[4].comp_repay, 0);
    assert_eq!(lines[5].fold_seq, None);
    assert_eq!(lines[5].ttl_repay, 35_000);
    assert_eq!(report.totals.ttl_repay, 36_000);
    assert_eq!(report.folds[0].t, 46_000 + 45_900 + 13_000);
}

#[test]
fn the_cold_claim_window_is_bounded_without_a_warm_line() {
    let samples = [
        sample(0, 100_000, 99_000),
        sample(1, 51_000, 5_000),
        sample(2, 52_000, 5_100),
        sample(3, 53_000, 5_200),
        sample(4, 54_000, 5_300),
        sample(5, 55_000, 5_400),
    ];
    let report = report(&samples, &[head_fold()]);
    let claimed = report
        .lines
        .iter()
        .filter(|line| line.fold_seq == Some(1))
        .count();
    assert_eq!(claimed, FOLD_CLAIM_COLD_ROUNDS);
    assert_eq!(report.lines[5].fold_seq, None);
    assert!(report.lines[5].ttl_repay > 0);
}

#[test]
fn a_warm_first_post_fold_sample_is_claimed_once_and_retires_the_fold() {
    let samples = [
        sample(0, 100_000, 99_000),
        sample(1, 51_000, 50_000),
        sample(2, 52_000, 20_000),
    ];
    let folds = [FoldEvent {
        view_before: Some(100_000),
        view_after: Some(95_500),
        ..fold(T0 + HALF_MIN, 5_000, 3_000)
    }];
    let report = report(&samples, &folds);
    assert_eq!(report.lines[1].fold_seq, Some(1));
    assert_eq!(report.lines[2].fold_seq, None);
    assert!(report.lines[2].ttl_repay > 0);
}

/// Two folds, a quiet stretch and an expiry, with outputs.
fn session() -> CacheReport {
    let with_output = |mut sample: CacheSample, output: u64| {
        sample.output = Some(output);
        sample
    };
    let samples = [
        with_output(sample(0, 70_000, 60_000), 800),
        with_output(sample(1, 22_667, 7_000), 1_200),
        with_output(sample(2, 24_000, 22_000), 300),
        sample(3, 26_000, 24_000),
        sample(4, 28_000, 26_000),
        with_output(sample(30, 30_000, 1_000), 50),
        sample(31, 32_000, 30_000),
        sample(32, 12_000, 4_000),
        sample(33, 14_000, 13_000),
    ];
    let folds = [
        big_fold(),
        FoldEvent {
            summary_tokens: Some(900),
            ..fold(T0 + 31 * MIN + HALF_MIN, 21_000, 3_000)
        },
    ];
    report(&samples, &folds)
}

const SESSION_FOLDS: [&str; 6] = [
    "",
    "FOLD ECONOMICS (2 folds @ w=1 r=0.1 q=4)",
    "  gross saved 424.2K tok · repay cost 23.7K tok · summary cost 2900 tok → net +397.6K tok",
    "",
    "  #1 10:00:30 S=50.0K σ=2000 h=30.9% T=15.7K ΔC₁=17.1K Δs=4800/turn n*=3.56 k=6 → PAID BACK",
    "  #2 10:31:30 S=21.0K σ=900 h=33.3% T=8000 ΔC₁=8700 Δs=2010/turn n*=4.33 k=— → NOT PAID BACK",
];

#[test]
fn a_session_summary_reads_as_the_kernel_writes_it() {
    let expected = [
        "ACP CACHE REPORT (sess-9) — 9 requests  [summary — detail:\"full\" for every fold & line]",
        "",
        "GRAND LEDGER",
        "  total input    258.7K tok",
        "  total cached   187.0K tok  (hit 72.3% → WATCH)",
        "  total output   2350 tok",
        "  miss breakdown (input − cached = 71.7K tok):",
        "    new content     10.3K tok  (~1148/req fresh append — not an invalidation)",
        "    compress re-pay 23.7K tok  (2 folds — re-billed prefix)",
        "    upstream-ttl-or-client-rewrite (unattributed)  37.7K tok  (stable-prefix misses: upstream TTL expiry / eviction or client-side wire rewrite — kernel cannot distinguish; top spikes: #1 10.0K · #6 27.0K, idle 26m)",
        "  identity check   OK — 258.7K = 187.0K + 10.3K + 23.7K + 37.7K (residual 0)",
        SESSION_FOLDS[0],
        SESSION_FOLDS[1],
        SESSION_FOLDS[2],
        "  verdict: 1 PAID BACK / 1 NOT PAID BACK / 0 unobserved",
        "  NOT PAID BACK = post-fold requests so far have not reached n* (early fold or session ended before payback — one-time cost, not data loss)",
        SESSION_FOLDS[4],
        SESSION_FOLDS[5],
        "",
        "LINE ITEMS (anomalies: hit<85% or miss≥5000):",
        "  #     time      input  cached    hit%      new    comp     ttl  fold",
        "     1  10:00:00    70.0K    60.0K   85.7%       0       0   10.0K  ",
        "     2  10:01:00    22.7K     7000   30.9%       0   15.7K       0  #1",
        "     6  10:30:00    30.0K     1000    3.3%    2000       0   27.0K  ",
        "     8  10:32:00    12.0K     4000   33.3%       0    8000       0  #2",
        "     9  10:33:00    14.0K    13.0K   92.9%    1000       0       0  ",
        "  … 4 lines omitted (median hit 92.9% — detail:\"full\" lists all)",
    ];
    assert_eq!(
        text(&session(), Some("sess-9"), CacheDetail::Summary),
        expected.join("\n")
    );
}

#[test]
fn a_session_in_full_reads_as_the_kernel_writes_it() {
    let expected = [
        "ACP CACHE REPORT — 9 requests",
        "",
        "GRAND LEDGER",
        "  total input    258.7K tok",
        "  total cached   187.0K tok  (hit 72.3%)",
        "  total output   2350 tok",
        "  miss breakdown (input − cached = 71.7K tok):",
        "    new content     10.3K tok  (fresh append — not an invalidation)",
        "    compress re-pay 23.7K tok  (caused by folds)",
        "    upstream-ttl-or-client-rewrite (unattributed)  37.7K tok  (stable-prefix misses: upstream TTL expiry / eviction or client-side wire rewrite — kernel cannot distinguish)",
        "  identity check   OK — 258.7K = 187.0K + 10.3K + 23.7K + 37.7K (residual 0)",
        SESSION_FOLDS[0],
        SESSION_FOLDS[1],
        SESSION_FOLDS[2],
        "  verdict: 1 paid back, 1 not paid back, 0 unobserved",
        SESSION_FOLDS[4],
        SESSION_FOLDS[5],
        "",
        "LINE ITEMS:",
        "  #     time      input  cached    hit%      new    comp     ttl  fold",
        "     1  10:00:00    70.0K    60.0K   85.7%       0       0   10.0K  ",
        "     2  10:01:00    22.7K     7000   30.9%       0   15.7K       0  #1",
        "     3  10:02:00    24.0K    22.0K   91.7%    1333       0     667  ",
        "     4  10:03:00    26.0K    24.0K   92.3%    2000       0       0  ",
        "     5  10:04:00    28.0K    26.0K   92.9%    2000       0       0  ",
        "     6  10:30:00    30.0K     1000    3.3%    2000       0   27.0K  ",
        "     7  10:31:00    32.0K    30.0K   93.8%    2000       0       0  ",
        "     8  10:32:00    12.0K     4000   33.3%       0    8000       0  #2",
        "     9  10:33:00    14.0K    13.0K   92.9%    1000       0       0  ",
    ];
    assert_eq!(
        text(&session(), None, CacheDetail::Full),
        expected.join("\n")
    );
}

#[test]
fn the_time_of_day_follows_the_offset() {
    let options = FormatCacheReportOptions {
        detail: CacheDetail::Full,
        utc_offset_seconds: 8 * 3600,
    };
    let text = format_cache_report(&session(), None, &options);
    assert!(text.contains("  #1 18:00:30 S=50.0K"));
    assert!(text.contains("     9  18:33:00 "));
}
