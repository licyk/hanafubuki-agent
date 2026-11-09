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
fn live(text: &str, expanded: bool) -> ThinkingCell {
    let mut cell = ThinkingCell::live(expanded);
    cell.state = ThinkingState::Live {
        started: Instant::now() - Duration::from_secs(5),
    };
    cell.push(text);
    cell
}

#[test]
fn a_live_collapsed_cell_counts_up_under_its_header() {
    insta::assert_snapshot!(render(&live(THOUGHT, false).display_lines(40)));
}

#[test]
fn a_live_expanded_cell_shows_the_streamed_markdown() {
    insta::assert_snapshot!(render(&live(THOUGHT, true).display_lines(40)));
}

#[test]
fn a_completed_collapsed_cell_says_how_long_it_thought() {
    let cell = ThinkingCell::completed(THOUGHT.to_string(), 12_400, false, false);
    insta::assert_snapshot!(render(&cell.display_lines(40)));
}

#[test]
fn a_completed_expanded_cell_keeps_its_text() {
    let cell = ThinkingCell::completed(THOUGHT.to_string(), 75_000, true, true);
    insta::assert_snapshot!(render(&cell.display_lines(40)));
}

#[test]
fn an_item_without_text_shows_only_its_header() {
    let cell = ThinkingCell::completed(String::new(), 2_000, false, true);
    insta::assert_snapshot!(render(&cell.display_lines(40)));
    assert_eq!(cell.transcript_lines(40).len(), 1);
}

#[test]
fn the_transcript_shows_a_collapsed_cells_text() {
    let cell = ThinkingCell::completed(THOUGHT.to_string(), 12_400, false, false);
    assert_eq!(cell.display_lines(40).len(), 1);
    let transcript = render(&cell.transcript_lines(40));
    assert!(
        transcript.starts_with("• Thought for 12s\n"),
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
    let mut reported = live(THOUGHT, false);
    reported.complete(Some(3_000));
    assert_eq!(render(&reported.display_lines(40)), "• Thought for 3s");
    let mut cut_off = live("", false);
    cut_off.complete(None);
    assert_eq!(render(&cut_off.display_lines(40)), "• Thought for 5s");
}
