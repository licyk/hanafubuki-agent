//! What leaves Scrollback's tail, and when: laid out by a transcript view
//! as the app lays it out, without a terminal.

use std::sync::Arc;

use hanafubuki_agent::AgentToolDisplay;
use ratatui::crossterm::event::{KeyCode, KeyEvent};
use ratatui::text::Line;

use super::*;
use crate::history_cell::{
    new_info_event, new_tool_call, new_user_prompt, AgentMessageCell, LiveCell, ToolCallCell,
};
use crate::transcript_view::TranscriptView;

const WIDTH: u16 = 60;

fn text(lines: &[Line<'static>]) -> Vec<String> {
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

fn info(text: &str) -> Arc<dyn HistoryCell> {
    Arc::new(new_info_event(text.to_string(), None))
}

/// An answer's lines, the first with its bullet.
fn answer(lines: impl IntoIterator<Item = String>, first: bool) -> Arc<dyn HistoryCell> {
    Arc::new(AgentMessageCell::new(
        lines.into_iter().map(Line::from).collect(),
        first,
    ))
}

/// A call that ran, with `lines` lines of output.
fn tool(call_id: &str, lines: usize) -> Arc<LiveCell<ToolCallCell>> {
    let tool = LiveCell::new(new_tool_call(
        call_id.to_string(),
        "exec_command".to_string(),
        None,
    ));
    let output: Vec<String> = (1..=lines).map(|n| format!("out-{n:02}")).collect();
    tool.update(|cell| {
        cell.finish(
            call_id,
            AgentToolDisplay::Text {
                text: output.join("\n"),
            },
        )
    });
    tool
}

struct Fixture {
    tail: Tail,
    view: TranscriptView,
}

impl Fixture {
    fn new() -> Self {
        Self {
            tail: Tail::default(),
            view: TranscriptView::default(),
        }
    }

    fn take(&mut self, limit: Option<u16>) -> (Vec<String>, Vec<TailEntry>) {
        let (lines, entries) = self.tail.take(&mut self.view, WIDTH, limit);
        (text(&lines), entries)
    }

    fn rows(&mut self) -> usize {
        self.take(Some(u16::MAX))
            .1
            .iter()
            .map(|entry| entry.rows)
            .sum()
    }
}

#[test]
fn the_oldest_entries_leave_once_the_tail_passes_its_limit_and_in_order() {
    let mut fixture = Fixture::new();
    for n in 0..4 {
        fixture.tail.push(info(&format!("note {n}")), false);
    }
    // 1 + 3 × 2 rows: the first has no blank row before it.
    assert_eq!(fixture.rows(), 7);
    let (written, entries) = fixture.take(Some(4));
    assert_eq!(written, ["• note 0", "", "• note 1"]);
    assert_eq!(entries.iter().map(|entry| entry.rows).sum::<usize>(), 4);
    // Under what was written, the next keeps its blank row.
    fixture.tail.push(info("note 4"), false);
    let (written, _) = fixture.take(Some(4));
    assert_eq!(written, ["", "• note 2"]);
}

#[test]
fn a_live_entry_holds_back_everything_after_it_until_it_completes() {
    let mut fixture = Fixture::new();
    let running: Arc<dyn HistoryCell> = LiveCell::new(new_tool_call(
        "call-1".to_string(),
        "web_search".to_string(),
        None,
    ));
    fixture.tail.push(Arc::clone(&running), true);
    for n in 0..5 {
        fixture.tail.push(info(&format!("note {n}")), false);
    }
    let (written, entries) = fixture.take(Some(3));
    assert!(written.is_empty(), "{written:?}");
    assert_eq!(entries.len(), 6);

    fixture.tail.complete(&running);
    let (written, entries) = fixture.take(Some(3));
    assert_eq!(written[0], "• Running web_search");
    // An entry without a control leaves as far as needed: here the blank
    // row before the fourth note.
    assert_eq!(written[written.len() - 2..], ["• note 2", ""]);
    assert_eq!(entries.iter().map(|entry| entry.rows).sum::<usize>(), 3);
}

#[test]
fn an_answer_leaves_line_by_line() {
    let mut fixture = Fixture::new();
    fixture.tail.push(info("before"), false);
    let lines = (1..=10).map(|n| format!("answer line {n}"));
    fixture.tail.push(answer(lines, true), false);
    let (written, entries) = fixture.take(Some(4));
    assert_eq!(
        written,
        [
            "• before",
            "",
            "• answer line 1",
            "  answer line 2",
            "  answer line 3",
            "  answer line 4",
            "  answer line 5",
            "  answer line 6",
        ]
    );
    assert_eq!(entries.iter().map(|entry| entry.rows).sum::<usize>(), 4);
    // The rest goes on where it was, one row at a time.
    fixture
        .tail
        .push(answer(["answer line 11".to_string()], false), false);
    let (written, entries) = fixture.take(Some(4));
    assert_eq!(written, ["  answer line 7"]);
    assert_eq!(entries.iter().map(|entry| entry.rows).sum::<usize>(), 4);
}

#[test]
fn an_entry_with_a_control_leaves_whole_as_it_is_shown() {
    let mut fixture = Fixture::new();
    fixture.tail.push(info("before"), false);
    fixture.tail.push(tool("call-1", 20), false);
    let (written, entries) = fixture.take(Some(1));
    assert!(entries.is_empty(), "{entries:?}");
    // Collapsed: the first lines, then where to see the rest.
    assert_eq!(written[2], "• Called exec_command");
    assert!(written.iter().any(|line| line.contains("out-03")));
    assert!(!written.iter().any(|line| line.contains("out-04")));
    assert_eq!(
        written.last().map(String::as_str),
        Some("    ctrl + t to expand")
    );
    assert!(!written.iter().any(|line| line.contains("+ 17 lines")));
}

#[test]
fn an_expanded_entry_leaves_without_its_control() {
    let mut fixture = Fixture::new();
    fixture.tail.push(tool("call-1", 20), false);
    let (_, entries) = fixture.take(Some(u16::MAX));
    assert!(entries[0].control);
    // Expanded with F4 and Enter, as a person does it.
    let cells = fixture.tail.cells().to_vec();
    for code in [KeyCode::F(4), KeyCode::Enter] {
        assert!(fixture
            .view
            .handle_tail_key(KeyEvent::from(code), &cells)
            .is_some());
    }
    let (written, _) = fixture.take(None);
    assert!(written.contains(&"    out-10".to_string()), "{written:?}");
    assert!(
        !written
            .iter()
            .any(|line| line.contains("Show less") || line.contains("to expand")),
        "{written:?}"
    );
}

#[test]
fn a_message_of_yours_writes_the_tail_before_it_whole() {
    let mut fixture = Fixture::new();
    fixture.tail.push(info("note"), false);
    fixture.tail.push(tool("call-1", 20), false);
    assert!(fixture.take(Some(40)).0.is_empty());
    fixture
        .tail
        .push(Arc::new(new_user_prompt(&"again".into())), false);
    let (written, entries) = fixture.take(Some(40));
    assert_eq!(written[0], "• note");
    assert_eq!(
        written.last().map(String::as_str),
        Some("    ctrl + t to expand")
    );
    assert_eq!(entries.len(), 1);
    assert_eq!(fixture.tail.cells().len(), 1);
}

#[test]
fn a_session_opened_in_its_place_writes_everything_even_what_ran() {
    let mut fixture = Fixture::new();
    let running: Arc<dyn HistoryCell> = LiveCell::new(new_tool_call(
        "call-1".to_string(),
        "web_search".to_string(),
        None,
    ));
    fixture.tail.push(running, true);
    fixture.tail.push(info("note"), false);
    fixture.tail.finish();
    fixture.tail.push(info("new session"), false);
    let (written, entries) = fixture.take(Some(40));
    assert_eq!(written, ["• Running web_search", "  └ …", "", "• note"]);
    assert_eq!(entries.len(), 1);
}

#[test]
fn at_exit_everything_leaves_live_or_not() {
    let mut fixture = Fixture::new();
    let running: Arc<dyn HistoryCell> = LiveCell::new(new_tool_call(
        "call-1".to_string(),
        "web_search".to_string(),
        None,
    ));
    fixture.tail.push(running, true);
    fixture.tail.push(info("note"), false);
    let (written, entries) = fixture.take(None);
    assert_eq!(written, ["• Running web_search", "  └ …", "", "• note"]);
    assert!(entries.is_empty());
}

#[test]
fn the_limit_is_a_share_of_the_screen_with_a_row_above_the_viewport() {
    assert_eq!(tail_limit(40, 5), 24);
    assert_eq!(tail_limit(10, 3), 6);
    assert_eq!(tail_limit(15, 4), 9);
    assert_eq!(tail_limit(4, 6), 0);
}
