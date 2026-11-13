//! A call keeps its shape while it changes: a row stands under its header
//! from its start until what it shows arrives, and a running command's
//! clock never moves where its header wraps.

use std::time::Duration;

use hanafubuki_agent::{AgentToolCallDisplay, AgentToolDisplay};
use unicode_width::UnicodeWidthStr;

use super::{command, no_display, render};
use crate::history_cell::tool::{new_tool_call, ToolCallCell};

fn call(name: &str, subject: &str) -> ToolCallCell {
    new_tool_call(
        "call-1".to_string(),
        name.to_string(),
        Some(subject.to_string()),
    )
}

fn displayed(display: AgentToolDisplay) -> AgentToolCallDisplay {
    AgentToolCallDisplay {
        subject: None,
        display: Some(display),
    }
}

/// The rows under the header.
fn rows(cell: &ToolCallCell) -> Vec<String> {
    render(cell, 60)
        .lines()
        .skip(1)
        .map(str::to_string)
        .collect()
}

#[test]
fn a_command_reserves_its_first_row_from_its_start_to_its_display() {
    let mut cell = call("exec_command", "git add .");
    assert_eq!(rows(&cell), ["  └ (running…)"]);
    cell.complete("call-1", false);
    assert_eq!(rows(&cell), ["  └ …"]);
    cell.show("call-1", displayed(command("git add .", "", Some(0))));
    assert_eq!(rows(&cell), ["  └ (no output)"]);

    // Its first line takes the reserved row's place.
    let mut cell = call("exec_command", "make");
    cell.push_output("call-1", "built\n", false);
    assert_eq!(rows(&cell), ["  └ built"]);
    cell.complete("call-1", false);
    cell.show("call-1", displayed(command("make", "built\n", Some(0))));
    assert_eq!(rows(&cell), ["  └ built"]);
}

#[test]
fn another_tool_reserves_a_row_its_result_replaces() {
    let mut cell = call("mcp_tool", "query");
    let running = render(&cell, 60);
    assert!(
        running.ends_with("Running mcp_tool: query\n  └ …"),
        "{running}"
    );
    cell.complete("call-1", false);
    assert_eq!(rows(&cell), ["  └ …"]);
    cell.show(
        "call-1",
        displayed(AgentToolDisplay::Text {
            text: "one\ntwo\n".to_string(),
        }),
    );
    assert_eq!(rows(&cell), ["  └ one", "    two"]);

    // Without a display, the header alone once that is known.
    let mut cell = call("mcp_tool", "query");
    cell.complete("call-1", false);
    cell.show("call-1", no_display());
    assert_eq!(render(&cell, 60), "• Called mcp_tool: query");
    // A call the turn's end overtook will have none either.
    let mut cell = call("mcp_tool", "query");
    cell.end_running();
    assert_eq!(render(&cell, 60), "• Failed mcp_tool: query");
}

#[test]
fn a_running_commands_clock_never_changes_where_its_header_wraps() {
    let mut cell = call(
        "exec_command",
        "cargo test --workspace --all-features -- --nocapture",
    );
    let header = |cell: &ToolCallCell, width: u16| -> Vec<String> {
        let rendered = render(cell, width);
        let rows: Vec<&str> = rendered.lines().collect();
        // Each row fits, so the transcript wraps none of them again.
        assert!(
            rows.iter().all(|row| row.width() <= usize::from(width)),
            "{rendered}"
        );
        let header = rows.len() - 1;
        // The rows up to the clock, which is the last header row's end,
        // after the spinner.
        let mut kept: Vec<String> = rows[..header].iter().map(|row| row.to_string()).collect();
        kept[0] = kept[0].chars().skip(1).collect();
        let last = kept.last_mut().unwrap();
        if let Some(clock) = last.rfind(" · ") {
            last.truncate(clock);
            last.push_str(" (clock)");
        }
        kept
    };
    for width in 20..=90 {
        cell.started_ago("call-1", Duration::from_millis(900));
        let first = header(&cell, width);
        for ago in [9_950, 10_050, 59_950, 62_000, 125_000] {
            cell.started_ago("call-1", Duration::from_millis(ago));
            assert_eq!(header(&cell, width), first, "at {width} columns, {ago} ms");
        }
    }
}

#[test]
fn a_header_without_room_for_the_clocks_widest_shows_no_clock() {
    let mut cell = call("exec_command", "cargo build");
    cell.started_ago("call-1", Duration::from_millis(3_400));
    // `• Running cargo build` is 21 columns: with the clock's 14, 35.
    let rendered = render(&cell, 35);
    assert!(rendered.contains("cargo build · 3.4s"), "{rendered}");
    let rendered = render(&cell, 34);
    assert!(
        rendered.ends_with("Running cargo build\n  └ (running…)"),
        "{rendered}"
    );
}
