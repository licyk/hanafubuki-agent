//! An answer streaming on a vt100 screen, in both presentations: what it
//! commits is never written again, what it holds (a table arriving) shows
//! live after it, a burst is caught up at once, and its end, an interrupt
//! or a resize leave the same rows as the whole answer would.

use std::sync::atomic::Ordering;

use ratatui::text::Line;

use super::*;
use crate::agent_bridge::{TurnEnd, TurnUpdate};
use crate::history_cell::{
    new_info_event, AgentMarkdownCell, HistoryCell, LiveCell, StreamTailCell,
};

const ANSWER: &str = "Here is the plan for 第二步 and the rest of the run.\n\n| Step | Owner | State |\n|---|---|---|\n| Parse the config | parser | done |\n| 第二步：校验 | checker | failed |\n| Write the report | writer | pending |\n\n```rust\nfn main() {\n    println!(\"done\");\n}\n```\n\n- [x] Parsed\n- [ ] Checked\n\nA closing paragraph long enough to wrap, with `code` and **strong** text.\n";

const PRESENTATIONS: [TranscriptMode; 2] = [TranscriptMode::Terminal, TranscriptMode::Owned];

impl Harness {
    fn streaming(mode: TranscriptMode) -> Self {
        let mut harness = Harness::start_with(Vec::new(), true, mode);
        harness.pump_until_screen_shows("fixture-model");
        harness
    }

    /// Handles what the app was sent, then draws.
    fn settle_events(&mut self) {
        while let Ok(event) = self.events.try_recv() {
            assert!(self.app.handle_event(&mut self.tui, event));
        }
        self.app.draw(&mut self.tui).unwrap();
    }

    fn turn_update(&mut self, update: TurnUpdate) {
        self.app.chat_widget.handle_turn_update(update);
        self.settle_events();
    }

    /// The lines written into the scrollback, as text.
    fn written(&self) -> Vec<String> {
        plain(&self.tui.written_history_lines)
    }

    /// The stream's tail entry, while it is in the transcript.
    fn stream_tail(&self) -> Option<&Arc<dyn HistoryCell>> {
        self.app
            .transcript_cells
            .iter()
            .find(|cell| cell.as_any().is::<LiveCell<StreamTailCell>>())
    }

    fn has_stream_tail(&self) -> bool {
        self.stream_tail().is_some()
    }
}

fn plain(lines: &[Line<'static>]) -> Vec<String> {
    lines
        .iter()
        .map(|line| {
            line.spans
                .iter()
                .map(|span| span.content.as_ref())
                .collect::<String>()
                .trim_end()
                .to_string()
        })
        .collect()
}

/// The source in chunks of a few bytes, cut at character boundaries.
fn chunks(source: &str) -> Vec<&str> {
    let mut chunks = Vec::new();
    let mut start = 0;
    let mut step = 3;
    while start < source.len() {
        step = (step * 7 + 3) % 11 + 1;
        let mut end = (start + step).min(source.len());
        while !source.is_char_boundary(end) {
            end += 1;
        }
        chunks.push(&source[start..end]);
        start = end;
    }
    chunks
}

/// The plan's acceptance: an answer with a table and a code block streams
/// a few bytes at a time, a tick of the animation after each, every tick
/// `8 ms` after the one before. In Scrollback what is written into the
/// scrollback is the answer's rows in order, each once, never rewritten
/// (no repair is asked at the end); in both presentations the table shows
/// while it arrives and the answer ends as the whole answer renders.
#[test]
fn a_long_answer_streams_without_rewriting_what_it_committed() {
    for mode in PRESENTATIONS {
        let mut harness = Harness::streaming(mode);
        harness.turn_update(TurnUpdate::Started);
        let start = Instant::now();
        let mut saw_table_arriving = false;
        for (tick, chunk) in chunks(ANSWER).into_iter().enumerate() {
            harness
                .app
                .chat_widget
                .handle_turn_update(TurnUpdate::MessageDelta(chunk.to_string()));
            let now = start + Duration::from_millis(8 * tick as u64);
            harness.app.chat_widget.on_commit_tick_at(now);
            harness.settle_events();
            let screen = harness.screen();
            if screen.contains("Parse the config") && !screen.contains("Write the report") {
                saw_table_arriving |= screen.contains("━━━");
            }
        }
        assert!(
            saw_table_arriving,
            "{mode:?}: the table showed while it arrived"
        );
        harness.turn_update(TurnUpdate::MessageCompleted);
        harness.turn_update(TurnUpdate::Ended(TurnEnd::Completed));
        assert!(!harness.has_stream_tail(), "{mode:?}");

        let expected = plain(
            &AgentMarkdownCell::new(ANSWER.to_string(), "/project".to_string())
                .display_lines(WIDTH),
        );
        if mode.is_owned() {
            let screen = harness.screen();
            for row in expected.iter().filter(|row| !row.is_empty()) {
                assert!(screen.contains(row.as_str()), "{row:?} not in\n{screen}");
            }
        } else {
            assert!(!harness.app.reflow.is_pending(), "no repair was needed");
            harness.app.write_tail_at_exit(&mut harness.tui).unwrap();
            let written = harness.written();
            let first = written
                .iter()
                .position(|row| row.starts_with("• Here is the plan"))
                .expect("the answer was written");
            assert_eq!(written[first..first + expected.len()], expected[..]);
        }
    }
}

/// The rows each tick commits go before the tail, which shows the line
/// still arriving last.
#[test]
fn committed_rows_come_before_the_line_still_arriving() {
    for mode in PRESENTATIONS {
        let mut harness = Harness::streaming(mode);
        harness.turn_update(TurnUpdate::Started);
        for delta in ["First row.\nSec", "ond row.\nThe line still arriv"] {
            harness
                .app
                .chat_widget
                .handle_turn_update(TurnUpdate::MessageDelta(delta.to_string()));
            harness.app.chat_widget.on_commit_tick();
            harness.settle_events();
        }
        let rows = [
            harness.find("First row."),
            harness.find("Second row."),
            harness.find("The line still arriv"),
        ];
        assert!(
            rows[0].0 < rows[1].0 && rows[1].0 < rows[2].0,
            "{mode:?}: {rows:?}"
        );
    }
}

/// A burst queues many rows at once: the delta's own catch-up commits them
/// before the next tick.
#[test]
fn a_burst_is_caught_up_at_once() {
    for mode in PRESENTATIONS {
        let mut harness = Harness::streaming(mode);
        harness.turn_update(TurnUpdate::Started);
        let lines: String = (1..=30)
            .map(|n| format!("Line {n:02} of the burst.\n"))
            .collect();
        harness.turn_update(TurnUpdate::MessageDelta(lines));
        assert!(harness.app.chat_widget.stream_queues_idle(), "{mode:?}");
        harness.pump_until_screen_shows("Line 30 of the burst.");
    }
}

/// A table arriving is laid out again for the new width at once, in the
/// tail, before the stream ends.
#[test]
fn a_resize_lays_out_the_table_arriving_at_the_new_width() {
    for mode in PRESENTATIONS {
        let mut harness = Harness::streaming(mode);
        harness.turn_update(TurnUpdate::Started);
        let table = "| Step | Owner | Why it matters |\n|---|---|---|\n| Parse the config | parser | Every later step reads what the parser keeps. |\n";
        harness.turn_update(TurnUpdate::MessageDelta(table.to_string()));
        assert!(harness.screen().contains("━━━"), "{mode:?}");
        harness.tui.terminal.backend_mut().resize(40, HEIGHT);
        harness.app.draw(&mut harness.tui).unwrap();
        harness.settle_events();
        let tail = harness
            .stream_tail()
            .expect("the stream's tail")
            .display_lines(40);
        assert!(
            tail.iter().all(|line| line.width() <= 40),
            "{mode:?}: {tail:?}"
        );
        // Its cells wrap in the narrower columns.
        assert!(
            plain(&tail)
                .iter()
                .any(|row| row.trim_start().starts_with("config ")),
            "{mode:?}: {:?}",
            plain(&tail)
        );
    }
}

/// An interrupt while a table arrives commits it as far as it arrived; the
/// stream's tail is gone and Scrollback writes what comes after.
#[test]
fn an_interrupt_mid_table_commits_what_arrived_and_releases_the_tail() {
    for mode in PRESENTATIONS {
        let mut harness = Harness::streaming(mode);
        harness.turn_update(TurnUpdate::Started);
        harness.turn_update(TurnUpdate::MessageDelta(
            "Intro.\n\n| A | B |\n|---|---|\n| 1 | 2 |\n| 3".to_string(),
        ));
        harness.turn_update(TurnUpdate::Ended(TurnEnd::Interrupted));
        assert!(!harness.has_stream_tail(), "{mode:?}");
        assert!(harness.app.chat_widget.stream_queues_idle());
        let transcript = harness.transcript();
        assert!(transcript.contains("1      2"), "{mode:?}:\n{transcript}");
        assert!(transcript.contains("Conversation interrupted"), "{mode:?}");
        if !mode.is_owned() {
            for n in 1..=60 {
                let cell = new_info_event(format!("note {n:02}"), None);
                harness.send(AppEvent::InsertHistoryCell(Box::new(cell)));
            }
            let written = harness.written();
            assert!(
                written.iter().any(|row| row.contains("note 30")),
                "{written:?}"
            );
            assert!(written.iter().any(|row| row.contains("1      2")));
        }
    }
}

/// The stream ends with rows still queued: the next tick stops the commit
/// animation, which had nothing left to commit (before, the tick thread
/// ran on until the next stream).
#[test]
fn the_commit_animation_stops_when_a_stream_ends_with_rows_queued() {
    let mut harness = Harness::streaming(TranscriptMode::Terminal);
    harness.turn_update(TurnUpdate::Started);
    let chat = &mut harness.app.chat_widget;
    chat.handle_turn_update(TurnUpdate::MessageDelta("one\ntwo\nthree\n".to_string()));
    assert!(
        !chat.stream_queues_idle(),
        "three rows are queued, not committed"
    );
    chat.handle_turn_update(TurnUpdate::MessageCompleted);
    // The animation as started, without its thread: the test sends the
    // tick itself.
    while let Ok(event) = harness.events.try_recv() {
        match event {
            AppEvent::StartCommitAnimation => harness
                .app
                .commit_anim_running
                .store(true, Ordering::Release),
            event => assert!(harness.app.handle_event(&mut harness.tui, event)),
        }
    }
    assert!(harness.app.commit_anim_running.load(Ordering::Acquire));
    harness.send(AppEvent::CommitTick);
    assert!(!harness.app.commit_anim_running.load(Ordering::Acquire));
    let screen = harness.screen();
    assert!(screen.contains("three"), "{screen}");
}
