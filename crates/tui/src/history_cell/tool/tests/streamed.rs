//! A command's output as it streams: the last lines under the running
//! header, the stream expanded, and the finished display taking over.

use std::time::{Duration, Instant};

use hanafubuki_agent::AgentToolCallDisplay;

use super::{command, numbered, render, text};
use crate::history_cell::tool::{new_tool_call, ToolCallCell};
use crate::history_cell::{ActivityDisclosure, HistoryCell};

fn running(command: &str) -> ToolCallCell {
    new_tool_call(
        "call-1".to_string(),
        "exec_command".to_string(),
        Some(command.to_string()),
    )
}

/// The rows under the header, which counts the time.
fn rows(rendered: &str) -> Vec<&str> {
    rendered.lines().skip(1).collect()
}

#[test]
fn a_running_command_shows_the_last_lines_it_printed_and_expands_to_the_stream() {
    let mut cell = running("cargo test");
    assert_eq!(rows(&render(&cell, 60)), ["  └ (running…)"]);
    cell.push_output("call-1", &numbered(5), false);
    cell.push_output("call-1", "compiling\tfoo", false);
    let collapsed = render(&cell, 60);
    assert!(collapsed.contains("Running cargo test · "), "{collapsed}");
    assert_eq!(
        rows(&collapsed),
        ["  └ line 4", "    line 5", "    compiling   foo"]
    );
    assert_eq!(
        cell.activity_disclosure(60),
        Some(ActivityDisclosure::OutputLines(3))
    );
    let expanded = text(&cell.transcript_lines(60));
    assert_eq!(
        rows(&expanded),
        [
            "  └ $ cargo test",
            "    line 1",
            "    line 2",
            "    line 3",
            "    line 4",
            "    line 5",
            "    compiling   foo",
        ]
    );
    // The unfinished line is redrawn in place.
    cell.push_output("call-1", "\r\x1b[2Klinking", false);
    assert_eq!(
        rows(&render(&cell, 60)),
        ["  └ line 4", "    line 5", "    linking"]
    );
}

#[test]
fn a_finished_command_keeps_its_rows_until_its_display_takes_over() {
    let mut cell = running("cargo test");
    cell.push_output("call-1", &numbered(4), false);
    cell.complete("call-1", false);
    // Marked finished, it shows what it printed until its display arrives.
    assert_eq!(
        render(&cell, 60),
        "• Ran cargo test\n  └ line 2\n    line 3\n    line 4"
    );
    assert_eq!(cell.transcript_animation_tick(), None);
    cell.show(
        "call-1",
        AgentToolCallDisplay {
            subject: None,
            display: Some(command("cargo test", &numbered(4), Some(0))),
        },
    );
    // The same rows, now the display's.
    assert_eq!(
        render(&cell, 60),
        "• Ran cargo test\n  └ line 2\n    line 3\n    line 4"
    );
    assert!(cell.calls[0].live.is_none());

    // When no display can be read, what it printed stays.
    let mut cell = running("make");
    cell.push_output("call-1", "built\n", false);
    cell.complete("call-1", true);
    cell.show(
        "call-1",
        AgentToolCallDisplay {
            subject: None,
            display: None,
        },
    );
    assert_eq!(render(&cell, 60), "• Ran make\n  └ built");
}

#[test]
fn output_for_another_call_changes_nothing() {
    let mut cell = running("sleep 1");
    cell.push_output("call-2", "other\n", false);
    assert_eq!(rows(&render(&cell, 60)), ["  └ (running…)"]);
}

#[test]
fn a_running_command_counts_its_time_in_tenths() {
    let mut cell = running("sleep 9");
    cell.calls[0].start_time = Instant::now() - Duration::from_millis(4_250);
    assert!(
        render(&cell, 60).contains("Running sleep 9 · 4."),
        "{}",
        render(&cell, 60)
    );
    // The clock ticks every tenth; another tool's spinner every 600 ms.
    assert_eq!(
        cell.transcript_animation_tick().map(|tick| tick / 10),
        Some(4)
    );
}
