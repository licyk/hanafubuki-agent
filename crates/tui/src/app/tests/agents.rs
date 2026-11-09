//! `/agents` end to end over the standalone Agent: the sessions with their
//! subagents nested, the status filters, and the actions: open (a session,
//! a subagent beside its parent), new, fork, archive and restore, delete.

use hanafubuki_agent::test_support::assistant_events;
use hanafubuki_agent::{AgentSessionLifecycle, AgentSessionListRequest};
use ratatui::crossterm::event::KeyCode;

use super::subagents::spawn_and_wait;
use super::Harness;

impl Harness {
    fn top_level(&self, lifecycle: AgentSessionLifecycle) -> usize {
        let agent = self.runtime.slot().agent().unwrap();
        agent
            .list_sessions_page(AgentSessionListRequest {
                lifecycle,
                ..AgentSessionListRequest::default()
            })
            .unwrap()
            .sessions
            .len()
    }

    /// Moves the center's selection to the row that shows `text`.
    fn select_row(&mut self, text: &str) {
        self.key(KeyCode::Home);
        for _ in 0..10 {
            self.app.draw(&mut self.tui).unwrap();
            let screen = self.screen();
            if screen
                .lines()
                .any(|line| line.starts_with('›') && line.contains(text))
            {
                return;
            }
            self.key(KeyCode::Down);
        }
        panic!("no row shows {text:?}:\n{}", self.screen());
    }

    fn open_agents(&mut self) {
        self.type_and_submit("/agents");
        self.pump_until_screen_shows("1 All · 2 Running");
    }
}

#[test]
fn the_command_center_lists_sessions_with_their_subagents_and_acts_on_them() {
    let mut harness = Harness::start(vec![
        assistant_events("Hi."),
        spawn_and_wait("Count the files."),
        assistant_events("Twelve files."),
        assistant_events("There are twelve."),
    ]);
    let first = harness.answered("Hello.", "Hi.");
    harness.type_and_submit("/new");
    harness.pump_until("no session to be shown", |harness| {
        harness.app.bridge.session_id().is_none()
    });
    let parent = harness.answered("Delegate the count.", "There are twelve.");

    harness.open_agents();
    let screen = harness.screen();
    let rows: Vec<&str> = screen
        .lines()
        .filter(|line| line.contains("ago") || line.contains("just now"))
        .collect();
    assert_eq!(rows.len(), 3, "{screen}");
    assert!(rows[0].contains("Delegate the count.") && rows[0].contains("(shown)"));
    assert!(rows[1].contains("└ counter") && rows[1].contains("subagent"));
    assert!(rows[2].contains("Hello."));

    // The filters.
    harness.key(KeyCode::Char('2'));
    harness.pump_until_screen_shows("No sessions match this filter.");
    harness.key(KeyCode::Char('3'));
    harness.pump_until_screen_shows("└ counter");
    harness.key(KeyCode::Char('1'));

    // Enter on the subagent shows it beside its parent.
    harness.select_row("└ counter");
    harness.key(KeyCode::Enter);
    harness.pump_until_screen_shows("Subagent counter · from Delegate the count.");
    harness.key(KeyCode::Esc);
    harness.pump_until_screen_shows("Main conversation");
    assert_eq!(harness.app.bridge.session_id(), Some(parent));

    // Archived, listed under the archived filter, restored.
    harness.open_agents();
    harness.select_row("Hello.");
    harness.key(KeyCode::Char('a'));
    harness.pump_until_screen_shows("Session archived");
    assert_eq!(harness.top_level(AgentSessionLifecycle::Archived), 1);
    harness.key(KeyCode::Char('5'));
    harness.pump_until_screen_shows("Hello.");
    harness.key(KeyCode::Enter);
    harness.pump_until_screen_shows("Session restored");
    assert_eq!(harness.top_level(AgentSessionLifecycle::Archived), 0);

    // The session shown is not deleted from here; another is, once
    // confirmed.
    harness.key(KeyCode::Char('1'));
    harness.select_row("(shown)");
    harness.key(KeyCode::Char('d'));
    harness.pump_until_screen_shows("Delete the session and its children?");
    harness.key(KeyCode::Char('y'));
    harness.pump_until_screen_shows("Use /delete for the session shown.");
    harness.select_row("Hello.");
    harness.key(KeyCode::Char('d'));
    harness.key(KeyCode::Char('y'));
    harness.pump_until_screen_shows("Session deleted");
    assert_eq!(harness.top_level(AgentSessionLifecycle::Available), 1);
    let agent = harness.runtime.slot().agent().unwrap();
    assert!(agent.snapshot(first).is_err(), "the session is gone");

    // A fork of the session is shown in its place.
    harness.select_row("Delegate the count.");
    harness.key(KeyCode::Char('f'));
    harness.pump_until("the fork to be shown", |harness| {
        harness
            .app
            .bridge
            .session_id()
            .is_some_and(|shown| shown != parent)
    });
    assert_eq!(harness.top_level(AgentSessionLifecycle::Available), 2);

    // A new session: none until the next message.
    harness.open_agents();
    harness.key(KeyCode::Char('n'));
    harness.pump_until("no session to be shown", |harness| {
        harness.app.bridge.session_id().is_none()
    });
}
