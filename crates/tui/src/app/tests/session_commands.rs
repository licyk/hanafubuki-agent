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
    pub(super) fn exchange(&mut self, text: &str, answer: &str) -> AgentSessionId {
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
    pub(super) fn confirm(&mut self, question: &str) {
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
fn a_namespaced_tool_keeps_its_name_when_the_session_is_resumed() {
    let mut harness = Harness::start(vec![
        hanafubuki_agent::test_support::namespace_tool_events("remote", "read", "{}"),
        assistant_events("The tool is unavailable."),
    ]);
    let session = harness.exchange("Read the remote file.", "The tool is unavailable.");
    assert!(harness.transcript().contains("Failed remote__read"));
    harness.type_and_submit("/new");
    harness.pump_until_no_session();
    harness.type_and_submit(&format!("/resume {}", session.value()));
    harness.pump_until("the complete replayed tool name", |harness| {
        harness.app.bridge.session_id() == Some(session)
            && harness.transcript().contains("Failed remote__read")
    });
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
    // The project is a Git repository: the fork may run in a new worktree.
    harness.pump_until_screen_shows("Where should the fork run?");
    harness.key(KeyCode::Enter);
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

#[test]
fn permissions_survive_new_clear_and_resume_without_changing_defaults() {
    use crate::agent_bridge::SessionCall;
    use hanafubuki_agent::{AgentPermissionMode, AgentSandboxMode};

    let mut harness = Harness::start(vec![
        assistant_events("First."),
        assistant_events("Second."),
        assistant_events("Third."),
        assistant_events("Fresh."),
    ]);
    let first = harness.exchange("First", "First.");
    harness.app.bridge.submit(crate::app_event::Op::Command(
        crate::app_event::AgentCommand::Session(SessionCall::SetPreset(
            AgentPermissionMode::FullAccess,
            AgentSandboxMode::FullAccess,
        )),
    ));
    let agent = harness.runtime.slot().agent().unwrap();
    for (command, prompt, answer) in [("/new", "Second", "Second."), ("/clear", "Third", "Third.")]
    {
        harness.type_and_submit(command);
        harness.pump_until_no_session();
        let id = harness.exchange(prompt, answer);
        assert_eq!(
            agent.session_permission_mode(id).unwrap(),
            Some(AgentPermissionMode::FullAccess)
        );
        assert_eq!(
            agent.session_sandbox_mode(id).unwrap(),
            Some(AgentSandboxMode::FullAccess)
        );
    }
    harness.type_and_submit(&format!("/resume {}", first.value()));
    harness.pump_until("resumed", |h| h.app.bridge.session_id() == Some(first));
    assert_eq!(
        agent.session_permission_mode(first).unwrap(),
        Some(AgentPermissionMode::FullAccess)
    );
    assert_eq!(
        agent.session_sandbox_mode(first).unwrap(),
        Some(AgentSandboxMode::FullAccess)
    );
    drop(agent);
    let mut restarted = harness.restart();
    let fresh = restarted.exchange("Fresh", "Fresh.");
    let agent = restarted.runtime.slot().agent().unwrap();
    assert_eq!(agent.session_permission_mode(fresh).unwrap(), None);
    assert_eq!(agent.session_sandbox_mode(fresh).unwrap(), None);
    restarted.type_and_submit(&format!("/resume {}", first.value()));
    restarted.pump_until("resumed after restart", |h| {
        h.app.bridge.session_id() == Some(first)
    });
    assert_eq!(
        agent.session_permission_mode(first).unwrap(),
        Some(AgentPermissionMode::FullAccess)
    );
    assert_eq!(
        agent.session_sandbox_mode(first).unwrap(),
        Some(AgentSandboxMode::FullAccess)
    );
}

#[test]
fn plan_tool_renders_once_with_its_steps_live_and_after_resume() {
    let mut harness = Harness::start(vec![
        hanafubuki_agent::test_support::tool_events(
            "update_plan",
            r#"{"explanation":"Track the work","plan":[{"step":"Verify the fix","status":"completed"},{"step":"Second step","status":"pending"},{"step":"Third step","status":"pending"},{"step":"Fourth step","status":"pending"},{"step":"Fifth step","status":"pending"},{"step":"Hidden sixth step","status":"pending"}]}"#,
        ),
        assistant_events("Verified."),
    ]);
    let session = harness.exchange("Check the plan", "Verified.");
    for resumed in [false, true] {
        if resumed {
            harness.type_and_submit("/new");
            harness.pump_until_no_session();
            harness.type_and_submit(&format!("/resume {}", session.value()));
            harness.pump_until("replayed plan", |h| {
                h.app.bridge.session_id() == Some(session)
                    && h.transcript().contains("Verify the fix")
            });
        }
        let transcript = harness.transcript();
        assert_eq!(
            transcript.matches("Updated Plan").count(),
            1,
            "{transcript}"
        );
        assert!(transcript.contains("1/6 complete"), "{transcript}");
        assert!(transcript.contains("Verify the fix"), "{transcript}");
        assert!(!transcript.contains("Called update_plan"), "{transcript}");
        let screen = harness.screen();
        assert!(screen.contains("Track the work"), "{screen}");
        assert!(screen.contains("Fifth step"), "{screen}");
        assert!(!screen.contains("Hidden sixth step"), "{screen}");
        harness.click_on("Show details");
        harness.pump_until_screen_shows("Hidden sixth step");
        harness.click_on("Hide details");
        harness.pump_until("the plan to collapse", |h| {
            !h.screen().contains("Hidden sixth step")
        });
    }
}
