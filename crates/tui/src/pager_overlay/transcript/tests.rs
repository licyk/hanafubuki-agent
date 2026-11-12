// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

use hanafubuki_agent::AgentToolDisplay;
use insta::assert_snapshot;
use ratatui::backend::TestBackend;
use ratatui::crossterm::event::{KeyModifiers, MouseButton, MouseEventKind};
use ratatui::Terminal;

use super::*;
use crate::history_cell::{new_tool_call, LiveCell};

#[derive(Debug)]
struct TestCell {
    lines: Vec<Line<'static>>,
}

impl HistoryCell for TestCell {
    fn display_lines(&self, _width: u16) -> Vec<Line<'static>> {
        self.lines.clone()
    }

    fn raw_lines(&self) -> Vec<Line<'static>> {
        self.lines.clone()
    }
}

fn cell(text: &str) -> Arc<dyn HistoryCell> {
    Arc::new(TestCell {
        lines: vec![Line::from(text.to_string())],
    })
}

fn numbered(count: usize) -> TranscriptOverlay {
    TranscriptOverlay::new(
        (0..count).map(|i| cell(&format!("line-{i:02}"))).collect(),
        HistoryRenderMode::Rich,
    )
}

fn draw(overlay: &mut TranscriptOverlay, terminal: &mut Terminal<TestBackend>) -> String {
    terminal
        .draw(|frame| {
            overlay.render(frame.area(), frame.buffer_mut());
        })
        .unwrap();
    terminal.backend().to_string()
}

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

#[test]
fn the_overlay_frames_the_compact_transcript() {
    let mut overlay = TranscriptOverlay::new(
        vec![cell("alpha"), cell("beta"), cell("gamma")],
        HistoryRenderMode::Rich,
    );
    let mut terminal = Terminal::new(TestBackend::new(80, 10)).unwrap();
    draw(&mut overlay, &mut terminal);
    assert_snapshot!(terminal.backend());
}

#[test]
fn a_cell_inserted_at_the_bottom_is_followed() {
    let mut overlay = numbered(20);
    let mut terminal = Terminal::new(TestBackend::new(40, 12)).unwrap();
    draw(&mut overlay, &mut terminal);
    overlay.insert_cell(cell("tail"));
    assert!(draw(&mut overlay, &mut terminal).contains("tail"));
}

#[test]
fn a_reader_scrolled_up_keeps_its_place_when_cells_arrive() {
    let mut overlay = numbered(20);
    let mut terminal = Terminal::new(TestBackend::new(40, 12)).unwrap();
    draw(&mut overlay, &mut terminal);
    overlay.handle_key(key(KeyCode::Home));
    let top = draw(&mut overlay, &mut terminal);
    assert!(top.contains("line-00"), "{top}");
    overlay.insert_cell(cell("tail"));
    let after = draw(&mut overlay, &mut terminal);
    assert!(
        after.contains("line-00") && !after.contains("tail"),
        "{after}"
    );
}

#[test]
fn paging_down_and_up_round_trips() {
    let mut overlay = numbered(50);
    let mut terminal = Terminal::new(TestBackend::new(40, 15)).unwrap();
    draw(&mut overlay, &mut terminal);
    overlay.handle_key(key(KeyCode::Home));
    overlay.handle_key(key(KeyCode::Down));
    overlay.handle_key(key(KeyCode::Down));
    let before = draw(&mut overlay, &mut terminal);
    overlay.handle_key(key(KeyCode::PageDown));
    let next = draw(&mut overlay, &mut terminal);
    assert_ne!(before, next);
    overlay.handle_key(key(KeyCode::PageUp));
    assert_eq!(draw(&mut overlay, &mut terminal), before);
}

#[test]
fn a_live_entry_changes_in_place_among_the_cells() {
    let mut overlay = numbered(2);
    let mut terminal = Terminal::new(TestBackend::new(40, 12)).unwrap();
    let live = LiveCell::new(TestCell {
        lines: vec![Line::from("still running")],
    });
    overlay.insert_cell(live.clone());
    overlay.insert_cell(cell("after"));
    let screen = draw(&mut overlay, &mut terminal);
    let running = screen.find("still running").unwrap();
    assert!(screen.find("line-01").unwrap() < running, "{screen}");
    assert!(running < screen.find("after").unwrap(), "{screen}");
    live.update(|cell| cell.lines = vec![Line::from("finished")]);
    let screen = draw(&mut overlay, &mut terminal);
    assert!(!screen.contains("still running"), "{screen}");
    assert_eq!(screen.find("finished"), Some(running), "{screen}");
}

#[test]
fn the_closing_keys_close_it() {
    for code in [KeyCode::Char('q'), KeyCode::Esc] {
        let mut overlay = numbered(3);
        overlay.handle_key(key(code));
        assert!(overlay.is_done());
    }
    // Ctrl-T shows the details first, then closes it.
    let mut overlay = numbered(3);
    let ctrl_t = KeyEvent::new(KeyCode::Char('t'), KeyModifiers::CONTROL);
    overlay.handle_key(ctrl_t);
    assert!(overlay.is_detailed() && !overlay.is_done());
    overlay.handle_key(ctrl_t);
    assert!(overlay.is_done());
}

#[test]
fn a_click_on_a_control_shows_an_entrys_details_in_the_compact_transcript() {
    let tool = LiveCell::new(new_tool_call(
        "call-1".to_string(),
        "exec_command".to_string(),
        None,
    ));
    let output = (1..=20).map(|n| format!("out-{n:02}")).collect::<Vec<_>>();
    tool.update(|cell| {
        cell.finish(
            "call-1",
            AgentToolDisplay::Text {
                text: output.join("\n"),
            },
        )
    });
    let mut overlay = TranscriptOverlay::new(vec![cell("alpha"), tool], HistoryRenderMode::Rich);
    let mut terminal = Terminal::new(TestBackend::new(60, 30)).unwrap();
    let screen = draw(&mut overlay, &mut terminal);
    assert!(!screen.contains("out-10"), "{screen}");
    let row = screen
        .lines()
        .position(|line| line.contains("+ 17 lines"))
        .expect(&screen);
    // Each row is drawn quoted.
    let column = screen.lines().nth(row).unwrap().find('+').unwrap() - 1;
    for kind in [
        MouseEventKind::Down(MouseButton::Left),
        MouseEventKind::Up(MouseButton::Left),
    ] {
        overlay.handle_mouse(MouseEvent {
            kind,
            column: u16::try_from(column).unwrap(),
            row: u16::try_from(row).unwrap(),
            modifiers: KeyModifiers::NONE,
        });
    }
    let screen = draw(&mut overlay, &mut terminal);
    assert!(screen.contains("out-10"), "{screen}");
}
