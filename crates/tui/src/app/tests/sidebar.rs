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
        click_text(&mut harness, "? help");
        harness.pump_until_screen_shows("Sidebar help");
        harness.key(KeyCode::End);
        harness.pump_until_screen_shows("100%");
        harness.key(KeyCode::Esc);
        click_text(&mut harness, "Browse ");
        click_text(&mut harness, "reader.txt");
        harness.pump_until_screen_shows("Hello from the file.");
        harness.key(KeyCode::Char('?'));
        harness.pump_until_screen_shows("Sidebar help");
        harness.key(KeyCode::Esc);
        harness.pump_until_screen_shows("Hello from the file.");
        harness.key(KeyCode::Char('/'));
        harness.key(KeyCode::Char('?'));
        assert!(harness.app.overlay.is_none());
        harness.key(KeyCode::Esc);
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
fn main_agent_entry_restores_the_parent_from_an_interactive_subagent() {
    let mut h = Harness::owned(vec![
        super::subagents::spawn_and_wait("Count the files."),
        assistant_events("Twelve files."),
        assistant_events("There are twelve."),
    ]);
    let parent = h.answered("Delegate the count.", "There are twelve.");
    let child = h.subagents_of(parent)[0];
    h.key_with(KeyCode::Right, KeyModifiers::ALT);
    h.pump_until("interactive child selected", |h| {
        h.app.bridge.session_id() == Some(child)
    });
    h.type_and_submit("/sidebar");
    click_text(&mut h, "Return to main conversation");
    h.pump_until("parent conversation focused", |h| {
        !h.app.workspace.sidebar.showing_panel() && h.app.bridge.session_id() == Some(parent)
    });
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
    for _ in 0..64 {
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

#[test]
fn chooser_reflows_the_same_entries_and_their_click_targets_on_resize() {
    let mut h = Harness::owned(vec![assistant_events("Ready.")]);
    let docs = h.home.path().join("project/docs");
    std::fs::create_dir(&docs).unwrap();
    std::fs::write(docs.join("guide.md"), "Inside the guide").unwrap();
    h.answered("Start", "Ready.");
    h.tui.terminal.backend_mut().resize(120, 22);
    h.type_and_submit("/sidebar");
    click_text(&mut h, "Browse ");
    click_text(&mut h, "docs/");
    let narrow = h.pump_until_screen_shows("guide.md");
    let sidebar = |frame: &str| {
        frame
            .lines()
            .filter_map(|line| line.rsplit_once('│').map(|(_, side)| side.trim_end()))
            .collect::<Vec<_>>()
            .join("\n")
    };
    insta::assert_snapshot!("chooser_at_40_columns", sidebar(&narrow));
    h.tui.terminal.backend_mut().resize(198, 22);
    h.app.draw(&mut h.tui).unwrap();
    insta::assert_snapshot!("chooser_at_66_columns", sidebar(&h.screen()));
    h.tui.terminal.backend_mut().resize(120, 22);
    h.app.draw(&mut h.tui).unwrap();
    let narrow = h.screen();
    let (row, line) = narrow
        .lines()
        .enumerate()
        .find(|(_, line)| line.contains("guide.md"))
        .unwrap();
    let col = line
        .find('│')
        .map(|n| line[..n].chars().count() as u16 + 6)
        .unwrap();
    for event in [
        MouseEventKind::Down(MouseButton::Left),
        MouseEventKind::Up(MouseButton::Left),
    ] {
        pointer(&mut h, col, row as u16 + 1, event);
    }
    h.pump_until_screen_shows("Inside the guide");
}

#[test]
fn chooser_shows_the_last_own_prompt_of_a_live_and_then_closed_saved_chat() {
    let mut harness = Harness::owned(vec![
        assistant_events("Ready."),
        assistant_events("First answer."),
        assistant_events("Second answer."),
    ]);
    let parent = harness.answered("Parent-only prompt", "Ready.");
    harness.app.bridge.set_page_size(1);
    harness.type_and_submit("/sidebar");
    click_text(&mut harness, "New side chat");
    harness.pump_until("a saved chat", |h| h.app.active_sidebar_chat().is_some());
    harness.key(KeyCode::Char('?'));
    harness.pump_until_screen_shows("Sidebar help");
    harness.key(KeyCode::Esc);
    harness.answered("Earlier prompt?", "First answer.");
    let saved = harness.answered("Latest    prompt", "Second answer.");

    // This empty saved fork still includes the parent's user message in model history.
    let agent = harness.runtime.slot().agent().unwrap();
    agent
        .create_side_conversation(parent, hanafubuki_agent::AgentSessionPersistence::Durable)
        .unwrap();
    harness.app.open_sidebar(&mut harness.tui);
    harness.pump_until("both saved chats listed", |h| {
        h.app.workspace.saved_chats.len() == 2
    });
    harness.pump_until_screen_shows("Latest prompt");
    harness.app.close_side_tab(&mut harness.tui, saved, false);
    harness.pump_until("the saved chat to leave memory", |h| {
        h.app.sidebar_chats().is_empty()
    });

    harness.app.open_sidebar(&mut harness.tui);
    let screen = harness.pump_until_screen_shows("Latest prompt");
    let divider = harness
        .app
        .workspace
        .last_layout()
        .unwrap()
        .divider
        .unwrap()
        .x;
    let chooser = screen
        .lines()
        .map(|line| {
            line.chars()
                .skip(usize::from(divider + 1))
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        chooser.lines().any(|line| line.trim() == "Latest prompt"),
        "{chooser}"
    );
    assert!(!chooser.contains("Parent-only prompt"), "{chooser}");
}
