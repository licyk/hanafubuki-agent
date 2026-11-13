//! A resize on a vt100 screen, in both presentations: the conversation laid
//! out at one width, the terminal resized, and the screen compared with the
//! same conversation laid out at the new width from the start. Fullscreen
//! lays its view out again; Scrollback, once the size holds, clears the
//! terminal and writes the transcript again from its cells. The vt100
//! screen keeps no scrollback and ignores the purge (`CSI 3 J`), so the
//! visible screen is compared; the rows a rewrite writes are tested on
//! `TranscriptView::scrollback_lines`.

use hanafubuki_agent::AgentToolDisplay;
use ratatui::crossterm::event::KeyCode;

use super::*;
use crate::agent_bridge::{TurnEnd, TurnUpdate};
use crate::history_cell::{new_tool_call, user_prompt, LiveCell};
use crate::pager_overlay::Overlay;

const ANSWER: &str = "Here is the plan for 第二步 and the rest of the run.\n\n| Step | Owner | State | Why it matters |\n|---|---|---|---|\n| Parse the config | parser | done | Every later step reads what the parser keeps, so it goes first. |\n| 第二步：校验 | checker | failed | 一个很长的说明，在窄的列里会换行，读起来很费劲。 |\n| Write the report | writer | pending | The person reads this one, at the end of the run. |\n\n- [x] Parsed, with a note long enough to wrap at a narrow width\n- [ ] Checked\n- Reported\n\n```rust\nfn main() { println!(\"done\"); }\n```\n\nA closing paragraph long enough to wrap at forty columns, with `code` and **strong** text.\n";

const PRESENTATIONS: [TranscriptMode; 2] = [TranscriptMode::Terminal, TranscriptMode::Owned];

impl Harness {
    /// The interface at `width` columns from its first frame.
    fn at_width(mode: TranscriptMode, width: u16) -> Self {
        let mut harness = Harness::start_with(Vec::new(), true, mode);
        harness.resize(width);
        harness.pump_until_screen_shows("fixture-model");
        harness
    }

    fn resize(&mut self, width: u16) {
        self.tui.terminal.backend_mut().resize(width, HEIGHT);
    }

    /// The frames after a resize: the first notes the size, the next runs
    /// the rewrite once its quiet period is over.
    fn settle(&mut self) {
        self.app.draw(&mut self.tui).unwrap();
        self.app.reflow.set_due_for_test();
        self.drain();
    }

    /// Handles what the app was sent, then draws.
    fn drain(&mut self) {
        while let Ok(event) = self.events.try_recv() {
            assert!(self.app.handle_event(&mut self.tui, event));
        }
        self.app.draw(&mut self.tui).unwrap();
    }

    fn update(&mut self, update: TurnUpdate) {
        self.app.chat_widget.handle_turn_update(update);
        self.drain();
    }

    /// `text` streamed line by line, each line's rows committed.
    fn stream(&mut self, text: &str) {
        for line in text.split_inclusive('\n') {
            self.app
                .chat_widget
                .handle_turn_update(TurnUpdate::MessageDelta(line.to_string()));
            for _ in 0..64 {
                self.app.chat_widget.on_commit_tick();
            }
            self.drain();
        }
    }

    /// A message of yours and the answer to it.
    fn turn(&mut self, prompt: &str, answer: &str) {
        let prompt = user_prompt(prompt.to_string(), Vec::new());
        self.send(AppEvent::InsertHistoryCell(Box::new(prompt)));
        self.update(TurnUpdate::Started);
        self.stream(answer);
        self.update(TurnUpdate::MessageCompleted);
        self.update(TurnUpdate::Ended(TurnEnd::Completed));
    }

    fn conversation(&mut self) {
        for n in 1..=3 {
            self.turn(&format!("Question {n}?"), ANSWER);
        }
    }
}

/// The screen's rows without the blank ones under the viewport, which an
/// inline viewport leaves where a turn's taller viewport was.
fn content_rows(harness: &Harness) -> Vec<String> {
    let mut rows = harness.rows();
    while rows.last().is_some_and(String::is_empty) {
        rows.pop();
    }
    rows
}

/// The same rows end the conversation on both screens, most of a screen of
/// them.
fn assert_same_screen(resized: &Harness, fresh: &Harness, what: &str) {
    let (resized, fresh) = (content_rows(resized), content_rows(fresh));
    let shown = resized.len().min(fresh.len());
    assert!(
        shown >= 30,
        "{what}: {shown} rows\n-- resized --\n{}\n-- fresh --\n{}",
        resized.join("\n"),
        fresh.join("\n")
    );
    assert_eq!(
        resized[resized.len() - shown..],
        fresh[fresh.len() - shown..],
        "{what}\n-- resized --\n{}\n-- fresh --\n{}",
        resized.join("\n"),
        fresh.join("\n")
    );
}

/// After a resize, every answer is laid out for the new width: the table
/// changes between its grid and its key/value records both ways, the lists
/// and prose wrap again, CJK included, down to 24 columns.
#[test]
fn after_a_resize_the_conversation_is_laid_out_for_the_new_width() {
    for mode in PRESENTATIONS {
        for (from, to) in [(100, 40), (40, 100), (100, 24)] {
            let mut resized = Harness::at_width(mode, from);
            resized.conversation();
            resized.resize(to);
            resized.settle();
            let mut fresh = Harness::at_width(mode, to);
            fresh.conversation();
            assert_same_screen(&resized, &fresh, &format!("{mode:?} {from} -> {to}"));
            let screen = resized.screen();
            let grid = screen.contains("━━━━");
            assert_eq!(grid, to == 100, "{mode:?} {from} -> {to}:\n{screen}");
        }
    }
}

/// A resize while an answer streams: its rows so far were rendered at the
/// old width, and the rest are, until the stream ends; then the answer is
/// laid out for the new width, in Scrollback by one more rewrite.
#[test]
fn a_resize_mid_stream_is_repaired_when_the_stream_ends() {
    let (first, rest) = ANSWER.split_at(ANSWER.find("- [x]").unwrap());
    for mode in PRESENTATIONS {
        let mut resized = Harness::at_width(mode, 100);
        resized.turn("Question 1?", ANSWER);
        resized.turn("Question 2?", ANSWER);
        resized.send(AppEvent::InsertHistoryCell(Box::new(user_prompt(
            "Question 3?".to_string(),
            Vec::new(),
        ))));
        resized.update(TurnUpdate::Started);
        resized.stream(first);
        resized.resize(40);
        resized.settle();
        resized.stream(rest);
        resized.update(TurnUpdate::MessageCompleted);
        resized.update(TurnUpdate::Ended(TurnEnd::Completed));
        // Scrollback's rewrite at the stream's end runs at the next frame.
        resized.drain();
        let mut fresh = Harness::at_width(mode, 40);
        fresh.conversation();
        assert_same_screen(&resized, &fresh, &format!("{mode:?}"));
        if !mode.is_owned() {
            assert!(!resized.app.reflow.is_pending());
        }
    }
}

/// A drag through several widths writes the scrollback once, at the width
/// it ends at, after the size has held.
#[test]
fn a_drag_rewrites_the_scrollback_once_the_size_holds() {
    let mut harness = Harness::at_width(TranscriptMode::Terminal, 100);
    harness.conversation();
    for width in [90, 70, 50] {
        harness.resize(width);
        harness.app.draw(&mut harness.tui).unwrap();
        assert!(harness.app.reflow.is_pending(), "at {width}");
    }
    harness.settle();
    assert!(!harness.app.reflow.is_pending());
    let mut fresh = Harness::at_width(TranscriptMode::Terminal, 50);
    fresh.conversation();
    assert_same_screen(&harness, &fresh, "dragged to 50");
}

/// While the transcript overlay holds the alternate screen it is laid out
/// at the new width, and the scrollback is written again once it closes.
#[test]
fn a_resize_under_the_transcript_overlay_rewrites_the_scrollback_once_it_closes() {
    let mut harness = Harness::at_width(TranscriptMode::Terminal, 100);
    harness.conversation();
    harness.key_with(KeyCode::Char('t'), KeyModifiers::CONTROL);
    harness.drain();
    assert!(matches!(harness.app.overlay, Some(Overlay::Transcript(_))));
    harness.resize(40);
    harness.drain();
    let screen = harness.screen();
    assert!(!screen.contains("━━━━"), "{screen}");
    assert!(screen.contains("Why it matters"), "{screen}");
    for _ in 0..3 {
        if harness.app.overlay.is_some() {
            harness.key(KeyCode::Esc);
        }
    }
    assert!(harness.app.overlay.is_none());
    harness.settle();
    let mut fresh = Harness::at_width(TranscriptMode::Terminal, 40);
    fresh.conversation();
    assert_same_screen(&harness, &fresh, "after the overlay");
}

/// A call's output, expanded or collapsed when it was written, is written
/// again so; in Fullscreen the disclosure stays as it was.
#[test]
fn a_resize_keeps_what_each_entry_disclosed() {
    for mode in PRESENTATIONS {
        let mut harness = Harness::at_width(mode, 100);
        harness.update(TurnUpdate::Started);
        for (call_id, expand) in [("call-1", true), ("call-2", false)] {
            let tool = LiveCell::new(new_tool_call(
                call_id.to_string(),
                "exec_command".to_string(),
                Some(format!("ls {call_id}")),
            ));
            harness.send(AppEvent::InsertLiveEntry(tool.clone()));
            let output: Vec<String> = (1..=6).map(|n| format!("{call_id}-out-{n}")).collect();
            tool.update(|cell| {
                cell.finish(
                    call_id,
                    AgentToolDisplay::Command {
                        command: format!("ls {call_id}"),
                        cwd: None,
                        output: output.join("\n"),
                        exit_code: Some(0),
                        duration_millis: Some(10),
                        truncated: false,
                    },
                )
            });
            harness.send(AppEvent::LiveEntryCompleted(tool));
            if expand {
                harness.click_on("+ 3 lines");
            }
        }
        // A message of yours writes Scrollback's tail as it is shown.
        harness.turn("Next?", "Done.");
        harness.resize(60);
        harness.settle();
        let after = harness.screen();
        assert!(after.contains("call-1-out-2"), "{mode:?}:\n{after}");
        assert!(!after.contains("call-2-out-2"), "{mode:?}:\n{after}");
        assert_eq!(
            after.matches("Ran ls call-1").count(),
            1,
            "{mode:?}: written once\n{after}"
        );
    }
}

/// Fullscreen with the find bar open on the table, then resized: no panic;
/// the match keeps the layout it was found in while the search lasts, and
/// once it ends the view is the new width's.
#[test]
fn a_resize_with_the_find_bar_open_keeps_the_search() {
    let mut harness = Harness::at_width(TranscriptMode::Owned, 100);
    harness.conversation();
    harness.key(KeyCode::F(3));
    for character in "Why it".chars() {
        harness.key(KeyCode::Char(character));
    }
    harness.drain();
    assert!(harness.app.transcript_view.is_search_editing());
    harness.resize(40);
    harness.drain();
    assert!(harness.app.transcript_view.is_search_editing());
    // Esc leaves the search (a second one would prime editing a message).
    while harness.app.transcript_view.has_active_interaction() {
        harness.key(KeyCode::Esc);
    }
    harness.app.transcript_view.jump_to_latest();
    harness.drain();
    let mut fresh = Harness::at_width(TranscriptMode::Owned, 40);
    fresh.conversation();
    assert_same_screen(&harness, &fresh, "after the search");
}

/// A reader scrolled into an answer as its stream ends keeps the rows it
/// reads, though the consolidated cell spaces its lists otherwise.
#[test]
fn a_reader_inside_a_streaming_answer_keeps_its_rows_when_it_is_consolidated() {
    let mut harness = Harness::at_width(TranscriptMode::Owned, 60);
    harness.turn("Question 1?", ANSWER);
    harness.send(AppEvent::InsertHistoryCell(Box::new(user_prompt(
        "Question 2?".to_string(),
        Vec::new(),
    ))));
    harness.update(TurnUpdate::Started);
    harness.stream(&format!("{ANSWER}\n{ANSWER}"));
    let cells = harness.app.transcript_cells.clone();
    harness.app.transcript_view.scroll(&cells, -8);
    harness.drain();
    assert!(!harness.app.transcript_view.is_following());
    let reading = harness.rows();
    harness.update(TurnUpdate::MessageCompleted);
    harness.update(TurnUpdate::Ended(TurnEnd::Completed));
    let last = harness.app.transcript_cells.last().unwrap();
    assert!(last.stream_id().is_none(), "consolidated");
    // The status row under the view goes; the view's rows stay.
    assert_eq!(harness.rows()[..30], reading[..30]);
}

/// Fullscreen with a selection in a streaming answer: resized, the stream
/// consolidated under it and resized again, the selection keeps what it
/// selected; once it ends, the view is the new width's.
#[test]
fn a_selection_holds_through_a_resize_and_a_consolidation() {
    let mut harness = Harness::at_width(TranscriptMode::Owned, 100);
    harness.turn("Question 1?", ANSWER);
    harness.send(AppEvent::InsertHistoryCell(Box::new(user_prompt(
        "Question 2?".to_string(),
        Vec::new(),
    ))));
    harness.update(TurnUpdate::Started);
    harness.stream(ANSWER);
    harness.key_with(KeyCode::Char(' '), KeyModifiers::CONTROL);
    for _ in 0..6 {
        harness.key_with(KeyCode::Up, KeyModifiers::SHIFT);
    }
    harness.drain();
    assert!(harness.app.transcript_view.has_active_interaction());
    harness.resize(40);
    harness.drain();
    harness.update(TurnUpdate::MessageCompleted);
    harness.update(TurnUpdate::Ended(TurnEnd::Completed));
    harness.resize(24);
    harness.drain();
    assert!(harness.app.transcript_view.has_active_interaction());
    while harness.app.transcript_view.has_active_interaction() {
        harness.key(KeyCode::Esc);
    }
    harness.app.transcript_view.jump_to_latest();
    harness.drain();
    let mut fresh = Harness::at_width(TranscriptMode::Owned, 24);
    fresh.turn("Question 1?", ANSWER);
    fresh.turn("Question 2?", ANSWER);
    assert_same_screen(&harness, &fresh, "after the selection");
}
