//! A tool call's entry frame by frame on a vt100 screen, in both
//! presentations: from its start, through its output, its completion and
//! its display, no frame is without it or the status row; where its shape
//! holds, no row above it moves; where it grows, by at most its control
//! and two rows, the rows above move by that much while following, and
//! not at all while reading.

use std::sync::Arc;
use std::time::Duration;

use hanafubuki_agent::{AgentToolCallDisplay, AgentToolDisplay};
use ratatui::crossterm::event::{KeyModifiers, MouseEvent, MouseEventKind};

use super::{AppEvent, Harness};
use crate::agent_bridge::TurnUpdate;
use crate::history_cell::{new_tool_call, LiveCell, ToolCallCell};
use crate::transcript_mode::TranscriptMode;

/// The rows a change may add to an entry: its control and two more.
const MOST_GROWTH: usize = 3;

/// One frame: the screen's rows, the entry's header row and its rows.
struct Frame {
    rows: Vec<String>,
    header: usize,
    height: usize,
}

impl Harness {
    /// A turn running over a conversation that fills the screen.
    fn busy(mode: TranscriptMode) -> Self {
        let mut harness = Harness::start_with(Vec::new(), true, mode);
        harness.pump_until_screen_shows("fixture-model");
        harness
            .app
            .chat_widget
            .handle_turn_update(TurnUpdate::Started);
        for n in 0..30 {
            harness.note(&format!("note {n:02}"));
        }
        harness
    }

    /// Draws a frame and finds the entry whose header names `subject`.
    fn frame(&mut self, subject: &str) -> Frame {
        self.app.draw(&mut self.tui).unwrap();
        let rows = self.rows();
        let screen = rows.join("\n");
        let header = rows
            .iter()
            .position(|row| row.contains(subject))
            .unwrap_or_else(|| panic!("a frame without the entry:\n{screen}"));
        assert!(
            rows.iter().any(|row| row.contains("Working")),
            "a frame without the status row:\n{screen}"
        );
        // Its rows under the header, up to what follows it or the bottom
        // of the conversation's area.
        let height = 1 + rows[header + 1..]
            .iter()
            .take_while(|row| row.starts_with("  └ ") || row.starts_with("    "))
            .count();
        Frame {
            rows,
            header,
            height,
        }
    }
}

fn call(name: &str, subject: &str) -> Arc<LiveCell<ToolCallCell>> {
    LiveCell::new(new_tool_call(
        "call-1".to_string(),
        name.to_string(),
        Some(subject.to_string()),
    ))
}

fn displayed(display: AgentToolDisplay) -> AgentToolCallDisplay {
    AgentToolCallDisplay {
        subject: None,
        display: Some(display),
    }
}

fn command_display(command: &str, output: &str) -> AgentToolDisplay {
    AgentToolDisplay::Command {
        command: command.to_string(),
        cwd: None,
        output: output.to_string(),
        exit_code: Some(0),
        duration_millis: Some(61_000),
        truncated: false,
    }
}

type Step = Box<dyn Fn(&mut ToolCallCell)>;

/// `next` after `previous`: the entry grew by at most [`MOST_GROWTH`]
/// rows, and the rows above it moved up by that much, or stayed where
/// they were when `following` is false.
fn assert_moved_by_growth(previous: &Frame, next: &Frame, following: bool, step: usize) {
    let screen = next.rows.join("\n");
    let growth = next
        .height
        .checked_sub(previous.height)
        .unwrap_or_else(|| panic!("step {step}: the entry shrank:\n{screen}"));
    assert!(
        growth <= MOST_GROWTH,
        "step {step}: grew {growth}:\n{screen}"
    );
    let moved = if following { growth } else { 0 };
    assert_eq!(
        previous.header - next.header,
        moved,
        "step {step}:\n{screen}"
    );
    assert_eq!(
        next.rows[..next.header],
        previous.rows[moved..previous.header],
        "step {step}: a row above the entry moved:\n{screen}"
    );
}

#[test]
fn a_running_command_keeps_its_shape_through_its_clock_output_completion_and_display() {
    // `• Running ` and the command fill 93 of the 100 columns: a clock of
    // 9.9s would fit after it, one of 10.0s would wrap it.
    let command = format!("probe-{}", "x".repeat(77));
    let steps: Vec<Step> = vec![
        Box::new(|cell| cell.started_ago("call-1", Duration::from_millis(9_950))),
        Box::new(|cell| cell.started_ago("call-1", Duration::from_millis(10_050))),
        Box::new(|cell| cell.started_ago("call-1", Duration::from_millis(61_000))),
        Box::new(|cell| cell.push_output("call-1", "built\n", false)),
        Box::new(|cell| cell.complete("call-1", false)),
        Box::new({
            let command = command.clone();
            move |cell| cell.show("call-1", displayed(command_display(&command, "built\n")))
        }),
    ];
    for mode in [TranscriptMode::Owned, TranscriptMode::Terminal] {
        let mut harness = Harness::busy(mode);
        let tool = call("exec_command", &command);
        tool.update(|cell| cell.started_ago("call-1", Duration::from_millis(900)));
        harness.send(AppEvent::InsertLiveEntry(tool.clone()));
        let first = harness.frame("probe-");
        assert_eq!(first.height, 2, "{}", first.rows.join("\n"));
        let viewport = harness.tui.terminal.viewport_area;
        for (index, step) in steps.iter().enumerate() {
            tool.update(|cell| step(cell));
            let frame = harness.frame("probe-");
            let screen = frame.rows.join("\n");
            assert_eq!(
                (frame.header, frame.height),
                (first.header, first.height),
                "step {index}:\n{screen}"
            );
            assert_eq!(
                frame.rows[..frame.header],
                first.rows[..first.header],
                "step {index}: a row above the entry moved:\n{screen}"
            );
            assert_eq!(harness.tui.terminal.viewport_area, viewport);
        }
        harness.send(AppEvent::LiveEntryCompleted(tool));
        let last = harness.frame("probe-");
        assert_eq!(last.rows[..last.header], first.rows[..first.header]);
        assert_eq!(last.rows[last.header + 1], "  └ built");
    }
}

#[test]
fn an_entry_grows_by_at_most_its_control_and_two_rows_and_only_the_rows_above_follow() {
    let lines = |count: usize| -> String { (1..=count).map(|n| format!("out-{n}\n")).collect() };
    let command_steps: Vec<Step> = vec![
        Box::new(move |cell| cell.push_output("call-1", &lines(3), false)),
        Box::new(|cell| cell.push_output("call-1", "out-4\nout-5\n", false)),
        Box::new(|cell| cell.complete("call-1", false)),
        Box::new(move |cell| {
            cell.show(
                "call-1",
                displayed(command_display("probe-make", &lines(5))),
            )
        }),
    ];
    let tool_steps: Vec<Step> = vec![
        Box::new(|cell| cell.complete("call-1", false)),
        Box::new(move |cell| {
            cell.show(
                "call-1",
                displayed(AgentToolDisplay::Text { text: lines(5) }),
            )
        }),
    ];
    let cases = [
        ("exec_command", "probe-make", &command_steps),
        ("mcp_tool", "probe-query", &tool_steps),
    ];
    for (name, subject, steps) in cases {
        for mode in [TranscriptMode::Owned, TranscriptMode::Terminal] {
            let mut harness = Harness::busy(mode);
            let tool = call(name, subject);
            harness.send(AppEvent::InsertLiveEntry(tool.clone()));
            let mut previous = harness.frame(subject);
            assert_eq!(previous.height, 2, "{}", previous.rows.join("\n"));
            for (index, step) in steps.iter().enumerate() {
                tool.update(|cell| step(cell));
                let frame = harness.frame(subject);
                assert_moved_by_growth(&previous, &frame, true, index);
                previous = frame;
            }
        }

        // A reader of the conversation sees nothing above the entry move.
        let mut harness = Harness::busy(TranscriptMode::Owned);
        let tool = call(name, subject);
        harness.send(AppEvent::InsertLiveEntry(tool.clone()));
        for n in 0..3 {
            harness.note(&format!("after {n}"));
        }
        harness.send(AppEvent::Mouse(MouseEvent {
            kind: MouseEventKind::ScrollUp,
            column: 0,
            row: 0,
            modifiers: KeyModifiers::NONE,
        }));
        assert!(!harness.app.transcript_view.is_following());
        let mut previous = harness.frame(subject);
        for (index, step) in steps.iter().enumerate() {
            tool.update(|cell| step(cell));
            let frame = harness.frame(subject);
            assert_moved_by_growth(&previous, &frame, false, index);
            previous = frame;
        }
    }
}
