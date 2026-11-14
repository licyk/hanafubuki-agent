//! A Mermaid diagram in a model's answer, end to end on a vt100 screen in
//! both presentations: drawn, and replayed by `/resume`; streamed a few
//! bytes at a time, its code shown in the tail and never committed until
//! its closer turns it into the diagram, with no committed row rewritten;
//! an interrupt inside the open fence leaving its code; a fallback's
//! notice; and `tui.rendering.mermaid` turned off in the settings center,
//! the next answer's fence shown as code.

use std::time::{Duration, Instant};

use hanafubuki_agent::test_support::assistant_events;
use ratatui::crossterm::event::KeyCode;

use super::streaming::{chunks, plain};
use super::{Harness, WIDTH};
use crate::agent_bridge::{TurnEnd, TurnUpdate};
use crate::history_cell::{AgentMarkdownCell, HistoryCell, LiveCell, StreamTailCell};
use crate::transcript_mode::TranscriptMode;

const ANSWER: &str = "The flow:\n\n```mermaid\nflowchart LR\n    A[Request] --> B[Reply]\n```\n\nA cycle:\n\n```mermaid\npie\n\"Cats\": 2\n```\n\nDone.\n";

const ROWS: [&str; 5] = [
    "• The flow:",
    "  ┌─────────┐  ┌───────┐",
    "  │ Request │  │ Reply │",
    "  This Mermaid diagram uses features the terminal renderer doesn't support.",
    "  \"Cats\": 2",
];

const PRESENTATIONS: [TranscriptMode; 2] = [TranscriptMode::Terminal, TranscriptMode::Owned];

fn lines(text: &str) -> Vec<String> {
    text.lines()
        .map(|line| line.trim_end().to_string())
        .collect()
}

fn assert_rows(text: &str, what: &str) {
    let lines = lines(text);
    for row in ROWS {
        assert!(
            lines.iter().any(|line| line == row),
            "{what}: no row {row:?} in\n{}",
            lines.join("\n")
        );
    }
    assert!(
        !lines
            .iter()
            .any(|line| line.contains("A[Request] --> B[Reply]")),
        "{what}: the diagram's code shows\n{}",
        lines.join("\n")
    );
}

#[test]
fn an_answers_diagram_renders_and_is_replayed_by_resume() {
    for mode in PRESENTATIONS {
        let mut harness = Harness::start_with(vec![assistant_events(ANSWER)], true, mode);
        harness.type_and_submit("Draw it");
        harness.pump_until_screen_shows("Done.");
        harness.pump_until_turn_ends();
        assert_rows(&harness.screen(), &format!("{mode:?} screen"));

        harness.type_and_submit("/new");
        harness.pump_until("no session", |harness| {
            harness.app.bridge.session_id().is_none()
        });
        harness.type_and_submit("/resume");
        harness.pump_until_screen_shows("Resume a previous session");
        harness.key(KeyCode::Enter);
        harness.pump_until("the resumed transcript", |harness| {
            harness.transcript().contains("│ Request │")
        });
        assert_rows(&harness.transcript(), &format!("{mode:?} resumed"));
    }
}

/// A few bytes a tick: while the fence is open its code shows in the tail
/// and none of it is committed; at the end the rows written are the
/// answer's, each once, with no repair.
#[test]
fn a_diagram_streams_without_rewriting_what_it_committed() {
    for mode in PRESENTATIONS {
        let mut harness = Harness::streaming(mode);
        harness.turn_update(TurnUpdate::Started);
        let start = Instant::now();
        let mut saw_code = false;
        for (tick, chunk) in chunks(ANSWER).into_iter().enumerate() {
            harness
                .app
                .chat_widget
                .handle_turn_update(TurnUpdate::MessageDelta(chunk.to_string()));
            harness
                .app
                .chat_widget
                .on_commit_tick_at(start + Duration::from_millis(8 * tick as u64));
            harness.settle_events();
            saw_code |= harness.screen().contains("A[Request] --> B[Reply]");
            for cell in &harness.app.transcript_cells {
                if cell.as_any().is::<LiveCell<StreamTailCell>>() {
                    continue;
                }
                assert!(
                    !plain(&cell.display_lines(WIDTH))
                        .iter()
                        .any(|row| row.contains("A[Request] -->")),
                    "{mode:?}: the open fence's code was committed"
                );
            }
        }
        assert!(saw_code, "{mode:?}: the open fence showed its code");
        harness.turn_update(TurnUpdate::MessageCompleted);
        harness.turn_update(TurnUpdate::Ended(TurnEnd::Completed));
        assert!(!harness.has_stream_tail(), "{mode:?}");
        let expected = plain(
            &AgentMarkdownCell::new(ANSWER.to_string(), "/project".to_string())
                .display_lines(WIDTH),
        );
        if mode.is_owned() {
            assert_rows(&harness.screen(), "Owned");
        } else {
            assert!(!harness.app.reflow.is_pending(), "no repair was needed");
            harness.app.write_tail_at_exit(&mut harness.tui).unwrap();
            let written = harness.written();
            let first = written
                .iter()
                .position(|row| row.starts_with("• The flow:"))
                .expect("the answer was written");
            assert_eq!(written[first..first + expected.len()], expected[..]);
        }
    }
}

/// An answer cut inside an open fence keeps its code, without a notice,
/// laid out again at each width with the same text.
#[test]
fn an_interrupt_inside_an_open_fence_leaves_its_code() {
    for mode in PRESENTATIONS {
        for ending in ["\n", ""] {
            let mut harness = Harness::streaming(mode);
            harness.turn_update(TurnUpdate::Started);
            let source = format!("Intro.\n\n```mermaid\nflowchart LR\nA --> B{ending}");
            for delta in ["Intro.\n\n", &source[8..]] {
                harness.turn_update(TurnUpdate::MessageDelta(delta.to_string()));
                harness.app.chat_widget.on_commit_tick();
                harness.settle_events();
            }
            harness.turn_update(TurnUpdate::Ended(TurnEnd::Interrupted));
            assert!(!harness.has_stream_tail(), "{mode:?}");
            for width in [80u16, 20, 80] {
                let text = harness
                    .app
                    .transcript_cells
                    .iter()
                    .flat_map(|cell| cell.display_lines(width))
                    .map(|line| line.to_string())
                    .collect::<Vec<_>>()
                    .join("\n");
                assert!(text.contains("A --> B"), "{mode:?} {width}:\n{text}");
                assert!(!text.contains("Mermaid diagram"), "{text}");
                assert!(!text.contains('┌'), "{text}");
                assert!(text.contains("Conversation interrupted"), "{text}");
            }
        }
    }
}

#[test]
fn diagrams_turned_off_show_the_next_answers_fence_as_code() {
    let mut harness = Harness::owned(vec![assistant_events(ANSWER)]);
    harness.type_and_submit("/config interface");
    harness.pump_until_screen_shows("Terminal interface");
    harness.select_setting("Mermaid diagrams");
    harness.key(KeyCode::Enter);
    harness.pump_until_screen_shows("Saved");
    assert!(harness.config_file().contains("mermaid = false"));
    harness.key(KeyCode::Esc);
    harness.pump_until("the center to close", |harness| {
        !harness.screen().contains("Changes save automatically")
            && !harness.screen().contains("Saved")
    });
    harness.type_and_submit("Draw it");
    harness.pump_until_screen_shows("Done.");
    harness.pump_until_turn_ends();
    let screen = lines(&harness.screen());
    for row in [
        "  flowchart LR",
        "      A[Request] --> B[Reply]",
        "  pie",
        "  \"Cats\": 2",
    ] {
        assert!(
            screen.iter().any(|line| line == row),
            "no row {row:?} in\n{}",
            screen.join("\n")
        );
    }
    assert!(!screen.iter().any(|line| line.contains("Mermaid diagram")));
    assert!(!screen.iter().any(|line| line.contains('┌')));
}
