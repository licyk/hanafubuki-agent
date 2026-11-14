//! Math in a model's answer, end to end on a vt100 screen in both
//! presentations: inline and display formulas in Unicode notation, prices
//! and shell variables beside them as written; replayed by `/resume`;
//! streamed a few bytes at a time without a committed row rewritten, the
//! open display shown as its source meanwhile; an interrupt inside a
//! display leaving its source; and `tui.rendering.math` turned off in the
//! settings center, the next answer's formulas shown as written.

use std::time::{Duration, Instant};

use hanafubuki_agent::test_support::assistant_events;
use ratatui::crossterm::event::KeyCode;

use super::streaming::{chunks, plain};
use super::{Harness, WIDTH};
use crate::agent_bridge::{TurnEnd, TurnUpdate};
use crate::history_cell::{AgentMarkdownCell, HistoryCell, LiveCell, StreamTailCell};
use crate::transcript_mode::TranscriptMode;

const ANSWER: &str = "The spectrum is $\\lambda_1 \\leq \\lambda_2$ on $\\mathbb{R}^n$; it costs $5 and $10.\n\n$$\nx=\\frac{-b\\pm\\sqrt{b^2-4ac}}{2a}\n$$\n\n$$A=\\begin{pmatrix}1&2\\\\3&4\\end{pmatrix}$$\n\nShell: $HOME/$USER and `$x$`.\n";

const ROWS: [&str; 7] = [
    "• The spectrum is λ₁ ≤ λ₂ on ℝⁿ; it costs $5 and $10.",
    "    -b±√(b²-4ac)",
    "  x=────────────",
    "         2a",
    "  A=⎛ 1  2 ⎞",
    "    ⎝ 3  4 ⎠",
    "  Shell: $HOME/$USER and $x$.",
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
}

#[test]
fn an_answers_math_renders_and_is_replayed_by_resume() {
    for mode in PRESENTATIONS {
        let mut harness = Harness::start_with(vec![assistant_events(ANSWER)], true, mode);
        harness.type_and_submit("Explain");
        harness.pump_until_screen_shows("Shell: $HOME/$USER");
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
            harness.transcript().contains("Shell: $HOME/$USER")
        });
        assert_rows(&harness.transcript(), &format!("{mode:?} resumed"));
    }
}

/// A few bytes a tick: while the display is open its source shows in the
/// tail and none of it is committed; at the end the rows written are the
/// answer's, each once, with no repair.
#[test]
fn math_streams_without_rewriting_what_it_committed() {
    for mode in PRESENTATIONS {
        let mut harness = Harness::streaming(mode);
        harness.turn_update(TurnUpdate::Started);
        let start = Instant::now();
        let mut saw_source = false;
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
            saw_source |= harness.screen().contains("x=\\frac{-b");
            // The source shows in the stream's tail only, never in a row
            // committed to the transcript.
            for cell in &harness.app.transcript_cells {
                if cell.as_any().is::<LiveCell<StreamTailCell>>() {
                    continue;
                }
                assert!(
                    !plain(&cell.display_lines(WIDTH))
                        .iter()
                        .any(|row| row.contains("\\frac")),
                    "{mode:?}: a formula's source was committed"
                );
            }
        }
        assert!(saw_source, "{mode:?}: the open display showed its source");
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
                .position(|row| row.starts_with("• The spectrum"))
                .expect("the answer was written");
            assert_eq!(written[first..first + expected.len()], expected[..]);
        }
    }
}

/// Codex's interruption case: an answer cut inside an open display keeps
/// the formula's source, laid out again at each width with the same text.
#[test]
fn an_interrupt_inside_a_display_leaves_its_source() {
    for mode in PRESENTATIONS {
        for ending in ["\n", ""] {
            let mut harness = Harness::streaming(mode);
            harness.turn_update(TurnUpdate::Started);
            let source = format!("Intro.\n\n$$\n\\frac{{a+b+c+d+e+f}}{{g+h}}{ending}");
            for delta in ["Intro.\n\n", &source[8..]] {
                harness.turn_update(TurnUpdate::MessageDelta(delta.to_string()));
                harness.app.chat_widget.on_commit_tick();
                harness.settle_events();
            }
            harness.turn_update(TurnUpdate::Ended(TurnEnd::Interrupted));
            assert!(!harness.has_stream_tail(), "{mode:?}");
            let mut contents = Vec::new();
            for width in [80u16, 20, 80] {
                let rows: Vec<String> = harness
                    .app
                    .transcript_cells
                    .iter()
                    .flat_map(|cell| cell.display_lines(width))
                    .map(|line| line.to_string())
                    .collect();
                // The answer's rows; the session header and the notice are
                // not wrapped.
                assert!(
                    rows.iter()
                        .filter(|row| row.starts_with("• ") || row.starts_with("  "))
                        .all(|row| crate::wrapping::display_width(row) <= usize::from(width)),
                    "{mode:?} {width}: {rows:#?}"
                );
                let text = rows.join("\n");
                assert!(text.contains("$$"), "{mode:?} {width}:\n{text}");
                assert!(text.contains("Conversation interrupted"), "{text}");
                // From the answer on: the session header abbreviates.
                let text = text.split_whitespace().collect::<String>();
                contents.push(text[text.find("•Intro.").unwrap()..].to_string());
            }
            assert!(
                contents.iter().all(|text| text == &contents[0]),
                "{contents:?}"
            );
            assert!(contents[0].contains("\\frac{a+b+c+d+e+f}{g+h}"));
        }
    }
}

#[test]
fn math_turned_off_shows_the_next_answers_formulas_as_written() {
    let mut harness = Harness::owned(vec![assistant_events(ANSWER)]);
    harness.type_and_submit("/config interface");
    harness.pump_until_screen_shows("Terminal interface");
    harness.select_setting("Math");
    harness.key(KeyCode::Enter);
    harness.pump_until_screen_shows("Saved");
    assert!(harness.config_file().contains("math = false"));
    harness.key(KeyCode::Esc);
    harness.pump_until("the center to close", |harness| {
        !harness.screen().contains("Changes save automatically")
            && !harness.screen().contains("Saved")
    });
    harness.type_and_submit("Explain");
    harness.pump_until_screen_shows("Shell: $HOME/$USER");
    harness.pump_until_turn_ends();
    let screen = lines(&harness.screen());
    for row in [
        "• The spectrum is $\\lambda_1 \\leq \\lambda_2$ on $\\mathbb{R}^n$; it costs $5 and $10.",
        "  x=\\frac{-b\\pm\\sqrt{b^2-4ac}}{2a}",
        "  $$A=\\begin{pmatrix}1&2\\\\3&4\\end{pmatrix}$$",
        "  Shell: $HOME/$USER and $x$.",
    ] {
        assert!(
            screen.iter().any(|line| line == row),
            "no row {row:?} in\n{}",
            screen.join("\n")
        );
    }
}
