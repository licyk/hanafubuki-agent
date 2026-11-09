// Derived from billion-context (https://github.com/ranxianglei/billion-context),
// tests of `kernel/tests` of acp-kernel 0.0.189 at commit `4be21b08e`.
// Copyright (c) 2026 ranxianglei. MIT License with an additional attribution
// term (NOTICE).
// Ports `ranges-consistency.test.ts`: the uncompressed view and the
// compressible ranges segment a session the same way.

use hanafubuki_agent_compression::{
    build_compressible_ranges, build_status_report, Block, Message, Role, State,
    StatusReportOptions, StatusScope, StatusSort, StatusView,
};

use crate::fixtures::{assign, config, state_for};

fn message(id: &str, text: &str, role: Role) -> Message {
    Message::text(id, role, text)
}

fn user(id: &str, text: &str) -> Message {
    message(id, text, Role::User)
}

fn assistant(id: &str, text: &str) -> Message {
    message(id, text, Role::Assistant)
}

/// A call without a call id.
fn tool_call(id: &str, tool: &str) -> Message {
    Message {
        tool_call_id: None,
        ..Message::tool_call(id, "", tool, format!("call {tool}"))
    }
}

/// `(start ref, end ref, count)` of each range line of an uncompressed
/// ranges report: `  m00001–m00009  (9 msgs, …` or `  m00010  (1 msgs, …`.
fn parse_ranges(report: &str) -> Vec<(String, String, usize)> {
    report
        .lines()
        .filter_map(|line| {
            let rest = line.strip_prefix("  m")?;
            let (refs, rest) = rest.split_once("  (")?;
            let (count, _) = rest.split_once(" msgs,")?;
            let (start, end) = match refs.split_once('–') {
                Some((start, end)) => (format!("m{start}"), end.to_owned()),
                None => (format!("m{refs}"), format!("m{refs}")),
            };
            Some((start, end, count.parse().ok()?))
        })
        .collect()
}

fn options(sort: StatusSort, limit: usize) -> StatusReportOptions<'static> {
    StatusReportOptions {
        scope: Some(StatusScope::Uncompressed),
        view: StatusView::Ranges,
        sort,
        limit,
        ..StatusReportOptions::default()
    }
}

fn uncompressed(
    state: &State,
    messages: &[Message],
    sort: StatusSort,
) -> Vec<(String, String, usize)> {
    parse_ranges(&build_status_report(state, messages, &options(sort, 30)))
}

fn recommended(state: &State, messages: &[Message]) -> Vec<(String, String, usize)> {
    build_compressible_ranges(messages, state, &config(), None, messages)
        .compressible
        .into_iter()
        .map(|range| (range.start_ref, range.end_ref, range.count))
        .collect()
}

fn span(start: &str, end: &str, count: usize) -> (String, String, usize) {
    (start.to_owned(), end.to_owned(), count)
}

#[test]
fn an_append_only_session_splits_the_same_in_both_views() {
    let messages = [
        user("u1", "question one"),
        assistant("a1", "answer one"),
        tool_call("c1", "bash"),
        message("r1", "output one", Role::Tool),
        user("u2", "question two"),
        assistant("a2", "answer two"),
        user("u3", "question three"),
        assistant("a3", "answer three"),
        tool_call("c3", "bash"),
        message("r3", "output three", Role::Tool),
        assistant("a4", "follow-up"),
    ];
    let state = state_for(&messages);
    let reported = uncompressed(&state, &messages, StatusSort::Time);
    assert_eq!(reported, recommended(&state, &messages));
    assert_eq!(
        reported,
        [span("m00001", "m00004", 4), span("m00005", "m00011", 7)]
    );
}

#[test]
fn a_coverage_gap_splits_both_views_at_the_same_place() {
    let messages: Vec<Message> = ["a", "b", "c", "d", "e"]
        .iter()
        .zip(["x", "y", "z", "w", "v"])
        .map(|(id, fill)| assistant(id, &fill.repeat(500)))
        .collect();
    let mut state = state_for(&messages);
    state.blocks.push(Block {
        block_id: "b0".to_owned(),
        effective_message_ids: vec!["c".to_owned()],
        ..Block::default()
    });
    let reported = uncompressed(&state, &messages, StatusSort::Time);
    assert_eq!(reported, recommended(&state, &messages));
    assert_eq!(
        reported,
        [span("m00001", "m00002", 2), span("m00004", "m00005", 2)]
    );
}

#[test]
fn holes_in_the_ref_map_do_not_fragment_ranges() {
    let a = assistant("a", &"x".repeat(2000));
    let e = assistant("e", &"v".repeat(2000));
    let state = state_for(&[
        a.clone(),
        user("b", "y"),
        user("c", "z"),
        user("d", "w"),
        e.clone(),
    ]);
    assert_eq!(
        uncompressed(&state, &[a, e], StatusSort::Size),
        [span("m00001", "m00005", 2)]
    );
}

#[test]
fn a_mid_array_summary_node_extends_the_range_without_a_descending_pair() {
    let a = assistant("a", &"x".repeat(2000));
    let d = assistant("d", &"w".repeat(2000));
    let e = assistant("e", &"v".repeat(2000));
    let summary = assistant("s", "Summary of the compressed span: did the work.");
    let mut state = state_for(&[
        a.clone(),
        user("b", "y"),
        user("c", "z"),
        d.clone(),
        e.clone(),
    ]);
    let view = [a, summary, d, e];
    assign(&view, &mut state);
    assert_eq!(state.message_refs.by_raw["s"], "m00006");
    assert_eq!(
        uncompressed(&state, &view, StatusSort::Size),
        [span("m00001", "m00005", 4)]
    );
}

#[test]
fn a_synthetic_node_with_a_ref_stays_visible_in_the_uncompressed_view() {
    let a = assistant("a", &"x".repeat(2000));
    let e = assistant("e", &"v".repeat(2000));
    let synthetic = assistant(
        "s",
        "[Compressed conversation section] earlier work summarized.",
    );
    let mut state = state_for(&[
        a.clone(),
        user("b", "y"),
        user("c", "z"),
        user("d", "w"),
        e.clone(),
    ]);
    let view = [a, synthetic, e];
    assign(&view, &mut state);
    assert_eq!(
        recommended(&state, &view),
        [span("m00001", "m00001", 1), span("m00005", "m00005", 1)]
    );
    assert_eq!(
        uncompressed(&state, &view, StatusSort::Size),
        [span("m00001", "m00005", 3)]
    );
}

fn display_session() -> (State, Vec<Message>) {
    let messages = vec![
        user("u1", &"q".repeat(10)),
        assistant("a1", &"a".repeat(10)),
        tool_call("c1", "bash"),
        assistant("x1", &"t".repeat(10)),
        user("u2", &"Q".repeat(100)),
        assistant("a2", &"A".repeat(100)),
        assistant("a3", &"B".repeat(100)),
        assistant("a4", &"C".repeat(100)),
        assistant("a5", &"D".repeat(100)),
        assistant("a6", &"E".repeat(100)),
        assistant("a7", &"F".repeat(100)),
        user("u3", &"q".repeat(10)),
        assistant("a8", &"a".repeat(10)),
    ];
    (state_for(&messages), messages)
}

#[test]
fn the_default_sort_is_by_size_with_a_header() {
    let (state, messages) = display_session();
    let report = build_status_report(&state, &messages, &options(StatusSort::Size, 30));
    assert!(report.contains("Sorted by size"));
    let at = |needle: &str| report.find(needle).unwrap();
    assert!(at("m00005–m00011") < at("m00001–m00004"));
    assert!(at("m00001–m00004") < at("m00012–m00013"));
}

#[test]
fn sorting_by_time_restores_the_chronological_order() {
    let (state, messages) = display_session();
    let report = build_status_report(&state, &messages, &options(StatusSort::Time, 30));
    assert!(report.contains("Sorted by time"));
    let at = |needle: &str| report.find(needle).unwrap();
    assert!(at("m00001–m00004") < at("m00005–m00011"));
    assert!(at("m00005–m00011") < at("m00012–m00013"));
}

#[test]
fn a_limit_truncates_and_reports_the_remaining_ranges() {
    let (state, messages) = display_session();
    let report = build_status_report(&state, &messages, &options(StatusSort::Size, 2));
    assert_eq!(parse_ranges(&report).len(), 2);
    assert!(report.contains("... and 1 more ranges"));
}
