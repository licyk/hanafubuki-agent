// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

use insta::assert_snapshot;
use ratatui::backend::TestBackend;
use ratatui::crossterm::event::KeyModifiers;
use ratatui::Terminal;

use super::*;

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
fn the_overlay_frames_the_detailed_transcript() {
    let mut overlay = TranscriptOverlay::new(
        vec![cell("alpha"), cell("beta"), cell("gamma")],
        HistoryRenderMode::Rich,
    );
    let mut terminal = Terminal::new(TestBackend::new(60, 10)).unwrap();
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
fn the_live_tail_follows_the_committed_cells() {
    let mut overlay = numbered(2);
    let mut terminal = Terminal::new(TestBackend::new(40, 12)).unwrap();
    let live = TestCell {
        lines: vec![Line::from("still running")],
    };
    let key = ActiveCellTranscriptKey {
        cacheable: true,
        revision: 1,
        is_stream_continuation: false,
        animation_tick: None,
    };
    assert!(overlay.sync_live_tail(40, Some(key), &[&live]));
    let screen = draw(&mut overlay, &mut terminal);
    let committed = screen.find("line-01").unwrap();
    assert!(
        screen.find("still running").unwrap() > committed,
        "{screen}"
    );
    // Once it is committed, it is gone from the tail.
    assert!(overlay.sync_live_tail(40, None, &[]));
    assert!(!draw(&mut overlay, &mut terminal).contains("still running"));
}

#[test]
fn the_closing_keys_close_it() {
    for code in [KeyCode::Char('q'), KeyCode::Esc] {
        let mut overlay = numbered(3);
        overlay.handle_key(key(code));
        assert!(overlay.is_done());
    }
    let mut overlay = numbered(3);
    overlay.handle_key(KeyEvent::new(KeyCode::Char('t'), KeyModifiers::CONTROL));
    assert!(overlay.is_done());
}
