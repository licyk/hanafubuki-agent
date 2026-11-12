//! The command center end to end over the standalone Agent: Left on an
//! empty composer opens it in both presentations; it lists the sessions
//! without a parent, a subagent's status rolled up into its parent's, and
//! acts on them: open, search, rename, archive and restore, delete (both
//! confirmed), fork, new; `g` is kept in `tui.agentsGrouping`.

use hanafubuki_agent::test_support::{assistant_events, tool_calls_events};
use hanafubuki_agent::{AgentSessionLifecycle, AgentSessionListRequest};
use ratatui::crossterm::event::KeyCode;

use super::subagents::spawn_and_wait;
use super::{command_arguments, Harness};
use crate::app::WorkspaceEvent;
use crate::app_event::AppEvent;

const TITLE: &str = "Agent command center";

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
        for _ in 0..10 {
            self.app.draw(&mut self.tui).unwrap();
            let screen = self.screen();
            if screen
                .lines()
                .any(|line| line.trim_start().starts_with('›') && line.contains(text))
            {
                return;
            }
            self.key(KeyCode::Down);
        }
        panic!("no row shows {text:?}:\n{}", self.screen());
    }

    fn open_agents(&mut self) {
        self.key(KeyCode::Left);
        self.pump_until_screen_shows(TITLE);
    }

    /// The center's session rows, as drawn.
    fn center_rows(&self) -> Vec<String> {
        self.screen()
            .lines()
            .filter(|line| line.contains(" ago") || line.contains("just now"))
            .map(str::to_string)
            .collect()
    }
}

#[test]
fn left_on_an_empty_composer_opens_the_command_center_in_both_presentations() {
    for mut harness in [Harness::start(Vec::new()), Harness::owned(Vec::new())] {
        harness.pump_until_screen_shows("← for agents");
        // A draft keeps Left for the cursor.
        harness.key(KeyCode::Char('x'));
        harness.key(KeyCode::Left);
        harness.app.draw(&mut harness.tui).unwrap();
        assert!(!harness.screen().contains(TITLE));
        harness.key(KeyCode::Backspace);
        harness.key(KeyCode::Right);
        harness.key(KeyCode::Backspace);

        harness.open_agents();
        harness.pump_until_screen_shows("No sessions yet.");
        harness.key(KeyCode::Esc);
        harness.pump_until("the center to close", |harness| {
            !harness.screen().contains(TITLE)
        });
    }
}

#[test]
fn the_command_center_lists_root_sessions_and_acts_on_them() {
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

    // The sessions without a parent, the shown one selected; the subagent
    // is no row of its own.
    harness.open_agents();
    let rows = harness.center_rows();
    assert_eq!(rows.len(), 2, "{}", harness.screen());
    assert!(rows[0].contains("› ") && rows[0].contains("Delegate the count. (shown)"));
    assert!(rows[0].contains("Ready"), "{}", rows[0]);
    assert!(rows[1].contains("Hello."));
    assert!(!harness.screen().contains("counter"));

    // The filters: Working has none, Ready both.
    harness.key(KeyCode::Char('3'));
    harness.pump_until_screen_shows("No sessions match.");
    harness.key(KeyCode::Char('4'));
    harness.pump_until("both under Ready", |harness| {
        harness.center_rows().len() == 2
    });
    harness.key(KeyCode::Char('1'));

    // A search by title.
    harness.key(KeyCode::Char('/'));
    for character in "Hel".chars() {
        harness.key(KeyCode::Char(character));
    }
    harness.pump_until("the search", |harness| harness.center_rows().len() == 1);
    harness.key(KeyCode::Esc);
    harness.pump_until("the whole list", |harness| harness.center_rows().len() == 2);

    // A rename of a session not shown.
    harness.select_row("Hello.");
    harness.key(KeyCode::Char('r'));
    for _ in 0.."Hello.".len() {
        harness.key(KeyCode::Backspace);
    }
    for character in "Greeting".chars() {
        harness.key(KeyCode::Char(character));
    }
    harness.key(KeyCode::Enter);
    harness.pump_until_screen_shows("Session renamed");
    harness.pump_until_screen_shows("Greeting");

    // Archived after its confirmation, restored under Archived.
    harness.select_row("Greeting");
    harness.key(KeyCode::Char('a'));
    harness.pump_until_screen_shows("Archive “Greeting”?");
    harness.key(KeyCode::Down);
    harness.key(KeyCode::Enter);
    harness.pump_until_screen_shows("Session archived");
    assert_eq!(harness.top_level(AgentSessionLifecycle::Archived), 1);
    harness.key(KeyCode::Char('6'));
    harness.pump_until_screen_shows("Greeting");
    harness.key(KeyCode::Enter);
    harness.pump_until_screen_shows("Session restored");
    assert_eq!(harness.top_level(AgentSessionLifecycle::Archived), 0);

    // The session shown is not deleted from here; another is, once
    // confirmed.
    harness.key(KeyCode::Char('1'));
    harness.select_row("(shown)");
    harness.key(KeyCode::Backspace);
    harness.pump_until_screen_shows("Use /delete for the session shown.");
    harness.select_row("Greeting");
    harness.key(KeyCode::Backspace);
    harness.pump_until_screen_shows("Permanently delete “Greeting”?");
    harness.key(KeyCode::Down);
    harness.key(KeyCode::Enter);
    harness.pump_until_screen_shows("Session deleted");
    assert_eq!(harness.top_level(AgentSessionLifecycle::Available), 1);
    let agent = harness.runtime.slot().agent().unwrap();
    assert!(agent.snapshot(first).is_err(), "the session is gone");

    // The grouping is kept, quietly.
    harness.key(KeyCode::Char('g'));
    harness.pump_until_screen_shows("Group: Directory");
    let config = harness.home.path().join("config.toml");
    harness.pump_until("the grouping to be written", |_| {
        std::fs::read_to_string(&config)
            .unwrap()
            .contains("agentsGrouping = \"directory\"")
    });

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

    // A new session: none until the next message. The center opens on the
    // grouping kept.
    harness.open_agents();
    harness.pump_until_screen_shows("Group: Directory");
    harness.key(KeyCode::Char('n'));
    harness.pump_until("no session to be shown", |harness| {
        harness.app.bridge.session_id().is_none()
    });

    // Enter (or Right) shows the session selected in place of the one shown.
    harness.open_agents();
    harness.select_row("Delegate the count.");
    harness.key(KeyCode::Right);
    harness.pump_until("the session to be shown", |harness| {
        harness.app.bridge.session_id().is_some()
    });
    assert!(!harness.screen().contains(TITLE));
}

#[test]
fn a_subagent_waiting_on_an_approval_puts_its_parent_under_needs_you() {
    let mut harness = Harness::start(vec![
        spawn_and_wait("Run the command."),
        tool_calls_events(&[(
            "call-child",
            "exec_command",
            &command_arguments("echo child-output"),
        )]),
        assistant_events("The command printed child-output."),
        assistant_events("The subagent ran it."),
    ]);
    harness.type_and_submit("Delegate the command.");
    harness.pump_until_screen_shows("Subagent counter: Run");

    let event = AppEvent::Workspace(WorkspaceEvent::OpenAgents);
    assert!(harness.app.handle_event(&mut harness.tui, event));
    let screen = harness.pump_until_screen_shows(TITLE);
    let rows = harness.center_rows();
    assert_eq!(rows.len(), 1, "{screen}");
    assert!(rows[0].contains("Needs input"), "{screen}");
    assert!(screen.contains("Needs you 1"), "{screen}");

    harness.key(KeyCode::Esc);
    harness.pump_until_screen_shows("Subagent counter: Run");
    harness.key(KeyCode::Char('y'));
    harness.pump_until_screen_shows("The subagent ran it.");
    harness.pump_until_turn_ends();
}
