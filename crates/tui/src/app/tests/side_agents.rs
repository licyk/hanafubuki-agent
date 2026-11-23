//! Agent navigation keeps the independent side conversation alive.

use std::time::Duration;

use hanafubuki_agent::test_support::assistant_events;
use ratatui::crossterm::event::{KeyCode, KeyModifiers};

use super::{subagents::spawn_and_wait, Harness};
use crate::transcript_mode::TranscriptMode;

#[test]
fn scrollback_chooser_docks_a_saved_chat_and_a_read_only_subagent() {
    let mut h = Harness::start(vec![
        spawn_and_wait("Count the files."),
        assistant_events("Twelve files."),
        assistant_events("There are twelve."),
    ]);
    let parent = h.answered("Delegate the count.", "There are twelve.");
    let child = h.subagents_of(parent)[0];
    h.type_and_submit("/sidebar");
    h.pump_until_screen_shows("New side chat");
    h.key(KeyCode::Enter);
    h.pump_until("new side chat docked", |h| {
        h.app.active_sidebar_chat().is_some()
    });
    let side = h.app.active_sidebar_chat().unwrap();
    assert!(h.tui.terminal.backend().vt100().screen().alternate_screen());
    assert!(h.app.workspace.last_layout().unwrap().strip.is_some());
    h.key(KeyCode::Esc);
    h.pump_until_screen_shows("Subagents");
    h.key(KeyCode::Home);
    h.key(KeyCode::Down);
    h.key(KeyCode::Down);
    h.key(KeyCode::Down);
    h.key(KeyCode::Enter);
    h.pump_until("subagent docked", |h| {
        h.app.active_sidebar_agent() == Some(child)
    });
    assert!(h.tui.terminal.backend().vt100().screen().alternate_screen());
    h.pump_until_screen_shows("Watch only");
    h.key(KeyCode::Char('x'));
    assert!(h.app.chat_widget.composer_is_empty());
    h.key(KeyCode::Char('/'));
    h.key(KeyCode::Char('f'));
    assert!(h.app.transcript_view.is_search_editing());
    h.key(KeyCode::Esc);
    assert_eq!(h.app.active_sidebar_agent(), Some(child));
    h.key(KeyCode::Esc);
    h.pump_until_screen_shows("Open in sidebar");
    h.key(KeyCode::Home);
    h.key(KeyCode::Down);
    h.key(KeyCode::Enter);
    h.pump_until("native main transcript", |h| {
        h.app.bridge.session_id() == Some(parent)
            && !h.tui.terminal.backend().vt100().screen().alternate_screen()
    });
    h.type_and_submit("/sidebar");
    h.pump_until_screen_shows("Side chats");
    h.key(KeyCode::Home);
    h.key(KeyCode::Down);
    h.key(KeyCode::Down);
    h.key(KeyCode::Enter);
    h.pump_until("saved chat refocused", |h| {
        h.app.bridge.session_id() == Some(side) && h.app.workspace.second_focused()
    });
}

#[test]
fn the_parent_keeps_streaming_with_a_child_selected_and_side_focused() {
    for mode in [TranscriptMode::Terminal, TranscriptMode::Owned] {
        let mut harness = Harness::start_timed(
            vec![
                (spawn_and_wait("Count the files."), Duration::ZERO),
                (assistant_events("Twelve files."), Duration::ZERO),
                (assistant_events("There are twelve."), Duration::ZERO),
                (
                    assistant_events("Parent continued."),
                    Duration::from_secs(1),
                ),
            ],
            true,
            mode,
        );
        let parent = harness.answered("Delegate the count.", "There are twelve.");
        let child = harness.subagents_of(parent)[0];
        harness.type_and_submit("Continue in parent.");
        harness.pump_until("parent running", |h| h.app.chat_widget.is_task_running());
        harness.type_and_submit("/side");
        harness.pump_until("side opened", |h| h.app.workspace.second_focused());
        let side = harness.app.bridge.session_id();
        harness.key_with(KeyCode::Right, KeyModifiers::ALT);
        harness.pump_until("child selected", |h| {
            h.app.bridge.session_id() == Some(child)
        });
        harness.key_with(KeyCode::Char('/'), KeyModifiers::CONTROL);
        assert_eq!(harness.app.bridge.session_id(), side);
        harness.pump_until("parked parent finished", |h| {
            h.app
                .workspace
                .parent()
                .is_some_and(|parent| !parent.chat_widget.is_task_running())
        });
        assert!(!harness.transcript().contains("Parent continued."));
        harness.key_with(KeyCode::Left, KeyModifiers::ALT);
        harness.pump_until("parent restored with its answer", |h| {
            h.app.bridge.session_id() == Some(parent)
                && h.transcript().contains("Parent continued.")
        });
        harness.key_with(KeyCode::Char('/'), KeyModifiers::CONTROL);
        assert_eq!(harness.app.bridge.session_id(), side);
    }
}

#[test]
fn side_focus_and_agent_navigation_keep_drafts_and_route_running_replies() {
    for mode in [TranscriptMode::Terminal, TranscriptMode::Owned] {
        let mut harness = Harness::start_timed(
            vec![
                (spawn_and_wait("Count the files."), Duration::ZERO),
                (assistant_events("Twelve files."), Duration::ZERO),
                (assistant_events("There are twelve."), Duration::ZERO),
                (assistant_events("Side answer."), Duration::from_secs(1)),
                (assistant_events("Child follow-up."), Duration::ZERO),
            ],
            true,
            mode,
        );
        let parent = harness.answered("Delegate the count.", "There are twelve.");
        let child = harness.subagents_of(parent)[0];
        harness.type_and_submit("/side");
        harness.pump_until("side opened", |h| h.app.workspace.second_focused());
        let side = harness.app.bridge.session_id();
        for c in "side draft".chars() {
            harness.key(KeyCode::Char(c));
        }
        harness.pump_until("side draft typed", |h| {
            h.app.chat_widget.composer_text() == "side draft"
        });
        harness.key_with(KeyCode::Char('/'), KeyModifiers::CONTROL);
        assert_eq!(harness.app.bridge.session_id(), Some(parent));
        for c in "parent draft".chars() {
            harness.key(KeyCode::Char(c));
        }
        harness.pump_until("parent draft typed", |h| {
            h.app.chat_widget.composer_text() == "parent draft"
        });
        harness.key_with(KeyCode::Char('/'), KeyModifiers::CONTROL);
        assert_eq!(harness.app.bridge.session_id(), side);
        assert_eq!(harness.app.chat_widget.composer_text(), "side draft");
        harness.key_with(KeyCode::Char('u'), KeyModifiers::CONTROL);
        harness.type_and_submit("Side question.");
        harness.pump_until("side request sent", |h| h.server.requests().len() == 4);

        harness.key_with(KeyCode::Right, KeyModifiers::ALT);
        harness.pump_until("child shown beside side", |h| {
            h.app.bridge.session_id() == Some(child)
        });
        assert!(!harness.app.workspace.second_focused());
        if mode.is_owned() {
            assert!(harness
                .app
                .workspace
                .last_layout()
                .unwrap()
                .passive
                .is_some());
        }
        assert!(!harness
            .transcript()
            .contains("Close the side conversation first"));
        harness.type_and_submit("Follow up.");
        harness.pump_until_screen_shows("Child follow-up.");
        harness.pump_until_turn_ends();
        assert!(!harness.transcript().contains("Side answer."));
        harness.key_with(KeyCode::Char('/'), KeyModifiers::CONTROL);
        assert_eq!(harness.app.bridge.session_id(), side);
        harness.pump_until_screen_shows("Side answer.");
        harness.pump_until_turn_ends();
        assert!(!harness.transcript().contains("Child follow-up."));
        harness.key_with(KeyCode::Char('/'), KeyModifiers::CONTROL);
        assert_eq!(harness.app.bridge.session_id(), Some(child));
        harness.key_with(KeyCode::Left, KeyModifiers::ALT);
        harness.pump_until("parent restored", |h| {
            h.app.bridge.session_id() == Some(parent)
        });
        assert_eq!(harness.app.chat_widget.composer_text(), "parent draft");
        harness.key_with(KeyCode::Char('/'), KeyModifiers::CONTROL);
        assert_eq!(harness.app.bridge.session_id(), side);
        harness.type_and_submit("/close");
        harness.pump_until_screen_shows("Side conversation closed.");
        assert_eq!(harness.app.bridge.session_id(), Some(parent));
        assert_eq!(harness.app.chat_widget.composer_text(), "parent draft");
    }
}
