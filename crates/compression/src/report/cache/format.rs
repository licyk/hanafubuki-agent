// Derived from billion-context (https://github.com/ranxianglei/billion-context),
// `formatCacheReport` of `kernel/src/cache-report.ts` of acp-kernel 0.0.189
// at commit `4be21b08e`. Copyright (c) 2026 ranxianglei. MIT License with an
// additional attribution term (NOTICE).

use super::{CacheReport, CacheReportLine, FoldEconomics};
use crate::number::{format_number, js_round, to_fixed};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CacheDetail {
    /// Totals, verdicts, notable folds and anomalous requests.
    #[default]
    Summary,
    /// Every fold and every request.
    Full,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FormatCacheReportOptions {
    pub detail: CacheDetail,
    /// The local time's offset from UTC, for the times of day shown.
    pub utc_offset_seconds: i64,
}

const HIT_HEALTHY_PCT: f64 = 90.0;
const HIT_WATCH_PCT: f64 = 70.0;
const ANOMALY_HIT_PCT: f64 = 85.0;
const ANOMALY_MISS_TOK: u64 = 5_000;
const TTL_SPIKE_TOK: u64 = 10_000;
const SUMMARY_FOLD_TOPS: usize = 3;
const SUMMARY_ANOMALY_CAP: usize = 20;
const LINE_HEADER: &str = "  #     time      input  cached    hit%      new    comp     ttl  fold";

/// The report as text for the model and the user. The identity line is
/// always printed, so a broken implementation cannot hide.
pub fn format_cache_report(
    report: &CacheReport,
    session_label: Option<&str>,
    options: &FormatCacheReportOptions,
) -> String {
    let label = session_label.map_or_else(String::new, |label| format!(" ({label})"));
    let header = format!(
        "ACP CACHE REPORT{label} — {} requests",
        report.totals.requests
    );
    let clock = Clock(options.utc_offset_seconds);
    match options.detail {
        CacheDetail::Full => [header, String::new(), full(report, clock)].join("\n"),
        CacheDetail::Summary => [
            format!("{header}  [summary — detail:\"full\" for every fold & line]"),
            String::new(),
            summary(report, clock),
        ]
        .join("\n"),
    }
}

#[derive(Clone, Copy)]
struct Clock(i64);

impl Clock {
    fn time(self, at: u64) -> String {
        let seconds = (at / 1000) as i64 + self.0;
        let of_day = seconds.rem_euclid(86_400);
        format!(
            "{:02}:{:02}:{:02}",
            of_day / 3600,
            of_day % 3600 / 60,
            of_day % 60
        )
    }
}

fn tok(value: f64) -> String {
    let value = js_round(value);
    if value >= 1_000_000.0 {
        format!("{}M", to_fixed(value / 1_000_000.0, 2))
    } else if value >= 10_000.0 {
        format!("{}K", to_fixed(value / 1000.0, 1))
    } else {
        format_number(value)
    }
}

fn tok_u(value: u64) -> String {
    tok(value as f64)
}

fn signed(value: i64) -> String {
    let sign = if value >= 0 { "+" } else { "" };
    format!("{sign}{}", tok(value as f64))
}

fn verdict(fold: &FoldEconomics) -> &'static str {
    match fold.paid_back {
        None => "?",
        Some(true) => "PAID BACK",
        Some(false) => "NOT PAID BACK",
    }
}

fn hit_verdict(pct: f64) -> &'static str {
    if pct >= HIT_HEALTHY_PCT {
        "HEALTHY"
    } else if pct >= HIT_WATCH_PCT {
        "WATCH"
    } else {
        "INVESTIGATE"
    }
}

fn median_hit_pct(lines: &[&CacheReportLine]) -> Option<f64> {
    let mut hits: Vec<f64> = lines.iter().map(|line| line.hit_pct).collect();
    hits.sort_by(f64::total_cmp);
    hits.get(hits.len() / 2).copied()
}

fn idle_gap(ms: u64) -> String {
    let ms = ms as f64;
    if ms < 90_000.0 {
        format!("{}s", js_round(ms / 1000.0).max(1.0))
    } else if ms < 90.0 * 60_000.0 {
        format!("{}m", js_round(ms / 60_000.0))
    } else if ms < 36.0 * 3_600_000.0 {
        format!("{}h", to_fixed(ms / 3_600_000.0, 1))
    } else {
        format!("{}d", to_fixed(ms / 86_400_000.0, 1))
    }
}

fn fold_line(fold: &FoldEconomics, clock: Clock) -> String {
    let breakeven = fold
        .breakeven_turns
        .map_or_else(|| "n/a".to_owned(), |turns| to_fixed(turns, 2));
    let next = fold
        .turns_to_next_fold
        .map_or_else(|| "—".to_owned(), |turns| turns.to_string());
    let hit = fold
        .h_pct
        .map_or_else(|| "n/a".to_owned(), |pct| format!("{}%", to_fixed(pct, 1)));
    format!(
        "  #{} {} S={} σ={} h={hit} T={} ΔC₁={} Δs={}/turn n*={breakeven} k={next} → {}",
        fold.seq,
        clock.time(fold.at),
        tok_u(fold.s),
        tok_u(fold.sigma),
        tok_u(fold.t),
        tok(fold.one_time_cost_units),
        tok(fold.per_turn_saving_units),
        verdict(fold),
    )
}

fn request_line(line: &CacheReportLine, clock: Clock) -> String {
    let fold = line
        .fold_seq
        .map_or_else(String::new, |seq| format!("#{seq}"));
    format!(
        "  {:>4}  {}  {:>7}  {:>7}  {:>5}%  {:>6}  {:>6}  {:>6}  {fold}",
        line.seq,
        clock.time(line.at),
        tok_u(line.input),
        tok_u(line.cached),
        to_fixed(line.hit_pct, 1),
        tok_u(line.new_content),
        tok_u(line.comp_repay),
        tok_u(line.ttl_repay),
    )
}

fn identity_line(report: &CacheReport) -> String {
    let totals = &report.totals;
    format!(
        "  identity check   {} — {} = {} + {} + {} + {} (residual {})",
        if totals.balanced { "OK" } else { "BROKEN" },
        tok_u(totals.input),
        tok_u(totals.cached),
        tok_u(totals.new_content),
        tok_u(totals.comp_repay),
        tok_u(totals.ttl_repay),
        totals.residual
    )
}

fn economics_head(report: &CacheReport) -> Vec<String> {
    let economics = &report.economics;
    let profile = &report.profile;
    vec![
        String::new(),
        format!(
            "FOLD ECONOMICS ({} folds @ w={} r={} q={})",
            economics.folds,
            format_number(profile.w),
            format_number(profile.r),
            format_number(profile.q)
        ),
        format!(
            "  gross saved {} tok · repay cost {} tok · summary cost {} tok → net {} tok",
            tok_u(economics.gross_saved),
            tok_u(economics.repay_cost),
            tok_u(economics.summary_cost),
            signed(economics.net_tokens)
        ),
    ]
}

fn full(report: &CacheReport, clock: Clock) -> String {
    let totals = &report.totals;
    let mut out = vec![
        "GRAND LEDGER".to_owned(),
        format!("  total input    {} tok", tok_u(totals.input)),
        format!(
            "  total cached   {} tok  (hit {}%)",
            tok_u(totals.cached),
            to_fixed(totals.hit_pct, 1)
        ),
        format!("  total output   {} tok", tok_u(totals.output)),
        format!(
            "  miss breakdown (input − cached = {} tok):",
            tok_u(totals.input.saturating_sub(totals.cached))
        ),
        format!(
            "    new content     {} tok  (fresh append — not an invalidation)",
            tok_u(totals.new_content)
        ),
        format!(
            "    compress re-pay {} tok  (caused by folds)",
            tok_u(totals.comp_repay)
        ),
        format!(
            "    upstream-ttl-or-client-rewrite (unattributed)  {} tok  (stable-prefix misses: upstream TTL expiry / eviction or client-side wire rewrite — kernel cannot distinguish)",
            tok_u(totals.ttl_repay)
        ),
        identity_line(report),
    ];
    let economics = &report.economics;
    if economics.folds > 0 {
        out.extend(economics_head(report));
        out.push(format!(
            "  verdict: {} paid back, {} not paid back, {} unobserved",
            economics.paid_back_count, economics.not_paid_back_count, economics.unobserved_count
        ));
        out.extend(report.folds.iter().map(|fold| fold_line(fold, clock)));
    }
    if !report.lines.is_empty() {
        out.push(String::new());
        out.push(if report.lines_omitted > 0 {
            format!(
                "LINE ITEMS (last {} of {}):",
                report.lines.len(),
                report.lines.len() + report.lines_omitted
            )
        } else {
            "LINE ITEMS:".to_owned()
        });
        out.push(LINE_HEADER.to_owned());
        out.extend(report.lines.iter().map(|line| request_line(line, clock)));
    }
    out.join("\n")
}

fn summary(report: &CacheReport, clock: Clock) -> String {
    let mut out = summary_ledger(report);
    if report.economics.folds > 0 {
        out.extend(summary_folds(report, clock));
    }
    if !report.lines.is_empty() {
        out.push(String::new());
        out.extend(summary_lines(report, clock));
    }
    out.join("\n")
}

fn summary_ledger(report: &CacheReport) -> Vec<String> {
    let totals = &report.totals;
    let average_new = if totals.requests > 0 {
        totals.new_content as f64 / totals.requests as f64
    } else {
        0.0
    };
    // The three largest unattributed spikes, shown in request order.
    let mut spikes: Vec<usize> = (0..report.lines.len())
        .filter(|index| report.lines[*index].ttl_repay >= TTL_SPIKE_TOK)
        .collect();
    spikes.sort_by_key(|index| std::cmp::Reverse(report.lines[*index].ttl_repay));
    spikes.truncate(SUMMARY_FOLD_TOPS);
    spikes.sort_unstable();
    let spikes: Vec<String> = spikes
        .iter()
        .map(|index| {
            let line = &report.lines[*index];
            let idle = index
                .checked_sub(1)
                .map(|previous| &report.lines[previous])
                .filter(|previous| line.at > previous.at)
                .map_or_else(String::new, |previous| {
                    format!(", idle {}", idle_gap(line.at - previous.at))
                });
            format!("#{} {}{idle}", line.seq, tok_u(line.ttl_repay))
        })
        .collect();
    let spikes = if spikes.is_empty() {
        String::new()
    } else {
        format!("; top spikes: {}", spikes.join(" · "))
    };
    vec![
        "GRAND LEDGER".to_owned(),
        format!("  total input    {} tok", tok_u(totals.input)),
        format!(
            "  total cached   {} tok  (hit {}% → {})",
            tok_u(totals.cached),
            to_fixed(totals.hit_pct, 1),
            hit_verdict(totals.hit_pct)
        ),
        format!("  total output   {} tok", tok_u(totals.output)),
        format!(
            "  miss breakdown (input − cached = {} tok):",
            tok_u(totals.input.saturating_sub(totals.cached))
        ),
        format!(
            "    new content     {} tok  (~{}/req fresh append — not an invalidation)",
            tok_u(totals.new_content),
            tok(average_new)
        ),
        format!(
            "    compress re-pay {} tok  ({} folds — re-billed prefix)",
            tok_u(totals.comp_repay),
            report.economics.folds
        ),
        format!(
            "    upstream-ttl-or-client-rewrite (unattributed)  {} tok  (stable-prefix misses: upstream TTL expiry / eviction or client-side wire rewrite — kernel cannot distinguish{spikes})",
            tok_u(totals.ttl_repay)
        ),
        identity_line(report),
    ]
}

fn summary_folds(report: &CacheReport, clock: Clock) -> Vec<String> {
    let economics = &report.economics;
    let mut out = economics_head(report);
    out.push(format!(
        "  verdict: {} PAID BACK / {} NOT PAID BACK / {} unobserved",
        economics.paid_back_count, economics.not_paid_back_count, economics.unobserved_count
    ));
    if economics.not_paid_back_count > 0 {
        out.push("  NOT PAID BACK = post-fold requests so far have not reached n* (early fold or session ended before payback — one-time cost, not data loss)".to_owned());
    }
    if report.folds.len() <= SUMMARY_FOLD_TOPS * 2 {
        out.extend(report.folds.iter().map(|fold| fold_line(fold, clock)));
        return out;
    }
    let mut largest: Vec<&FoldEconomics> = report.folds.iter().collect();
    largest.sort_by_key(|fold| std::cmp::Reverse(fold.s));
    largest.truncate(SUMMARY_FOLD_TOPS);
    let mut worst: Vec<&FoldEconomics> = report.folds.iter().collect();
    worst.sort_by(|a, b| b.one_time_cost_units.total_cmp(&a.one_time_cost_units));
    worst.truncate(SUMMARY_FOLD_TOPS);
    let largest_text: Vec<String> = largest
        .iter()
        .map(|fold| format!("#{} S={} {}", fold.seq, tok_u(fold.s), verdict(fold)))
        .collect();
    let worst_text: Vec<String> = worst
        .iter()
        .map(|fold| {
            let next = fold
                .turns_to_next_fold
                .map_or_else(String::new, |turns| format!(", k={turns}"));
            format!(
                "#{} ΔC₁={} ({}{next})",
                fold.seq,
                tok(fold.one_time_cost_units),
                verdict(fold)
            )
        })
        .collect();
    out.push(format!("  largest folds: {}", largest_text.join(" · ")));
    out.push(format!("  worst one-time: {}", worst_text.join(" · ")));
    let mut shown: Vec<usize> = largest.iter().chain(&worst).map(|fold| fold.seq).collect();
    shown.sort_unstable();
    shown.dedup();
    if report.folds.len() > shown.len() {
        out.push(format!(
            "  → {} more folds omitted (detail:\"full\" lists every fold)",
            report.folds.len() - shown.len()
        ));
    }
    out
}

fn summary_lines(report: &CacheReport, clock: Clock) -> Vec<String> {
    let lines = &report.lines;
    let anomalous =
        |line: &CacheReportLine| line.hit_pct < ANOMALY_HIT_PCT || line.missed >= ANOMALY_MISS_TOK;
    let mut shown: Vec<usize> = (0..lines.len())
        .filter(|index| anomalous(&lines[*index]))
        .collect();
    let last = lines.len() - 1;
    if !shown.contains(&last) {
        shown.push(last);
    }
    let kept = &shown[shown.len().saturating_sub(SUMMARY_ANOMALY_CAP)..];
    let anomalies = kept
        .iter()
        .filter(|index| anomalous(&lines[**index]))
        .count();
    let outside = if report.lines_omitted > 0 {
        format!(", {} older outside window", report.lines_omitted)
    } else {
        String::new()
    };
    let median = |lines: &[&CacheReportLine]| {
        median_hit_pct(lines).map_or_else(|| "n/a".to_owned(), |median| to_fixed(median, 1))
    };
    if anomalies == 0 {
        let all: Vec<&CacheReportLine> = lines.iter().collect();
        return vec![format!(
            "LINE ITEMS — no anomalies ({} requests, median hit {}%{outside})",
            lines.len(),
            median(&all)
        )];
    }
    let mut out = vec![
        format!(
            "LINE ITEMS (anomalies: hit<{}% or miss≥{}):",
            format_number(ANOMALY_HIT_PCT),
            tok_u(ANOMALY_MISS_TOK)
        ),
        LINE_HEADER.to_owned(),
    ];
    out.extend(kept.iter().map(|index| request_line(&lines[*index], clock)));
    let omitted = lines.len() - kept.len();
    if omitted > 0 {
        let rest: Vec<&CacheReportLine> = (0..lines.len())
            .filter(|index| !kept.contains(index))
            .map(|index| &lines[index])
            .collect();
        out.push(format!(
            "  … {omitted} lines omitted (median hit {}%{outside} — detail:\"full\" lists all)",
            median(&rest)
        ));
    }
    out
}
