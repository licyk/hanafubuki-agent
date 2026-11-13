// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! A resize while a stream arrives: what it committed stays, what it
//! queued and its tail are laid out again at the new width, and nothing
//! it received is lost or shown twice.

use super::{finalized_cell, full_render, lines_to_plain_strings, stream_controller};
use crate::streaming::controller::TestStream;

fn finalized_strings(ctrl: &mut super::StreamController) -> Vec<String> {
    let finalized = ctrl.finalize();
    finalized_cell(&finalized)
        .map(|cell| lines_to_plain_strings(&cell.display_lines(u16::MAX)))
        .unwrap_or_default()
        .into_iter()
        .map(|line| line.chars().skip(2).collect())
        .collect()
}

#[test]
fn controller_set_width_rebuilds_queued_lines() {
    let mut ctrl = stream_controller(Some(120));
    assert!(ctrl.push("This is a long line that should wrap into multiple rows when resized.\n"));
    assert_eq!(ctrl.queued_lines(), 1);
    ctrl.set_width(Some(24));
    let (cell, idle) = ctrl.on_commit_tick_batch(usize::MAX);
    let rendered = lines_to_plain_strings(&cell.expect("rows").display_lines(u16::MAX));
    assert!(idle);
    assert!(rendered.len() > 1, "{rendered:?}");
}

#[test]
fn controller_set_width_no_duplicate_after_emit() {
    let mut ctrl = stream_controller(Some(120));
    ctrl.push(
        "This is a long line that definitely wraps when the terminal shrinks to 24 columns.\n",
    );
    let (cell, _) = ctrl.on_commit_tick_batch(usize::MAX);
    assert!(cell.is_some());
    ctrl.set_width(Some(24));
    assert_eq!(
        ctrl.queued_lines(),
        0,
        "committed rows are not queued again"
    );
}

#[test]
fn controller_set_width_partial_drain_keeps_pending_queue() {
    let mut ctrl = stream_controller(Some(40));
    ctrl.push("AAAA BBBB CCCC DDDD EEEE FFFF GGGG HHHH IIII JJJJ\n");
    ctrl.push("second line\n");
    let (cell, idle) = ctrl.on_commit_tick();
    assert!(cell.is_some());
    assert!(!idle);
    ctrl.set_width(Some(20));
    assert!(ctrl.queued_lines() > 0, "resize must keep what was queued");
    let mut drained = Vec::new();
    for _ in 0..64 {
        let (cell, idle) = ctrl.on_commit_tick();
        if let Some(cell) = cell {
            drained.extend(lines_to_plain_strings(&cell.display_lines(u16::MAX)));
        }
        if idle {
            break;
        }
    }
    assert!(
        drained.iter().any(|line| line.contains("second line")),
        "{drained:?}"
    );
}

#[test]
fn controller_set_width_preserves_in_flight_tail() {
    let mut ctrl = stream_controller(Some(80));
    ctrl.push("tail without newline");
    ctrl.set_width(Some(24));
    assert_eq!(finalized_strings(&mut ctrl), vec!["tail without newline"]);
}

#[test]
fn controller_set_width_after_first_line_emit_does_not_requeue_first_line() {
    let mut ctrl = stream_controller(Some(120));
    ctrl.push("FIRSTTOKEN contains enough words to wrap once the width is reduced dramatically.\n");
    ctrl.push("second line remains pending\n");
    let (first, _) = ctrl.on_commit_tick();
    assert!(first.is_some());
    ctrl.set_width(Some(20));
    let remaining = finalized_strings(&mut ctrl);
    assert!(
        !remaining.iter().any(|line| line.contains("FIRSTTOKEN")),
        "{remaining:?}"
    );
    assert!(
        remaining.iter().any(|line| line.contains("second line")),
        "{remaining:?}"
    );
}

#[test]
fn controller_set_width_partial_wrapped_emit_keeps_wrapped_remainder() {
    let mut ctrl = stream_controller(Some(18));
    ctrl.push("alpha beta gamma delta epsilon zeta eta theta iota kappa lambda mu\n");
    let (first, idle) = ctrl.on_commit_tick();
    assert!(first.is_some());
    assert!(!idle);
    ctrl.set_width(Some(80));
    let joined = finalized_strings(&mut ctrl).join(" ");
    assert!(
        joined.contains("kappa") || joined.contains("lambda") || joined.contains("mu"),
        "{joined:?}"
    );
}

#[test]
fn a_table_tail_is_laid_out_again_at_each_width() {
    let mut stream = TestStream::new(Some(96), "/repo");
    let source = "| # | Feature | Details | Link |\n| --- | --- | --- | --- |\n| 1 | RESIZE_REPRO_SENTINEL | long wrapped content that should be reflowed | https://example.com/resize |\n";
    stream.push(source);
    for width in [48, 104, 56] {
        stream.set_width(Some(width));
        assert_eq!(
            lines_to_plain_strings(&stream.tail()),
            full_render(source, Some(width)),
            "at {width}"
        );
    }
}

#[test]
fn a_held_list_marker_survives_a_resize() {
    let mut stream = TestStream::new(Some(10), "/repo");
    stream.push("Long prose with enough words to wrap.\n\n");
    assert!(stream.commit().len() > 1);
    stream.push("8.\n");
    stream.set_width(Some(40));
    let remaining = lines_to_plain_strings(&stream.finalize());
    assert_eq!(remaining.last().map(String::as_str), Some("8. "));
}

/// A resize asks for the stream's repair: its committed rows are of the
/// old width.
#[test]
fn a_resize_after_rows_were_committed_asks_for_a_repair() {
    let mut stream = TestStream::new(Some(60), "/repo");
    stream.push("A paragraph long enough to wrap at the narrower width.\n\n");
    assert!(!stream.commit().is_empty());
    stream.push("| A | B |\n|---|---|\n| 1 | 2 |\n");
    stream.set_width(Some(20));
    let (_, diverged) = stream.finalize_checked();
    assert!(diverged);
}
