//! The chooser over a real session, using the same VT100 input and render path as the CLI.

use super::Harness;
use crate::app_event::AppEvent;
use crate::transcript_mode::TranscriptMode;
use hanafubuki_agent::test_support::assistant_events;
use ratatui::crossterm::event::{KeyCode, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};

fn pointer(harness: &mut Harness, column: u16, row: u16, kind: MouseEventKind) {
    harness.send(AppEvent::Mouse(MouseEvent {
        kind,
        column,
        row,
        modifiers: KeyModifiers::NONE,
    }));
}

fn click_text(harness: &mut Harness, label: &str) {
    let screen = harness.pump_until_screen_shows(label);
    let (row, column) = screen
        .lines()
        .enumerate()
        .find_map(|(row, line)| {
            line.find(label)
                .map(|i| (row as u16, line[..i].chars().count() as u16))
        })
        .unwrap();
    pointer(
        harness,
        column,
        row,
        MouseEventKind::Down(MouseButton::Left),
    );
    pointer(harness, column, row, MouseEventKind::Up(MouseButton::Left));
}

#[test]
fn sidebar_mouse_reads_workspace_files_and_keyboard_returns_to_main_draft() {
    for mode in [TranscriptMode::Owned, TranscriptMode::Terminal] {
        let mut harness = Harness::start_with(vec![assistant_events("Ready.")], true, mode);
        std::fs::write(
            harness.home.path().join("project/reader.txt"),
            "Hello from the file.",
        )
        .unwrap();
        let parent = harness.answered("Start", "Ready.");
        for c in "main draft".chars() {
            harness.key(KeyCode::Char(c))
        }
        harness.key_with(KeyCode::Char('x'), KeyModifiers::CONTROL);
        harness.key(KeyCode::Char('s'));
        harness.pump_until_screen_shows("Open in sidebar");
        click_text(&mut harness, "Browse ");
        click_text(&mut harness, "reader.txt");
        harness.pump_until_screen_shows("Hello from the file.");
        harness.key(KeyCode::Esc);
        harness.pump_until_screen_shows("Open in sidebar");
        harness.key_with(KeyCode::Char('x'), KeyModifiers::CONTROL);
        harness.key(KeyCode::Char('x'));
        harness.pump_until("the chooser closed with the main draft intact", |h| {
            !h.app.workspace.sidebar.showing_panel()
                && h.app.chat_widget.composer_text() == "main draft"
        });
        assert_eq!(harness.app.bridge.session_id(), Some(parent));
        assert_eq!(harness.server.requests().len(), 1);
    }
}

#[test]
fn chooser_can_scroll_past_selection_and_open_the_row_under_the_pointer() {
    let mut harness = Harness::owned(vec![assistant_events("Ready.")]);
    for index in 0..64 {
        std::fs::write(
            harness
                .home
                .path()
                .join(format!("project/item-{index:02}.txt")),
            format!("chosen file {index}"),
        )
        .unwrap();
    }
    harness.answered("Start", "Ready.");
    harness.type_and_submit("/sidebar");
    click_text(&mut harness, "Browse ");
    harness.pump_until_screen_shows("item-00.txt");
    for _ in 0..30 {
        pointer(&mut harness, 80, 16, MouseEventKind::ScrollDown);
        harness.app.draw(&mut harness.tui).unwrap();
    }
    click_text(&mut harness, "item-63.txt");
    harness.pump_until_screen_shows("chosen file 63");
}

#[test]
fn document_tabs_restore_the_reader_and_close_to_the_last_visited_file() {
    let mut harness = Harness::owned(vec![assistant_events("Ready.")]);
    for (name, text) in [
        ("first.txt", "First document"),
        ("second.txt", "Second document"),
    ] {
        std::fs::write(harness.home.path().join("project").join(name), text).unwrap();
    }
    harness.answered("Start", "Ready.");
    harness.type_and_submit("/sidebar");
    click_text(&mut harness, "Browse ");
    click_text(&mut harness, "first.txt");
    harness.pump_until_screen_shows("First document");
    harness.key(KeyCode::Esc);
    click_text(&mut harness, "second.txt");
    harness.pump_until_screen_shows("Second document");
    harness.key_with(KeyCode::Char('x'), KeyModifiers::CONTROL);
    harness.key(KeyCode::Up);
    harness.pump_until_screen_shows("First document");
    harness.key_with(KeyCode::Char('x'), KeyModifiers::CONTROL);
    harness.key(KeyCode::Char('w'));
    harness.pump_until_screen_shows("Second document");
}
