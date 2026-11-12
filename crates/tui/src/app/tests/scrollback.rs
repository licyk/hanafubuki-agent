//! Scrollback's tail on a vt100 screen: entries that change and expand in
//! place above the chat, written into the scrollback as they leave it, a
//! viewport that keeps its height during a turn, and the mouse taken and
//! let go.

use std::sync::Arc;

use hanafubuki_agent::test_support::assistant_events;
use hanafubuki_agent::{AgentScrollbackMouse, AgentToolDisplay};
use ratatui::crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
use ratatui::text::Line;

use super::*;
use crate::agent_bridge::{TurnEnd, TurnUpdate};
use crate::history_cell::{
    new_info_event, new_tool_call, new_user_prompt, AgentMessageCell, LiveCell, ToolCallCell,
};

impl Harness {
    fn send(&mut self, event: AppEvent) {
        assert!(self.app.handle_event(&mut self.tui, event));
        self.app.draw(&mut self.tui).unwrap();
    }

    fn note(&mut self, text: &str) {
        let cell = new_info_event(text.to_string(), None);
        self.send(AppEvent::InsertHistoryCell(Box::new(cell)));
    }

    /// The screen's rows, each without its trailing blanks.
    pub(super) fn rows(&self) -> Vec<String> {
        let screen = self.tui.terminal.backend().vt100().screen();
        screen
            .rows(0, screen.size().1)
            .map(|row| row.trim_end().to_string())
            .collect()
    }

    /// The row and column of `text` on the screen.
    pub(super) fn find(&self, text: &str) -> (u16, u16) {
        let rows = self.rows();
        let row = rows
            .iter()
            .position(|row| row.contains(text))
            .unwrap_or_else(|| panic!("{text:?} not on the screen:\n{}", rows.join("\n")));
        let column = rows[row][..rows[row].find(text).unwrap()].chars().count();
        (u16::try_from(row).unwrap(), u16::try_from(column).unwrap())
    }

    pub(super) fn click_on(&mut self, text: &str) {
        let (row, column) = self.find(text);
        for kind in [
            MouseEventKind::Down(MouseButton::Left),
            MouseEventKind::Up(MouseButton::Left),
        ] {
            self.send(AppEvent::Mouse(MouseEvent {
                kind,
                column,
                row,
                modifiers: KeyModifiers::NONE,
            }));
        }
    }

    fn viewport(&self) -> ratatui::layout::Rect {
        self.tui.terminal.viewport_area
    }
}

/// The call ran `ls`, with `lines` lines of output.
fn completed(tool: &LiveCell<ToolCallCell>, call_id: &str, lines: usize) {
    let output: Vec<String> = (1..=lines).map(|n| format!("out-{n:02}")).collect();
    tool.update(|cell| {
        cell.finish(
            call_id,
            AgentToolDisplay::Command {
                command: "ls".to_string(),
                cwd: None,
                output: output.join("\n"),
                exit_code: Some(0),
                duration_millis: Some(10),
                truncated: false,
            },
        )
    });
}

fn running(call_id: &str) -> Arc<LiveCell<ToolCallCell>> {
    LiveCell::new(new_tool_call(
        call_id.to_string(),
        "exec_command".to_string(),
        Some("ls".to_string()),
    ))
}

#[test]
fn a_call_changes_in_place_in_the_tail_where_a_click_or_f4_expands_it() {
    let mut harness = Harness::start(Vec::new());
    harness.pump_until_screen_shows("fixture-model");
    harness
        .app
        .chat_widget
        .handle_turn_update(TurnUpdate::Started);
    let tool = running("call-1");
    harness.send(AppEvent::InsertLiveEntry(tool.clone()));
    harness.find("Running ls");
    completed(&tool, "call-1", 20);
    harness.send(AppEvent::LiveEntryCompleted(tool.clone()));
    let screen = harness.screen();
    assert_eq!(screen.matches("Ran ls").count(), 1, "{screen}");
    assert!(!screen.contains("Running"), "{screen}");
    assert!(screen.contains("out-20"), "{screen}");
    assert!(!screen.contains("out-10"), "{screen}");
    assert!(harness.tui.captures_inline_mouse());

    // The viewport grows with it, from the bottom of the screen.
    harness.click_on("+ 17 lines");
    harness.pump_until_screen_shows("out-10");
    let (expanded, _) = harness.find("Ran ls");

    // A click focuses the call: Esc leaves it, F4 focuses it again and
    // Enter collapses it. The viewport does not shrink during the turn, so
    // the entry stays where it is.
    harness.key(KeyCode::Esc);
    assert!(!harness.app.transcript_view.is_activity_focused());
    harness.key(KeyCode::F(4));
    harness.key(KeyCode::Enter);
    harness.app.draw(&mut harness.tui).unwrap();
    assert!(!harness.screen().contains("out-10"));
    assert_eq!(harness.find("Ran ls").0, expanded);
}

#[test]
fn the_viewport_keeps_its_height_during_a_turn() {
    let mut harness = Harness::start(Vec::new());
    harness.pump_until_screen_shows("fixture-model");
    harness
        .app
        .chat_widget
        .handle_turn_update(TurnUpdate::Started);
    let tool = running("call-1");
    completed(&tool, "call-1", 14);
    harness.send(AppEvent::InsertLiveEntry(tool.clone()));
    harness.send(AppEvent::LiveEntryCompleted(tool));
    harness.click_on("+ 11 lines");
    harness.pump_until_screen_shows("out-07");
    let expanded = harness.viewport();
    harness.click_on("− Show less");
    harness.app.draw(&mut harness.tui).unwrap();
    assert!(!harness.screen().contains("out-07"));
    assert_eq!(harness.viewport(), expanded);

    harness
        .app
        .chat_widget
        .handle_turn_update(TurnUpdate::Ended(TurnEnd::Completed));
    harness.app.draw(&mut harness.tui).unwrap();
    assert!(harness.viewport().height < expanded.height);
}

#[test]
fn the_mouse_is_let_go_at_the_wheel_and_taken_again_at_the_next_key() {
    let mut harness = Harness::start(Vec::new());
    harness.pump_until_screen_shows("fixture-model");
    harness.note("no control here");
    assert!(!harness.tui.captures_inline_mouse());
    let tool = running("call-1");
    completed(&tool, "call-1", 20);
    harness.send(AppEvent::InsertLiveEntry(tool.clone()));
    harness.send(AppEvent::LiveEntryCompleted(tool));
    assert!(harness.tui.captures_inline_mouse());

    let wheel = MouseEvent {
        kind: MouseEventKind::ScrollUp,
        column: 0,
        row: 0,
        modifiers: KeyModifiers::NONE,
    };
    harness.send(AppEvent::Mouse(wheel));
    assert!(!harness.tui.captures_inline_mouse());
    // An answer's next lines are not a new entry.
    let chunk = AgentMessageCell::new(vec![Line::from("more of the answer")], false);
    harness.send(AppEvent::InsertHistoryCell(Box::new(chunk)));
    assert!(!harness.tui.captures_inline_mouse());
    harness.key(KeyCode::Right);
    harness.app.draw(&mut harness.tui).unwrap();
    assert!(harness.tui.captures_inline_mouse());

    harness.send(AppEvent::Mouse(wheel));
    harness.note("a new entry");
    assert!(harness.tui.captures_inline_mouse());

    harness.app.tail_mouse.setting = AgentScrollbackMouse::Off;
    harness.app.draw(&mut harness.tui).unwrap();
    assert!(!harness.tui.captures_inline_mouse());
}

#[test]
fn the_oldest_entries_are_written_above_the_tail_in_order_and_the_rest_on_the_next_message() {
    let mut harness = Harness::start(Vec::new());
    // A short screen: the tail's limit still leaves a row above the
    // viewport.
    let mut terminal = Terminal::with_options(VT100Backend::new(WIDTH, 15)).unwrap();
    terminal.set_viewport_area(ratatui::layout::Rect::new(0, 14, WIDTH, 1));
    harness.tui = Tui::new(terminal);
    harness.pump_until_screen_shows("fixture-model");
    for n in 0..12 {
        harness.note(&format!("note {n:02}"));
    }
    let rows = harness.rows();
    let viewport = harness.viewport();
    assert!(viewport.y >= 1, "{viewport:?}");
    let shown: Vec<&String> = rows.iter().filter(|row| row.contains("• note")).collect();
    let numbers: Vec<&str> = shown
        .iter()
        .filter_map(|row| row.split_whitespace().last())
        .collect();
    let mut sorted = numbers.clone();
    sorted.sort();
    assert_eq!(numbers, sorted, "{}", rows.join("\n"));
    assert_eq!(numbers.last().copied(), Some("11"));
    let in_tail = harness.app.tail.cells().len();
    assert!(in_tail < 12 + 2, "{in_tail}");

    // Your next message writes the tail before it.
    let prompt = new_user_prompt("next".to_string());
    harness.send(AppEvent::InsertHistoryCell(Box::new(prompt)));
    assert_eq!(harness.app.tail.cells().len(), 1);
    let rows = harness.rows();
    let (prompt_row, _) = harness.find("› next");
    let (last_row, _) = harness.find("• note 11");
    assert!(last_row < prompt_row, "{}", rows.join("\n"));
    assert!(last_row < harness.viewport().y, "{}", rows.join("\n"));
}

#[test]
fn a_long_answer_is_written_line_by_line_after_the_question() {
    let items: Vec<String> = (1..=20).map(|n| format!("- item {n:02}")).collect();
    let mut harness = Harness::start(vec![assistant_events(&items.join("\n"))]);
    harness.type_and_submit("List them.");
    harness.pump_until_turn_ends();
    harness.pump_until_screen_shows("item 20");
    let rows = harness.rows();
    let mut from = harness.find("› List them.").0;
    for n in 1..=20 {
        let (row, _) = harness.find(&format!("item {n:02}"));
        assert!(row > from, "item {n:02}:\n{}", rows.join("\n"));
        from = row;
    }
    let tail_rows: usize = harness
        .app
        .tail
        .cells()
        .iter()
        .map(|cell| cell.display_lines(WIDTH).len())
        .sum();
    assert!(tail_rows <= 24, "{tail_rows}\n{}", rows.join("\n"));
}

#[test]
fn at_exit_the_tail_is_written_whole_running_calls_too() {
    let mut harness = Harness::start(Vec::new());
    harness.pump_until_screen_shows("fixture-model");
    let tool = running("call-1");
    harness.send(AppEvent::InsertLiveEntry(tool));
    harness.note("last words");
    harness.app.write_tail_at_exit(&mut harness.tui).unwrap();
    assert!(harness.app.tail.cells().is_empty());
    let viewport = harness.viewport().y;
    assert!(harness.find("Running ls").0 < viewport);
    assert!(harness.find("last words").0 < viewport);
}

#[test]
fn a_resize_writes_the_tail() {
    let mut harness = Harness::start(Vec::new());
    harness.pump_until_screen_shows("fixture-model");
    harness
        .app
        .chat_widget
        .handle_turn_update(TurnUpdate::Started);
    harness.note("before the resize");
    assert!(!harness.app.tail.cells().is_empty());
    let mut terminal = Terminal::with_options(VT100Backend::new(WIDTH - 20, HEIGHT)).unwrap();
    terminal.set_viewport_area(ratatui::layout::Rect::new(0, HEIGHT - 1, WIDTH - 20, 1));
    harness.tui = Tui::new(terminal);
    harness.app.draw(&mut harness.tui).unwrap();
    assert!(harness.app.tail.cells().is_empty());
    assert!(harness.find("before the resize").0 < harness.viewport().y);
}
