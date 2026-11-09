// Derived from billion-context (https://github.com/ranxianglei/billion-context),
// tests of `kernel/tests` of acp-kernel 0.0.189 at commit `4be21b08e`.
// Copyright (c) 2026 ranxianglei. MIT License with an additional attribution
// term (NOTICE).
// Ports `panel.test.ts`.

use hanafubuki_agent_compression::{
    build_status_panel, cache_hit_stats, default_count_tokens, format_compact_tokens,
    format_hit_rate, topic_fallback, Block, CacheUsageSample, CompressibleRange, ContextBreakdown,
    NudgeDecision, State, StatusPanelInput, VIABLE_RANGE_MIN_TOKENS,
};

use super::golden_state;
use crate::fixtures::crossed;

fn sent_view(growth: i64) -> ContextBreakdown {
    ContextBreakdown {
        tool: 20_000,
        text: 4_000,
        total: 24_000,
        growth,
        ..ContextBreakdown::default()
    }
}

fn idle(reason: &str, breakdown: ContextBreakdown) -> NudgeDecision {
    NudgeDecision {
        reason: reason.to_owned(),
        context_breakdown: Some(breakdown),
        ..NudgeDecision::default()
    }
}

fn input<'a>(
    state: &'a State,
    nudge: Option<&'a NudgeDecision>,
    token_count: u64,
) -> StatusPanelInput<'a> {
    StatusPanelInput {
        version: None,
        token_count,
        system_prompt_tokens: 0,
        state,
        nudge,
        model_context_limit: 1_000_000,
        unpruned_tokens: None,
        cache_usages: None,
    }
}

fn range(start: &str, end: &str, tokens: u64) -> CompressibleRange {
    CompressibleRange {
        start_ref: start.to_owned(),
        end_ref: end.to_owned(),
        tokens,
        ..CompressibleRange::default()
    }
}

#[test]
fn the_panel_separates_session_accounting_from_the_sent_view() {
    let mut nudge = idle(
        "idle — max compressible 8106 < threshold 50000",
        sent_view(6_100),
    );
    nudge.compressible_ranges = vec![
        range("m00002", "m00005", 16),
        range("m00010", "m00040", 8_106),
    ];
    nudge.context_usage = 0.43;
    let state = State::default();
    let text = build_status_panel(&StatusPanelInput {
        version: Some("acp-kernel@0.0.36 (test)"),
        ..input(&state, Some(&nudge), 430_000)
    });
    assert!(text.contains("Context (session accounting, host footer scale): 43% (430k / 1.0M) — includes compressed originals; shrinks slower than the sent view"));
    assert!(text.contains("Sent to LLM (after compression, est.): 24k (2% of limit)"));
    assert!(!text.contains("Session-only"));
    assert!(text.contains("Token Breakdown (sent view):"));
    assert!(!text.contains("Framework"));
    let tool_line = text
        .lines()
        .find(|line| line.trim().starts_with("Tool"))
        .unwrap();
    assert!(tool_line.contains(" 83%"), "{tool_line}");
    // Below the viability floor, a range is not listed.
    assert!(!text.contains("m00002–m00005"));
    assert!(text.contains("m00010–m00040"));
}

#[test]
fn the_panel_renders_blocks_with_a_topic_fallback_and_no_version_line_when_omitted() {
    let state = State {
        blocks: vec![Block {
            block_id: "b1".to_owned(),
            summary: "Plugin discovery and registration walkthrough.".to_owned(),
            compressed_tokens: 25_000,
            created_at: 1,
            ..Block::default()
        }],
        ..State::default()
    };
    let mut state = state;
    state.stats.tokens_compressed = 25_000;
    let text = build_status_panel(&StatusPanelInput {
        model_context_limit: 200_000,
        ..input(&state, None, 1_000)
    });
    assert!(text.contains("Blocks: 1 active / 1 total (25k tokens compressed, cumulative)"));
    let line = text
        .lines()
        .find(|line| line.contains("[b1] T1 25k→"))
        .unwrap();
    assert!(
        line.ends_with(": Plugin discovery and registrat…"),
        "{line}"
    );
    assert!(!text.contains("acp-kernel@"));
}

#[test]
fn the_viability_floor_stays_coupled_to_the_summary_rules() {
    assert_eq!(VIABLE_RANGE_MIN_TOKENS, 200);
}

#[test]
fn session_only_derives_on_the_estimation_scale() {
    let nudge = idle("idle", sent_view(0));
    let state = State::default();
    let text = build_status_panel(&StatusPanelInput {
        unpruned_tokens: Some(134_000),
        ..input(&state, Some(&nudge), 430_000)
    });
    assert!(text
        .contains("Session-only (compressed originals, est.): 110k — pruned from every request"));
}

#[test]
fn session_only_stays_on_the_cjk_aware_scale() {
    let nudge = idle("idle", sent_view(0));
    let state = State::default();
    let unpruned = 24_000 + default_count_tokens(&"中".repeat(1000));
    let text = build_status_panel(&StatusPanelInput {
        unpruned_tokens: Some(unpruned),
        ..input(&state, Some(&nudge), 430_000)
    });
    assert!(text
        .contains("Session-only (compressed originals, est.): 1.0k — pruned from every request"));
}

#[test]
fn the_topic_fallback_takes_the_first_sentence_up_to_30_characters() {
    assert_eq!(
        topic_fallback("Database migration steps failed twice."),
        "Database migration steps faile…"
    );
    assert_eq!(
        topic_fallback("He said \"hello\". More text."),
        "He said \"hello\""
    );
    assert_eq!(topic_fallback("Short."), "Short");
}

#[test]
fn the_topic_fallback_truncates_long_segments_with_an_ellipsis() {
    let out = topic_fallback(&"A".repeat(50));
    assert_eq!(out.chars().count(), 31);
    assert!(out.ends_with('…'));
}

#[test]
fn compact_tokens_match_the_footer_thresholds() {
    assert_eq!(format_compact_tokens(999), "999");
    assert_eq!(format_compact_tokens(9_500), "9.5k");
    assert_eq!(format_compact_tokens(430_000), "430k");
    assert_eq!(format_compact_tokens(1_500_000), "1.5M");
}

fn usage(input: u64, cache_read: u64, cache_write: u64) -> CacheUsageSample {
    CacheUsageSample {
        input,
        cache_read,
        cache_write,
    }
}

#[test]
fn the_session_hit_rate_is_weighted_by_billed_prompt_tokens() {
    let out = cache_hit_stats(&[usage(1_000, 99_000, 0), usage(10_000, 0, 90_000)]);
    assert_eq!(out.requests, 2);
    assert_eq!(out.cache_read, 99_000);
    assert_eq!(out.billed_prompt, 200_000);
    assert_eq!(out.session, Some(99_000.0 / 200_000.0));
    // A write-only request still counts as the last one.
    assert_eq!(out.last, Some(0.0));
}

#[test]
fn requests_without_cache_reporting_are_left_out() {
    let out = cache_hit_stats(&[usage(5_000, 0, 0), usage(0, 80_000, 0)]);
    assert_eq!(out.requests, 1);
    assert_eq!(out.session, Some(1.0));
    assert_eq!(out.last, Some(1.0));
    let none = cache_hit_stats(&[usage(5_000, 0, 0)]);
    assert_eq!(none.requests, 0);
    assert_eq!(none.session, None);
    assert_eq!(none.last, None);
}

#[test]
fn hit_rates_clamp_and_show_one_decimal() {
    assert_eq!(format_hit_rate(0.923), "92.3%");
    assert_eq!(format_hit_rate(1.0), "100.0%");
    assert_eq!(format_hit_rate(1.2), "100.0%");
    assert_eq!(format_hit_rate(0.0), "0.0%");
}

#[test]
fn the_cache_line_shows_only_with_a_cache_signal() {
    let state = State::default();
    let with_cache = [usage(1_000, 99_000, 0), usage(0, 180_000, 20_000)];
    let text = build_status_panel(&StatusPanelInput {
        cache_usages: Some(&with_cache),
        ..input(&state, None, 430_000)
    });
    // session = (99k + 180k) / (100k + 200k) = 93.0%; last = 180k / 200k = 90.0%
    assert!(text.contains("Prompt cache (provider-reported): 90.0% last · 93.0% session avg — 279k of 300k billed prompt tokens served from cache (2 req)"));
    let no_signal = [usage(5_000, 0, 0)];
    let text = build_status_panel(&StatusPanelInput {
        cache_usages: Some(&no_signal),
        ..input(&state, None, 430_000)
    });
    assert!(!text.contains("Prompt cache"));
    assert!(!build_status_panel(&input(&state, None, 430_000)).contains("Prompt cache"));
}

#[test]
fn the_panel_is_byte_identical_to_the_kernel() {
    let state = golden_state();
    let nudge = NudgeDecision {
        should_inject: false,
        reason: "idle reason".to_owned(),
        ..crossed()
    };
    let cache = [usage(1000, 99_000, 0), usage(0, 180_000, 20_000)];
    let text = build_status_panel(&StatusPanelInput {
        version: Some("hanafubuki-agent test"),
        system_prompt_tokens: 5000,
        unpruned_tokens: Some(90_000),
        cache_usages: Some(&cache),
        ..input(&state, Some(&nudge), 430_000)
    });
    assert_eq!(text, include_str!("../golden/status-panel.txt"));
}
