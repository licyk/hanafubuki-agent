// Derived from billion-context (https://github.com/ranxianglei/billion-context),
// tests of `kernel/tests` of acp-kernel 0.0.189 at commit `4be21b08e`.
// Copyright (c) 2026 ranxianglei. MIT License with an additional attribution
// term (NOTICE).
// Ports the status-report and recap cases of `decompress-report.test.ts`,
// `report-accounting.test.ts`, `report-tool-accounting.test.ts` and
// `report-truncation.test.ts`; `ranges` ports `ranges-consistency.test.ts`,
// `panel` ports `panel.test.ts`. The `golden/status-*.txt` files are the
// kernel's own `buildStatusReport` and `buildStatusPanel` output for the
// session built in `golden_state`, rendered with bun from `kernel/src` at
// the same commit.

mod panel;
mod ranges;

use hanafubuki_agent_compression::{
    build_compressible_ranges, build_recap, build_status_report, render_nudge_text, Block, Message,
    NudgeDecision, Prompts, Role, State, StatusReportOptions, StatusScope, StatusSort, StatusView,
};

use crate::fixtures::{assistant, call, config, state_for, user};

fn block(id: &str) -> Block {
    Block {
        block_id: id.to_owned(),
        run_id: "r0".to_owned(),
        summary: "summary".to_owned(),
        created_at: 1000,
        ..Block::default()
    }
}

fn with_blocks(blocks: Vec<Block>) -> State {
    State {
        blocks,
        ..State::default()
    }
}

/// A result carrying only its call id, as wire converters project it.
pub(super) fn unnamed_result(id: &str, call_id: &str, output: &str) -> Message {
    Message {
        tool_name: None,
        ..Message::tool_result(id, call_id, "", output)
    }
}

fn overview(state: &State, messages: &[Message]) -> String {
    build_status_report(state, messages, &StatusReportOptions::default())
}

fn uncompressed_messages(tool: Option<&str>) -> StatusReportOptions<'_> {
    StatusReportOptions {
        scope: Some(StatusScope::Uncompressed),
        view: StatusView::Messages,
        tool,
        ..StatusReportOptions::default()
    }
}

fn breakdown_line(report: &str) -> &str {
    report
        .lines()
        .find(|line| line.contains(" tool (") && line.contains(" text ("))
        .unwrap_or_else(|| panic!("no breakdown line in:\n{report}"))
}

/// The percentage beside a bucket of the breakdown line.
fn bucket_pct(line: &str, label: &str) -> u64 {
    let after = &line[line.find(&format!(" {label} (")).unwrap() + label.len() + 3..];
    after[..after.find('%').unwrap()].parse().unwrap()
}

// decompress-report.test.ts

#[test]
fn the_overview_shows_the_breakdown_and_the_active_blocks() {
    let mut b1 = block("b1");
    b1.summary = "compressed summary".to_owned();
    b1.effective_message_ids = vec!["old1".to_owned()];
    let report = overview(
        &with_blocks(vec![b1]),
        &[user("live1", "visible text message")],
    );
    assert!(report.contains("CONTEXT BREAKDOWN"));
    assert!(report.contains("COMPRESSED BLOCKS"));
    assert!(report.contains("b1"));
}

#[test]
fn the_compressed_scope_lists_the_blocks_with_details() {
    let mut b1 = block("b1");
    b1.summary = "first block".to_owned();
    b1.topic = Some("auth".to_owned());
    b1.survived_count = 3;
    let mut b2 = block("b2");
    b2.summary = "second block".to_owned();
    b2.topic = Some("deploy".to_owned());
    let report = build_status_report(
        &with_blocks(vec![b1, b2]),
        &[],
        &StatusReportOptions {
            scope: Some(StatusScope::Compressed),
            ..StatusReportOptions::default()
        },
    );
    assert!(report.contains("COMPRESSED — 2 blocks"));
    assert!(report.contains("\"auth\""));
    assert!(report.contains("\"deploy\""));
}

#[test]
fn the_recap_lists_every_active_block() {
    let mut b1 = block("b1");
    b1.summary = "first summary content".to_owned();
    let mut b2 = block("b2");
    b2.summary = "second".to_owned();
    let recap = build_recap(&with_blocks(vec![b1, b2]), None);
    assert!(recap.contains("Active compression blocks (2)"));
    assert!(recap.contains("b1"));
    assert!(recap.contains("b2"));
}

#[test]
fn the_recap_of_one_block_is_its_full_summary() {
    let mut b1 = block("b1");
    b1.summary = "the full detailed summary text".to_owned();
    b1.topic = Some("research".to_owned());
    let recap = build_recap(&with_blocks(vec![b1]), Some("b1"));
    assert!(recap.contains("the full detailed summary text"));
    assert!(recap.contains("research"));
}

#[test]
fn the_recap_reports_missing_and_inactive_blocks() {
    let mut b1 = block("b1");
    b1.active = false;
    let state = with_blocks(vec![b1]);
    assert!(build_recap(&state, Some("b9")).contains("not found"));
    assert!(build_recap(&state, Some("b1")).contains("inactive"));
}

// report-accounting.test.ts

#[test]
fn tool_results_count_in_the_tool_bucket() {
    let messages = [
        user("m1", "run the build"),
        call("m2", "c1", "bash", "npm test"),
        unnamed_result("m3", "c1", &"x".repeat(51_361)),
    ];
    let report = overview(&state_for(&messages), &messages);
    // tool = ceil(51361/4) + ceil(8/4) = 12843, text = ceil(13/4) = 4
    assert!(breakdown_line(&report).contains("12.8K tool (100%)"));
    assert!(breakdown_line(&report).contains("4 text (0%)"));
    assert!(report.contains("Top tools: bash (100%)"));
}

#[test]
fn the_breakdown_agrees_with_the_compressible_ranges_tool_share() {
    let messages = [
        call("m1", "c1", "bash", "ls"),
        unnamed_result("m2", "c1", &"y".repeat(40_000)),
    ];
    let state = state_for(&messages);
    let ranges = build_compressible_ranges(
        &messages,
        &state,
        &config(),
        Some(&Default::default()),
        &messages,
    );
    assert_eq!(ranges.compressible.len(), 1);
    assert_eq!(ranges.compressible[0].tool_pct, 100);
    assert_eq!(ranges.compressible[0].text_pct, 0);
    assert!(breakdown_line(&overview(&state, &messages)).contains("100%)"));
}

#[test]
fn percentages_have_no_floor_and_never_sum_past_100() {
    let messages = [
        user("m1", &"z".repeat(4000)),
        call("m2", "c1", "bash", "ls"),
    ];
    let report = overview(&state_for(&messages), &messages);
    let line = breakdown_line(&report);
    assert!(line.contains("1 tool (0%)"));
    assert!(line.contains("1.0K text (100%)"));
    let sum: u64 = ["tool", "text", "summaries"]
        .iter()
        .map(|label| bucket_pct(line, label))
        .sum();
    assert!(sum <= 100, "{line}");
}

#[test]
fn cjk_is_counted_one_token_a_character() {
    let messages = [user("m1", &"中".repeat(4000))];
    let report = overview(&state_for(&messages), &messages);
    assert!(breakdown_line(&report).contains("4.0K text (100%)"));
}

#[test]
fn a_result_without_a_resolvable_call_stays_in_the_tool_bucket() {
    let messages = [unnamed_result("m1", "orphan", &"w".repeat(4000))];
    let report = overview(&state_for(&messages), &messages);
    assert!(breakdown_line(&report).contains("1.0K tool (100%)"));
    assert!(report.contains("Top tools: tool (100%)"));
}

#[test]
fn the_message_drilldown_filters_results_by_their_resolved_tool() {
    let messages = [
        call("m1", "c1", "bash", "ls"),
        unnamed_result("m2", "c1", &"y".repeat(4000)),
    ];
    let report = build_status_report(
        &state_for(&messages),
        &messages,
        &uncompressed_messages(Some("bash")),
    );
    let header = report.lines().next().unwrap();
    assert!(
        header.contains("bash: ") && header.contains(" | 2 msgs"),
        "{header}"
    );
}

// report-tool-accounting.test.ts

fn tool_heavy_session(pairs: &[(&str, &str, usize)], text_chars: usize) -> Vec<Message> {
    let mut messages = vec![user("m1", &"a".repeat(text_chars))];
    for (call_id, tool, result_chars) in pairs {
        messages.push(call(&format!("call-{call_id}"), call_id, tool, "{}"));
        // Results come back on the user side, named only by their call id.
        messages.push(Message {
            role: Role::User,
            ..unnamed_result(
                &format!("res-{call_id}"),
                call_id,
                &"a".repeat(*result_chars),
            )
        });
    }
    messages
}

#[test]
fn tool_result_volume_goes_to_the_tool_bucket() {
    let messages = tool_heavy_session(
        &[
            ("c1", "bash", 10_000),
            ("c2", "read", 10_000),
            ("c3", "edit", 10_000),
            ("c4", "grep", 10_000),
            ("c5", "bash", 10_000),
        ],
        200,
    );
    let report = overview(&state_for(&messages), &messages);
    let line = breakdown_line(&report);
    assert!(line.contains("12.5K tool"), "{line}");
    assert!(bucket_pct(line, "tool") >= 99, "{line}");
}

#[test]
fn tool_results_resolve_to_their_calling_tool() {
    let messages = tool_heavy_session(&[("c1", "bash", 8000), ("c2", "read", 2000)], 200);
    let state = state_for(&messages);
    let report = overview(&state, &messages);
    let top = report
        .lines()
        .find(|line| line.starts_with("  Top tools:"))
        .unwrap();
    assert!(top.contains("bash") && top.contains("read"), "{top}");
    if let Some(at) = top.find("text (") {
        let pct: u64 = top[at + 6..top[at..].find('%').unwrap() + at]
            .parse()
            .unwrap();
        assert!(pct <= 5, "{top}");
    }
    let by_name = build_status_report(&state, &messages, &uncompressed_messages(Some("bash")));
    assert!(by_name.contains("m00003"), "{by_name}");
}

#[test]
fn orphan_results_fall_into_the_generic_tool_bucket() {
    let messages = [
        user("m1", &"a".repeat(200)),
        Message {
            role: Role::User,
            ..unnamed_result("m2", "missing-call", &"a".repeat(8000))
        },
    ];
    let report = overview(&state_for(&messages), &messages);
    let line = breakdown_line(&report);
    assert!(line.contains("2.0K tool"), "{line}");
    assert!(bucket_pct(line, "tool") >= 97, "{line}");
}

#[test]
fn tiny_buckets_round_to_zero_and_the_sum_stays_within_100() {
    let messages = tool_heavy_session(&[("c1", "bash", 40_000)], 4);
    let report = overview(&state_for(&messages), &messages);
    let line = breakdown_line(&report);
    let (tool, text, summaries) = (
        bucket_pct(line, "tool"),
        bucket_pct(line, "text"),
        bucket_pct(line, "summaries"),
    );
    assert_eq!(text, 0, "{line}");
    assert!(tool + text + summaries <= 100, "{line}");
}

#[test]
fn the_ranges_view_names_ranges_by_their_resolved_tool() {
    let messages = tool_heavy_session(&[("c1", "bash", 8000)], 200);
    let report = build_status_report(
        &state_for(&messages),
        &messages,
        &StatusReportOptions {
            scope: Some(StatusScope::Uncompressed),
            ..StatusReportOptions::default()
        },
    );
    let range_lines: Vec<&str> = report
        .lines()
        .filter(|line| {
            line.starts_with("  m") && line[3..].starts_with(|c: char| c.is_ascii_digit())
        })
        .collect();
    assert!(!range_lines.is_empty(), "{report}");
    assert!(
        range_lines
            .iter()
            .any(|line| line.trim_end().ends_with("bash")),
        "{report}"
    );
}

// report-truncation.test.ts

/// 35 tier-1 and 6 tier-2 active blocks: a tier-2 nudge is due while the
/// overview's 30-entry cap hides some of its targets.
fn session_state() -> State {
    let mut blocks = Vec::new();
    for index in 1..=35u64 {
        let mut block = block(&format!("b{index}"));
        block.summary = format!("tier-1 summary {index}");
        block.topic = Some(format!("topic-{index}"));
        block.compressed_tokens = 8000 + index * 100;
        block.effective_message_ids = (0..5).map(|j| format!("m{index}_{j}")).collect();
        block.created_at = 1000 + index;
        blocks.push(block);
    }
    for index in 36..=41u64 {
        let mut block = block(&format!("b{index}"));
        block.tier = 2;
        block.summary = format!("tier-2 summary {index}");
        block.topic = Some(format!("distilled-{index}"));
        block.compressed_tokens = 40_000 + index * 100;
        block.effective_message_ids = (0..40).map(|j| format!("m{index}_{j}")).collect();
        block.direct_block_ids = vec![format!("b{}", index - 3)];
        block.created_at = 5000 + index;
        blocks.push(block);
    }
    with_blocks(blocks)
}

#[test]
fn the_overview_announces_a_truncated_block_list() {
    let state = session_state();
    let report = overview(&state, &[]);
    assert!(report.contains("41 active"));
    let shown = state
        .blocks
        .iter()
        .filter(|block| report.contains(&format!("{} (", block.block_id)))
        .count();
    assert!(shown < 41);
    assert!(report.contains("11 more blocks not shown"));
    assert!(report.contains("scope:\"compressed\""));
}

#[test]
fn the_nudge_targets_and_the_overview_reconcile_once_truncation_is_announced() {
    let state = session_state();
    let targets: Vec<Block> = state
        .active_blocks()
        .filter(|block| block.tier == 1)
        .cloned()
        .collect();
    let mut decision = NudgeDecision {
        should_inject: true,
        reason: "tier-2 distillation pending".to_owned(),
        context_usage: 0.7,
        tier: Some(2),
        tier_target_blocks: targets,
        ..NudgeDecision::default()
    };
    decision.breakdown.usage = 0.7;
    decision.breakdown.has_pending_nudge = true;
    decision.breakdown.pending_t1 = 35;
    let nudge = render_nudge_text(&decision, &Prompts::default(), false).text;
    assert!(nudge.contains("Target tier-1 blocks to distill (35)"));
    for id in ["b1", "b20", "b35"] {
        assert!(nudge.contains(&format!("{id} ")), "{id}");
    }

    let report = overview(&state, &[]);
    let t1_shown = state
        .blocks
        .iter()
        .filter(|block| block.tier == 1 && report.contains(&format!("{} (T1)", block.block_id)))
        .count();
    assert!(t1_shown < 35);
    assert!(report.contains("41 active"));
    assert!(report.contains("11 more blocks not shown"));
    let tiers = report
        .lines()
        .find(|line| line.contains("Tier usage:"))
        .unwrap();
    assert!(
        tiers.contains("T1: ") && tiers.contains("(35 blocks)"),
        "{tiers}"
    );
    assert!(
        tiers.contains("T2: ") && tiers.contains("(6 blocks)"),
        "{tiers}"
    );
}

#[test]
fn the_overview_announces_nothing_when_the_list_fits() {
    let report = overview(&with_blocks(vec![block("b1"), block("b2")]), &[]);
    assert!(!report.contains("more blocks not shown"));
}

// Byte-exact against the kernel.

pub(super) fn golden_messages() -> Vec<Message> {
    vec![
        user("u1", &format!("please look {}", "a".repeat(3000))),
        call("c1", "k1", "bash", "{\"command\":\"ls\"}"),
        unnamed_result("r1", "k1", &"b".repeat(9000)),
        call("c2", "k2", "read", "{}"),
        Message::tool_result("r2", "k2", "read", "中".repeat(700)),
        assistant("a1", "done ```code```"),
        user("x1", "covered"),
    ]
}

pub(super) fn golden_state() -> State {
    let golden_block =
        |id: &str, tier, summary: String, topic: Option<&str>, tokens, created_at| Block {
            block_id: id.to_owned(),
            run_id: "r1".to_owned(),
            tier,
            summary,
            topic: topic.map(str::to_owned),
            compressed_tokens: tokens,
            created_at,
            survived_count: 2,
            ..Block::default()
        };
    let mut state = state_for(&golden_messages());
    state.blocks = vec![
        Block {
            effective_message_ids: vec!["x1".to_owned()],
            ..golden_block(
                "b1",
                1,
                "summary one about things".to_owned(),
                Some("first"),
                4000,
                10,
            )
        },
        Block {
            direct_block_ids: vec!["b2".to_owned()],
            ..golden_block(
                "b3",
                2,
                format!("tier two {}", "z".repeat(400)),
                None,
                25_000,
                20,
            )
        },
    ];
    state.stats.tokens_compressed = 29_000;
    state
}

fn golden_report(options: StatusReportOptions<'_>) -> String {
    build_status_report(&golden_state(), &golden_messages(), &options)
}

#[test]
fn the_overview_is_byte_identical_to_the_kernel() {
    assert_eq!(
        golden_report(StatusReportOptions::default()),
        include_str!("../golden/status-overview.txt")
    );
}

#[test]
fn the_compressed_scope_by_age_is_byte_identical_to_the_kernel() {
    assert_eq!(
        golden_report(StatusReportOptions {
            scope: Some(StatusScope::Compressed),
            sort: StatusSort::Age,
            ..StatusReportOptions::default()
        }),
        include_str!("../golden/status-compressed.txt")
    );
}

#[test]
fn the_uncompressed_ranges_are_byte_identical_to_the_kernel() {
    assert_eq!(
        golden_report(StatusReportOptions {
            scope: Some(StatusScope::Uncompressed),
            ..StatusReportOptions::default()
        }),
        include_str!("../golden/status-uncompressed.txt")
    );
}

#[test]
fn the_message_listing_by_time_is_byte_identical_to_the_kernel() {
    assert_eq!(
        golden_report(StatusReportOptions {
            sort: StatusSort::Time,
            ..uncompressed_messages(None)
        }),
        include_str!("../golden/status-messages.txt")
    );
}
