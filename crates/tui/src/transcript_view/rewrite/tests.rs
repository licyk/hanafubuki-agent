// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! Codex's capped resize reflow tests (`app/resize_reflow_tests.rs`), on the
//! rows a rewrite of the scrollback writes; and an entry written again as
//! the tail writes it.

use super::*;
use crate::transcript_view::tests::fixtures::{cell, tool_call};

fn cells(count: usize) -> Vec<Arc<dyn HistoryCell>> {
    (0..count)
        .map(|index| cell(format!("cell {index}")))
        .collect()
}

fn rewrite(cells: &[Arc<dyn HistoryCell>], width: u16, cap: usize) -> String {
    let mut view = TranscriptView::default();
    view.scrollback_lines(cells, width, cap)
        .iter()
        .map(|line| line.line.to_string().trim_end().to_owned())
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn a_rewrite_within_its_cap_writes_every_entry_separated() {
    assert_eq!(rewrite(&cells(3), 80, 32), "cell 0\n\ncell 1\n\ncell 2");
}

#[test]
fn capped_rewrite_keeps_the_latest_rows_under_a_notice() {
    let rewritten = rewrite(&cells(12), 80, 8);
    assert_eq!(rewritten.lines().count(), 8);
    insta::assert_snapshot!(rewritten, @r"
    Earlier messages are available — press ctrl + t to view the full transcript
    cell 8

    cell 9

    cell 10

    cell 11
    ");
}

#[test]
fn capped_rewrite_counts_the_wrapped_notice_rows() {
    let rewritten = rewrite(&cells(12), 28, 8);
    assert_eq!(rewritten.lines().count(), 8);
    insta::assert_snapshot!(rewritten, @r"
    Earlier messages are
    available — press ctrl + t
    to view the full transcript
    cell 9

    cell 10

    cell 11
    ");
}

#[test]
fn a_one_row_cap_keeps_the_conversation_instead_of_the_notice() {
    assert_eq!(rewrite(&cells(2), 80, 1), "cell 1");
}

#[test]
fn a_rewrite_writes_an_entry_as_the_tail_writes_it() {
    let cells = vec![cell("note"), tool_call("call-1", 20)];
    let mut view = TranscriptView::default();
    let collapsed = view.scrollback_lines(&cells, 40, 1_000);
    view.measure_tail(&cells, 40, false);
    let written = (0..cells.len())
        .flat_map(|index| view.tail_lines(&cells, index, false))
        .collect::<Vec<_>>();
    assert_eq!(collapsed, written);
}
