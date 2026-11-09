//! The session commands end to end over the standalone Agent: what each
//! did to the sessions, read back through the facade.

use hanafubuki_agent::test_support::assistant_events;
use hanafubuki_agent::{AgentSessionId, AgentSessionLifecycle};
use ratatui::crossterm::event::{KeyCode, KeyModifiers};

use super::Harness;

const RECAP: &str =
    r#"{"summary":"Answered the question; nothing is pending.","next_action":"Ask another?"}"#;

impl Harness {
    /// Sends `text` and waits for the answer `answer` and the turn's end.
    fn exchange(&mut self, text: &str, answer: &str) -> AgentSessionId {
        self.type_and_submit(text);
        self.pump_until_screen_shows(answer);
        self.pump_until_turn_ends();
        self.app.bridge.session_id().unwrap()
    }

    fn title(&self, session_id: AgentSessionId) -> Option<String> {
        let agent = self.runtime.slot().agent().unwrap();
        let opened = agent.open_session(session_id, 1).unwrap();
        opened.runtime.title.map(|title| title.value)
    }

    fn lifecycle(&self, session_id: AgentSessionId) -> AgentSessionLifecycle {
        let agent = self.runtime.slot().agent().unwrap();
        agent.open_session(session_id, 1).unwrap().runtime.lifecycle
    }

    /// Picks "yes" in a confirmation, "no" being first.
    fn confirm(&mut self, question: &str) {
        self.pump_until_screen_shows(question);
        self.key(KeyCode::Down);
        self.key(KeyCode::Enter);
    }

    fn pump_until_no_session(&mut self) {
        self.pump_until("no session", |harness| {
            harness.app.bridge.session_id().is_none()
        });
    }
}

#[test]
fn rename_names_the_session_and_resume_finds_it_by_title_or_id() {
    let mut harness = Harness::start(vec![
        assistant_events("First answer."),
        assistant_events("Second answer."),
    ]);
    let first = harness.exchange("First question", "First answer.");
    harness.type_and_submit("/rename Login fix");
    harness.pump_until_screen_shows("Session renamed to \"Login fix\"");
    assert_eq!(harness.title(first).as_deref(), Some("Login fix"));

    harness.type_and_submit("/new");
    harness.pump_until_no_session();
    let second = harness.exchange("Second question", "Second answer.");
    assert_ne!(first, second);

    harness.type_and_submit("/resume Login fix");
    harness.pump_until("the session named", |harness| {
        harness.app.bridge.session_id() == Some(first)
    });
    harness.pump_until("its transcript", |harness| {
        harness.transcript().contains("First answer.")
    });

    harness.type_and_submit(&format!("/resume {}", second.value()));
    harness.pump_until("the session by its id", |harness| {
        harness.app.bridge.session_id() == Some(second)
    });

    // No session has this title: the picker searches for it.
    harness.type_and_submit("/resume nothing like it");
    let picker = harness.pump_until_screen_shows("Search: nothing like it");
    assert!(picker.contains("No results for your search"), "{picker}");
}

#[test]
fn the_rename_prompt_without_the_titles_plugin_says_there_is_no_suggestion() {
    let mut harness = Harness::start(vec![assistant_events("An answer.")]);
    let session = harness.exchange("A question", "An answer.");
    harness.type_and_submit("/rename");
    let screen = harness.pump_until_screen_shows("No title suggestion");
    // The prompt starts from the session's title, its first message.
    assert!(screen.contains("A question"), "{screen}");
    for _ in 0.."A question".len() {
        harness.key(KeyCode::Backspace);
    }
    for character in "Typed".chars() {
        harness.key(KeyCode::Char(character));
    }
    harness.key(KeyCode::Enter);
    harness.pump_until_screen_shows("Session renamed to \"Typed\"");
    assert_eq!(harness.title(session).as_deref(), Some("Typed"));
}

#[test]
fn archive_and_delete_leave_the_session_after_a_confirmation() {
    let mut harness = Harness::start(vec![
        assistant_events("Kept answer."),
        assistant_events("Gone answer."),
    ]);
    let archived = harness.exchange("Keep this", "Kept answer.");
    harness.type_and_submit("/archive");
    harness.confirm("Archive this session?");
    harness.pump_until_no_session();
    harness.pump_until_screen_shows("Session archived");
    assert_eq!(harness.lifecycle(archived), AgentSessionLifecycle::Archived);

    let deleted = harness.exchange("Drop this", "Gone answer.");
    harness.type_and_submit("/delete");
    harness.confirm("Delete this session?");
    harness.pump_until_no_session();
    harness.pump_until_screen_shows("Session deleted");
    let agent = harness.runtime.slot().agent().unwrap();
    assert!(agent.open_session(deleted, 1).is_err());
}

#[test]
fn the_picker_archives_with_ctrl_a_and_restores_from_the_archived_list() {
    let mut harness = Harness::start(vec![
        assistant_events("Old answer."),
        assistant_events("New answer."),
    ]);
    let old = harness.exchange("Old question", "Old answer.");
    harness.type_and_submit("/new");
    harness.pump_until_no_session();
    let current = harness.exchange("New question", "New answer.");

    harness.type_and_submit("/resume");
    harness.pump_until_screen_shows("Resume a previous session");
    // The latest first: the session shown is not archived from here.
    harness.key_with(KeyCode::Char('a'), KeyModifiers::CONTROL);
    harness.pump_until_screen_shows("Use /archive to archive the current session");
    assert_eq!(harness.lifecycle(current), AgentSessionLifecycle::Available);

    harness.key(KeyCode::Down);
    harness.key_with(KeyCode::Char('a'), KeyModifiers::CONTROL);
    harness.pump_until_screen_shows("Session archived");
    assert_eq!(harness.lifecycle(old), AgentSessionLifecycle::Archived);

    harness.key(KeyCode::Tab);
    let archived = harness.pump_until_screen_shows("Archived sessions");
    assert!(archived.contains("Old question"), "{archived}");
    harness.key(KeyCode::Enter);
    harness.pump_until_screen_shows("Session restored");
    assert_eq!(harness.lifecycle(old), AgentSessionLifecycle::Available);
}

#[test]
fn fork_shows_a_named_copy_and_the_original_stays() {
    let mut harness = Harness::start(vec![assistant_events("Shared answer.")]);
    let original = harness.exchange("Shared question", "Shared answer.");
    harness.type_and_submit("/fork Experiment");
    harness.pump_until_screen_shows("Forked into a new session, \"Experiment\"");
    let fork = harness.app.bridge.session_id().unwrap();
    assert_ne!(fork, original);
    assert_eq!(harness.title(fork).as_deref(), Some("Experiment"));
    assert!(harness.transcript().contains("Shared answer."));
    assert_eq!(harness.title(original).as_deref(), Some("Shared question"));
}

#[test]
fn export_saves_the_conversation_as_markdown_without_replacing_a_file() {
    let mut harness = Harness::start(vec![assistant_events("The answer is **42**.")]);
    let session = harness.exchange("What is the answer?", "The answer is");
    harness.type_and_submit("/export chat.md");
    harness.pump_until_screen_shows("Saved the conversation to");
    let path = harness.home.path().join("project").join("chat.md");
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        concat!(
            "# Hanafubuki Agent conversation\n",
            "\n## User\n\nWhat is the answer?\n",
            "\n## Assistant\n\nThe answer is **42**.\n",
        )
    );

    harness.type_and_submit("/export chat.md");
    harness.pump_until_screen_shows("/export:");
    assert!(harness.transcript().contains("was not created"));

    // The picker's file prompt offers the default name.
    harness.type_and_submit("/export");
    harness.pump_until_screen_shows("Export conversation");
    harness.key(KeyCode::Down);
    harness.key(KeyCode::Enter);
    let name = format!("hanafubuki-agent-session-{}.md", session.value());
    harness.pump_until_screen_shows(&name);
    harness.key(KeyCode::Enter);
    harness.pump_until("the default file", |harness| {
        harness.home.path().join("project").join(&name).exists()
    });
}

#[test]
fn pwd_ps_and_stop_report_the_workspace_and_the_terminals() {
    let mut harness = Harness::start(vec![assistant_events("Ready.")]);
    harness.type_and_submit("/pwd");
    let screen = harness.pump_until_screen_shows("Current working directory:");
    assert!(screen.contains("project"), "{screen}");

    harness.type_and_submit("/ps");
    harness.pump_until_screen_shows("No background terminals.");

    harness.exchange("Hello", "Ready.");
    harness.type_and_submit("/clean");
    harness.confirm("Stop all background terminals?");
    harness.pump_until_screen_shows("Stopped 0 background terminals");
}

#[test]
fn recap_writes_the_models_catch_up_after_a_waiting_line() {
    let mut harness = Harness::start(vec![assistant_events("Done."), assistant_events(RECAP)]);
    harness.exchange("Do the thing", "Done.");
    harness.type_and_submit("/recap");
    harness.pump_until_screen_shows("Writing a recap…");
    let screen = harness.pump_until_screen_shows("Answered the question");
    assert!(screen.contains("Recap:"), "{screen}");
    assert!(screen.contains("Next: Ask another?"), "{screen}");
}

#[test]
fn clear_starts_over_and_names_the_next_session() {
    let mut harness = Harness::start(vec![
        assistant_events("Before."),
        assistant_events("After."),
    ]);
    harness.exchange("First", "Before.");
    harness.type_and_submit("/clear Second round");
    harness.pump_until_no_session();
    assert!(!harness.transcript().contains("Before."));
    let session = harness.exchange("Again", "After.");
    assert_eq!(harness.title(session).as_deref(), Some("Second round"));
}

#[test]
fn init_submits_the_agents_md_prompt_and_copy_offers_the_last_answer() {
    let mut harness = Harness::owned(vec![assistant_events(
        "Wrote it:\n\n```sh\ncat AGENTS.md\n```\n",
    )]);
    harness.type_and_submit("/init");
    harness.pump_until_screen_shows("cat AGENTS.md");
    harness.pump_until_turn_ends();
    let request = String::from_utf8_lossy(&harness.server.requests()[0].body).into_owned();
    assert!(
        request.contains("Generate a file named AGENTS.md"),
        "{request}"
    );

    harness.type_and_submit("/copy");
    let picker = harness.pump_until_screen_shows("Copy to clipboard");
    for choice in ["Whole answer", "sh code", "Select in the transcript"] {
        assert!(picker.contains(choice), "{choice}: {picker}");
    }
}
