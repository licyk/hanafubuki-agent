use std::time::{Duration, Instant};

use super::*;

const THOUGHT: &str = "**Reading the manifest**\n\nThe crate needs `serde` and a `tokio` runtime:\n\n- check the features\n- then the lockfile";

fn render(lines: &[Line<'static>]) -> String {
    lines
        .iter()
        .map(|line| {
            line.spans
                .iter()
                .map(|span| span.content.as_ref())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// A live cell that started five seconds ago.
fn live(text: &str) -> ThinkingCell {
    let mut cell = ThinkingCell::live(AgentReasoningId::new(7));
    cell.state = ThinkingState::Live {
        started: Instant::now() - Duration::from_secs(5),
    };
    cell.push(text);
    cell
}

fn replayed(text: &str, duration_millis: u64, truncated: bool) -> ThinkingCell {
    ThinkingCell::replayed(42, text.to_string(), duration_millis, truncated)
}

#[test]
fn a_live_cell_counts_up_in_tenths_and_is_its_header_collapsed() {
    let cell = live(THOUGHT);
    let collapsed = render(&cell.display_lines(40));
    // Five seconds and the tenths the test took.
    assert!(collapsed.starts_with("• Thinking 5."), "{collapsed}");
    assert!(collapsed.ends_with('s') && !collapsed.contains('\n'));
}

#[test]
fn a_live_expanded_cell_shows_the_streamed_markdown() {
    let lines = live(THOUGHT).expanded_lines(40);
    assert!(render(&lines[..1]).starts_with("• Thinking 5."));
    insta::assert_snapshot!(render(&lines[1..]));
}

#[test]
fn a_completed_collapsed_cell_says_how_long_it_thought() {
    insta::assert_snapshot!(render(&replayed(THOUGHT, 12_400, false).display_lines(40)));
}

#[test]
fn a_completed_expanded_cell_keeps_its_text() {
    insta::assert_snapshot!(render(&replayed(THOUGHT, 75_000, true).expanded_lines(40)));
}

#[test]
fn an_item_without_text_has_no_control_and_no_id() {
    let cell = replayed("", 2_000, false);
    insta::assert_snapshot!(render(&cell.expanded_lines(40)));
    assert_eq!(cell.transcript_lines(40).len(), 1);
    assert!(cell.activity_ids().is_empty());
    assert_eq!(cell.activity_disclosure(40), None);
    let mut live = live("");
    assert!(live.activity_ids().is_empty());
    live.push("First words.");
    assert_eq!(live.activity_ids(), vec!["thinking:7".to_string()]);
    assert_eq!(
        live.activity_disclosure(40),
        Some(ActivityDisclosure::Generic)
    );
}

#[test]
fn a_live_id_names_the_reasoning_item_and_a_replayed_one_its_history_item() {
    assert_eq!(
        replayed(THOUGHT, 1, false).activity_ids(),
        vec!["thinking:h42".to_string()]
    );
    let mut cell = live(THOUGHT);
    let id = cell.activity_ids();
    cell.complete(Some(1_000));
    assert_eq!(cell.activity_ids(), id, "the same entry live and completed");
}

#[test]
fn the_transcript_shows_a_collapsed_cells_text() {
    let cell = replayed(THOUGHT, 12_400, false);
    assert_eq!(cell.display_lines(40).len(), 1);
    let transcript = render(&cell.transcript_lines(40));
    assert!(
        transcript.starts_with("• Thought · 12.4s\n"),
        "{transcript}"
    );
    assert!(
        transcript.contains("  Reading the manifest"),
        "{transcript}"
    );
    assert!(transcript.contains("then the lockfile"), "{transcript}");
}

#[test]
fn a_live_cell_completes_with_its_duration_or_the_time_so_far() {
    let mut reported = live(THOUGHT);
    reported.complete(Some(20_790));
    assert_eq!(render(&reported.display_lines(40)), "• Thought · 20.7s");
    let mut cut_off = live("");
    cut_off.complete(None);
    let header = render(&cut_off.display_lines(40));
    assert!(header.starts_with("• Thought · 5."), "{header}");
    assert_eq!(
        render(&replayed("", 65_000, true).display_lines(40)),
        "• Thought · 1m 05s (truncated)"
    );
}

#[test]
fn the_live_clock_ticks_every_tenth_of_a_second() {
    let mut cell = live("");
    let started = Instant::now() - Duration::from_millis(1_250);
    cell.state = ThinkingState::Live { started };
    let tick = cell.transcript_animation_tick().unwrap();
    assert!((12..15).contains(&tick), "{tick}");
    cell.complete(Some(1_000));
    assert_eq!(cell.transcript_animation_tick(), None);
}
