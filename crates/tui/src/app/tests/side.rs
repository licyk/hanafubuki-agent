//! `/side` end to end over the standalone Agent: refused before the first
//! message and while a review runs; opened (in scrollback mode, a switch
//! to it), its messages sent to the side session, left with Esc, shown
//! again with `/side`, and closed; in fullscreen, a pane beside the
//! transcript that the leader's keys and the pointer focus.

use std::time::Duration;

use hanafubuki_agent::test_support::assistant_events;
use hanafubuki_agent::{AgentSessionId, AgentSessionOrigin};
use ratatui::crossterm::event::{KeyCode, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};

use super::Harness;
use crate::app_event::AppEvent;
use crate::transcript_mode::TranscriptMode;

impl Harness {
    fn session(&self) -> AgentSessionId {
        self.app.bridge.session_id().unwrap()
    }

    fn origin(&self, session_id: AgentSessionId) -> Option<AgentSessionOrigin> {
        let agent = self.runtime.slot().agent().unwrap();
        agent
            .snapshot(session_id)
            .ok()
            .and_then(|snapshot| snapshot.origin)
    }

    fn snapshot_screen(&self) -> String {
        self.screen()
            .lines()
            .map(str::trim_end)
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn leader(&mut self, code: KeyCode) {
        self.key_with(KeyCode::Char('x'), KeyModifiers::CONTROL);
        self.key(code);
    }

    fn click(&mut self, column: u16, row: u16) {
        let event = |kind| {
            AppEvent::Mouse(MouseEvent {
                kind,
                column,
                row,
                modifiers: KeyModifiers::NONE,
            })
        };
        assert!(self.app.handle_event(
            &mut self.tui,
            event(MouseEventKind::Down(MouseButton::Left))
        ));
        assert!(self
            .app
            .handle_event(&mut self.tui, event(MouseEventKind::Up(MouseButton::Left))));
    }
}

#[test]
fn a_side_conversation_opens_while_the_first_turn_still_runs() {
    let mut harness = Harness::start_timed(
        vec![(assistant_events("Hi."), Duration::from_secs(2))],
        true,
        TranscriptMode::Terminal,
    );
    harness.type_and_submit("Hello.");
    harness.pump_until("the first turn's request", |harness| {
        harness.server.requests().len() == 1
    });

    // The conversation has started and its answer has not come: the side
    // conversation opens beside it.
    harness.type_and_submit("/side");
    let screen = harness.pump_until_screen_shows("Side conversation · fixture-model");
    assert!(
        !screen.contains("unavailable until the current conversation"),
        "{screen}"
    );
    let side = harness.session();
    assert_eq!(harness.origin(side), Some(AgentSessionOrigin::Side));
    harness.pump_until_turn_ends();
    harness.type_and_submit("/close");
    harness.pump_until_screen_shows("Side conversation closed.");
    assert_eq!(harness.origin(side), None);
}

#[test]
fn a_side_conversation_takes_its_own_messages_and_esc_returns_to_the_main_one() {
    let mut harness = Harness::start(vec![
        assistant_events("Hi."),
        assistant_events("Four."),
        assistant_events("Five."),
    ]);
    harness.type_and_submit("/side");
    harness.pump_until_screen_shows("'/side' is unavailable until the current conversation");

    let parent = harness.answered("Hello.", "Hi.");
    harness.type_and_submit("/btw What is 2 + 2?");
    harness.pump_until_screen_shows("Four.");
    harness.pump_until_turn_ends();
    let side = harness.session();
    assert_ne!(side, parent);
    assert_eq!(harness.app.main_session_id(), Some(parent));
    assert_eq!(harness.origin(side), Some(AgentSessionOrigin::Side));
    assert!(harness
        .screen()
        .contains("Side conversation · ctrl-x x or /close closes · Esc returns"));
    let request = String::from_utf8(harness.server.requests()[1].body.clone()).unwrap();
    assert!(request.contains("What is 2 + 2?"));
    assert!(
        request.contains("Hello."),
        "the side reads the main history"
    );

    // Only its commands run there.
    harness.type_and_submit("/model");
    harness.pump_until_screen_shows("'/model' is unavailable here");

    harness.key(KeyCode::Esc);
    harness.pump_until_screen_shows("Main conversation");
    assert_eq!(harness.session(), parent);
    assert!(!harness.transcript().contains("Four."));

    // `/side` shows the same side conversation again, and sends there.
    harness.type_and_submit("/side And 2 + 3?");
    harness.pump_until_screen_shows("Five.");
    harness.pump_until_turn_ends();
    assert_eq!(harness.session(), side);

    harness.type_and_submit("/close");
    harness.pump_until_screen_shows("Side conversation closed.");
    assert_eq!(harness.session(), parent);
    assert_eq!(harness.origin(side), None, "the side session is gone");
    assert_eq!(harness.server.requests().len(), 3);
}

#[test]
fn hidden_side_chats_finish_in_their_own_transcript_and_restore_their_drafts() {
    let mut harness = Harness::start_timed(
        vec![
            (assistant_events("Main answer."), Duration::ZERO),
            (
                assistant_events("Saved answer."),
                Duration::from_millis(400),
            ),
        ],
        true,
        TranscriptMode::Terminal,
    );
    let parent = harness.answered("Parent message.", "Main answer.");
    harness.app.new_side_chat(&mut harness.tui, true);
    harness.pump_until("the saved chat to open", |h| {
        h.app.active_sidebar_chat().is_some()
    });
    let saved = harness.session();
    harness.type_and_submit("Saved question.");
    harness.pump_until("the saved turn to begin", |h| {
        h.server.requests().len() == 2
    });
    harness
        .app
        .chat_widget
        .set_composer_text("saved draft".into());
    harness.app.new_side_chat(&mut harness.tui, false);
    harness.pump_until("a second chat", |h| {
        h.app.active_sidebar_chat().is_some_and(|id| id != saved)
    });
    let temporary = harness.session();
    harness
        .app
        .chat_widget
        .set_composer_text("other draft".into());
    harness.pump_until("the hidden answer to finish", |h| {
        h.runtime
            .slot()
            .agent()
            .unwrap()
            .snapshot(saved)
            .unwrap()
            .turn
            .is_some_and(|turn| turn.phase.is_terminal())
    });
    assert!(!harness.transcript().contains("Saved answer."));
    harness.app.select_side_chat(&mut harness.tui, saved);
    harness.pump_until("saved answer to render", |h| {
        h.transcript().contains("Saved answer.")
    });
    assert_eq!(harness.app.chat_widget.composer_text(), "saved draft");
    harness.app.close_side_tab(&mut harness.tui, saved, false);
    harness.pump_until("saved tab to close", |h| {
        h.app.active_sidebar_chat().is_none()
    });
    harness.app.select_side_chat(&mut harness.tui, saved);
    harness.pump_until("persisted answer to reopen", |h| {
        h.app.active_sidebar_chat() == Some(saved) && h.transcript().contains("Saved answer.")
    });
    assert!(!harness.transcript().contains("Parent message."));
    assert!(harness.app.chat_widget.composer_is_empty());
    harness.app.select_side_chat(&mut harness.tui, temporary);
    assert_eq!(harness.app.chat_widget.composer_text(), "other draft");
    harness.app.close_side_tab(&mut harness.tui, saved, true);
    harness.pump_until("saved chat to be archived", |h| {
        h.runtime
            .slot()
            .agent()
            .unwrap()
            .saved_side_conversations(parent)
            .unwrap()
            .is_empty()
    });
    assert_eq!(harness.app.active_sidebar_chat(), Some(temporary));
}

#[test]
fn no_side_conversation_opens_while_a_review_runs() {
    let mut harness = Harness::start_timed(
        vec![
            (assistant_events("Hi."), Duration::ZERO),
            (assistant_events("{}"), Duration::from_secs(30)),
        ],
        true,
        TranscriptMode::Terminal,
    );
    harness.answered("Hello.", "Hi.");
    harness.type_and_submit("/review look at the tests");
    harness.pump_until("the review to sample", |harness| {
        harness.server.requests().len() == 2
    });
    harness.type_and_submit("/side Quick one?");
    harness.pump_until_screen_shows("'/side' is unavailable while a review runs");
    harness.key(KeyCode::Esc);
}

#[test]
fn in_fullscreen_the_side_conversation_is_a_pane_the_leader_and_pointer_focus() {
    let mut harness = Harness::owned(vec![assistant_events("Hi."), assistant_events("Four.")]);
    let parent = harness.answered("Hello.", "Hi.");
    harness.type_and_submit("/side What is 2 + 2?");
    harness.pump_until_screen_shows("Four.");
    harness.pump_until_turn_ends();
    let side = harness.session();
    let screen = harness.screen();
    insta::assert_snapshot!("side_pane_beside_the_transcript", harness.snapshot_screen());
    // Both conversations are on the screen, each under its title.
    assert!(screen.contains("Main conversation"), "{screen}");
    assert!(
        screen.contains("Side · ctrl-x x or /close closes"),
        "{screen}"
    );
    assert!(screen.contains("Hi."), "{screen}");
    let divider = harness
        .app
        .workspace
        .last_layout()
        .unwrap()
        .divider
        .unwrap();
    assert_eq!(divider.x, 63, "the side pane takes its narrowest width");

    // Ctrl+X shows its keys; ← focuses the main conversation.
    harness.key_with(KeyCode::Char('x'), KeyModifiers::CONTROL);
    harness.pump_until_screen_shows("main conversation");
    insta::assert_snapshot!("leader_keys_over_the_side_pane", harness.snapshot_screen());
    harness.key(KeyCode::Left);
    harness.pump_until_screen_shows("Main conversation · ctrl-x right side");
    assert_eq!(harness.session(), parent);
    assert!(harness.screen().contains("Four."), "the side pane stays");

    // The divider moves left, the side pane grows.
    harness.leader(KeyCode::Char('<'));
    harness.pump_until("the divider to move", |harness| {
        harness
            .app
            .workspace
            .last_layout()
            .and_then(|layout| layout.divider)
            .is_some_and(|divider| divider.x == 59)
    });

    // A click in the side pane focuses it.
    harness.click(90, 10);
    harness.pump_until("the side conversation to have the focus", |harness| {
        harness.app.bridge.session_id() == Some(side)
    });
    harness.leader(KeyCode::Char('z'));
    harness.pump_until("the side pane to fill the screen", |harness| {
        harness
            .app
            .workspace
            .last_layout()
            .is_some_and(|layout| layout.divider.is_none())
    });
    harness.leader(KeyCode::Char('x'));
    harness.pump_until_screen_shows("Side conversation closed.");
    assert_eq!(harness.session(), parent);
    assert!(!harness.screen().contains("Main conversation"));
}

#[test]
fn the_leader_closes_only_the_side_view_and_the_main_agent_can_continue() {
    let mut harness = Harness::start(vec![
        assistant_events("Main answer."),
        assistant_events("Side answer."),
        assistant_events("Main continued."),
    ]);
    let parent = harness.answered("Main question", "Main answer.");
    harness.type_and_submit("/btw Side question");
    harness.pump_until_screen_shows("Side answer.");
    harness.pump_until_turn_ends();
    assert_ne!(harness.session(), parent);
    // Ctrl-C belongs to the transcript's copy and cancel keys; the side
    // conversation closes on `hide_pane`.
    harness.key_with(KeyCode::Char('c'), KeyModifiers::CONTROL);
    assert_ne!(
        harness.session(),
        parent,
        "Ctrl-C leaves the side view open"
    );
    harness.leader(KeyCode::Char('x'));
    harness.pump_until_screen_shows("Side conversation closed.");
    assert_eq!(harness.session(), parent);
    assert_eq!(harness.answered("Continue main", "Main continued."), parent);
}
