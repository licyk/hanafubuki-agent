//! Editing an earlier message, end to end: Esc twice browses the prompts,
//! Enter forks the session before the chosen one, archives the original
//! and puts the prompt back in the composer.

use std::time::Duration;

use hanafubuki_agent::test_support::assistant_events;
use hanafubuki_agent::{
    AgentSessionId, AgentSessionLifecycle, AgentSessionListRequest, ConversationItem,
};
use ratatui::crossterm::event::KeyCode;

use super::Harness;
use crate::transcript_mode::TranscriptMode;

impl Harness {
    /// Two exchanges, each answered and ended.
    fn two_exchanges(&mut self) {
        for (question, answer) in [("First?", "First answer."), ("Second?", "Second answer.")] {
            self.type_and_submit(question);
            self.pump_until_screen_shows(answer);
            self.pump_until_turn_ends();
        }
    }

    /// The text drawn reversed: the highlighted prompt.
    fn highlighted(&self) -> String {
        let screen = self.tui.terminal.backend().vt100().screen();
        let (rows, columns) = screen.size();
        let mut text = String::new();
        for row in 0..rows {
            for column in 0..columns {
                if let Some(cell) = screen.cell(row, column).filter(|cell| cell.inverse()) {
                    text.push_str(cell.contents());
                }
            }
        }
        text
    }

    fn sessions(&self, lifecycle: AgentSessionLifecycle) -> Vec<AgentSessionId> {
        let agent = self.runtime.slot().agent().unwrap();
        agent
            .list_sessions_page(AgentSessionListRequest {
                lifecycle,
                ..AgentSessionListRequest::default()
            })
            .unwrap()
            .sessions
            .into_iter()
            .map(|summary| summary.session_id)
            .collect()
    }

    /// The messages a session holds, oldest first.
    fn saved_messages(&self, session_id: AgentSessionId) -> Vec<String> {
        let agent = self.runtime.slot().agent().unwrap();
        let mut messages: Vec<String> = agent
            .open_session(session_id, 50)
            .unwrap()
            .transcript
            .items
            .into_iter()
            .filter_map(|item| match item.content {
                ConversationItem::UserMessage { text, .. } => Some(text),
                _ => None,
            })
            .collect();
        messages.reverse();
        messages
    }
}

#[test]
fn a_first_esc_primes_and_any_other_key_cancels() {
    let mut harness = Harness::start(vec![assistant_events("Done.")]);
    harness.pump_until_screen_shows("fixture-model");
    // Nothing to edit yet: the second Esc says so.
    harness.key(KeyCode::Esc);
    harness.app.draw(&mut harness.tui).unwrap();
    assert!(!harness.screen().contains("again to edit"));
    harness.key(KeyCode::Esc);
    harness.pump_until_screen_shows("No previous message to edit.");
    assert!(!harness.app.backtrack.is_primed());

    harness.type_and_submit("Hello?");
    harness.pump_until_screen_shows("Done.");
    harness.pump_until_turn_ends();
    harness.key(KeyCode::Esc);
    harness.pump_until_screen_shows("esc again to edit a previous message");
    assert!(harness.app.backtrack.is_primed());
    harness.key(KeyCode::Left);
    harness.app.draw(&mut harness.tui).unwrap();
    assert!(!harness.screen().contains("again to edit"));
    assert!(!harness.app.backtrack.is_primed());
    // A cancelled first Esc does not count towards the second.
    harness.key(KeyCode::Esc);
    assert!(!harness.app.backtrack.is_browsing());
}

#[test]
fn esc_while_a_turn_runs_interrupts_and_does_not_prime() {
    let mut harness = Harness::start_timed(
        vec![(assistant_events("Too late."), Duration::from_secs(30))],
        true,
        TranscriptMode::Terminal,
    );
    harness.type_and_submit("Take your time.");
    harness.pump_until("the turn to run", |harness| {
        harness.app.chat_widget.is_task_running()
    });
    harness.key(KeyCode::Esc);
    assert!(!harness.app.backtrack.is_primed());
    harness.pump_until_screen_shows("Conversation interrupted");
}

#[test]
fn browsing_in_the_owned_transcript_moves_between_prompts_and_edits_in_a_fork() {
    let mut harness = Harness::owned(vec![
        assistant_events("First answer."),
        assistant_events("Second answer."),
    ]);
    harness.two_exchanges();
    let original = harness.app.bridge.session_id().unwrap();

    harness.key(KeyCode::Esc);
    harness.key(KeyCode::Esc);
    let screen = harness.pump_until_screen_shows("Browsing");
    assert!(screen.contains("←/→ prompt"), "{screen}");
    assert!(harness.highlighted().contains("Second?"));
    harness.key(KeyCode::Left);
    harness.app.draw(&mut harness.tui).unwrap();
    assert!(harness.highlighted().contains("First?"));
    // There is none before the first.
    harness.key(KeyCode::Char('h'));
    harness.app.draw(&mut harness.tui).unwrap();
    assert!(harness.highlighted().contains("First?"));
    harness.key(KeyCode::Right);
    harness.app.draw(&mut harness.tui).unwrap();
    assert!(harness.highlighted().contains("Second?"));

    // Esc goes back, nothing edited.
    harness.key(KeyCode::Esc);
    harness.app.draw(&mut harness.tui).unwrap();
    assert!(!harness.screen().contains("Browsing"));
    assert!(!harness.highlighted().contains("Second?"));
    assert_eq!(harness.app.bridge.session_id(), Some(original));

    harness.key(KeyCode::Esc);
    harness.key(KeyCode::Esc);
    harness.key(KeyCode::Enter);
    harness.pump_until_screen_shows("the original is archived");
    let fork = harness.app.bridge.session_id().unwrap();
    assert_ne!(fork, original);
    // The fork's transcript ends before the prompt, which is back in the
    // composer.
    let transcript = harness.transcript();
    assert!(transcript.contains("First answer."), "{transcript}");
    assert!(!transcript.contains("Second answer."), "{transcript}");
    let composer = harness.bottom_rows(4);
    assert!(composer.contains("Second?"), "{composer}");
    assert_eq!(
        harness.sessions(AgentSessionLifecycle::Archived),
        vec![original]
    );
}

#[test]
fn browsing_in_the_terminal_overlay_edits_and_the_fork_goes_on() {
    let mut harness = Harness::start(vec![
        assistant_events("First answer."),
        assistant_events("Second answer."),
        assistant_events("Edited answer."),
    ]);
    harness.two_exchanges();
    let original = harness.app.bridge.session_id().unwrap();

    harness.key(KeyCode::Esc);
    harness.key(KeyCode::Esc);
    harness.pump_until_screen_shows("T R A N S C R I P T");
    assert!(harness.screen().contains("Browsing"));
    assert!(harness.highlighted().contains("Second?"));
    harness.key(KeyCode::Left);
    harness.app.draw(&mut harness.tui).unwrap();
    assert!(harness.highlighted().contains("First?"));
    harness.key(KeyCode::Enter);
    harness.pump_until("the fork", |harness| {
        harness
            .app
            .bridge
            .session_id()
            .is_some_and(|id| id != original)
    });
    let screen = harness.pump_until_screen_shows("the original is archived");
    assert!(!screen.contains("T R A N S C R I P T"), "{screen}");
    let composer = harness.bottom_rows(4);
    assert!(composer.contains("First?"), "{composer}");

    // The prompt, edited, starts the fork's first turn.
    harness.type_and_submit(" Again.");
    harness.pump_until_screen_shows("Edited answer.");
    harness.pump_until_turn_ends();
    let fork = harness.app.bridge.session_id().unwrap();
    assert_eq!(harness.saved_messages(fork), vec!["First? Again."]);
    assert_eq!(
        harness.sessions(AgentSessionLifecycle::Available),
        vec![fork]
    );
    assert_eq!(
        harness.sessions(AgentSessionLifecycle::Archived),
        vec![original]
    );
    assert_eq!(harness.saved_messages(original), vec!["First?", "Second?"]);
}
