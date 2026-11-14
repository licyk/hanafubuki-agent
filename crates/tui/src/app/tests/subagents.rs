//! `/subagents` and a subagent's approval end to end over the standalone
//! Agent: the parent's scripted model spawns a subagent and waits for it;
//! the status feed lists it and the interface shows it beside its parent;
//! in Manual mode the approval it waits on is shown in the parent,
//! labelled with its title, and answered on it.

use std::time::Duration;

use hanafubuki_agent::test_support::{assistant_events, tool_calls_events};
use hanafubuki_agent::{AgentBackgroundEvent, AgentEventKind, AgentSessionId, AgentSessionOrigin};
use ratatui::crossterm::event::KeyCode;

use super::{command_arguments, Harness};
use crate::app::WorkspaceEvent;
use crate::app_event::AppEvent;
use crate::transcript_mode::TranscriptMode;

/// The parent's step that spawns `counter` with `task` and waits for it.
pub(super) fn spawn_and_wait(task: &str) -> Vec<u8> {
    let spawn = serde_json::json!({
        "task_name": "counter",
        "message": task,
        "fork_turns": "none",
    })
    .to_string();
    tool_calls_events(&[
        ("call-spawn", "spawn_agent", &spawn),
        ("call-wait", "wait_agent", r#"{"timeout_ms":30000}"#),
    ])
}

impl Harness {
    /// The children of `parent`, as the Agent lists them.
    pub(super) fn subagents_of(&self, parent: AgentSessionId) -> Vec<AgentSessionId> {
        let agent = self.runtime.slot().agent().unwrap();
        let reader = agent.clone();
        agent
            .runtime()
            .block_on(async move { reader.subagents(parent).await })
            .unwrap()
            .unwrap()
            .into_iter()
            .map(|status| status.session_id)
            .collect()
    }
}

#[test]
fn subagents_lists_them_and_shows_one_beside_its_parent() {
    let mut harness = Harness::start(vec![
        spawn_and_wait("Count the files."),
        assistant_events("Twelve files."),
        assistant_events("There are twelve."),
    ]);
    let parent = harness.answered("Delegate the count.", "There are twelve.");
    let [child] = harness.subagents_of(parent)[..] else {
        panic!("one subagent");
    };

    harness.type_and_submit("/subagents");
    harness.pump_until_screen_shows("Enter shows one in place of this conversation");
    harness.pump_until("the status cell", |harness| {
        harness
            .transcript()
            .contains("• counter  idle · fixture-model · default")
    });
    assert!(harness.transcript().contains("Twelve files."));

    harness.key(KeyCode::Enter);
    harness.pump_until_screen_shows("Subagent counter · from Delegate the count.");
    assert_eq!(harness.app.bridge.session_id(), Some(child));
    harness.pump_until("the subagent's transcript", |harness| {
        harness.transcript().contains("Twelve files.")
    });
    let shown = harness.transcript();
    assert!(shown.contains("Count the files."), "{shown}");
    assert!(!shown.contains("There are twelve."), "{shown}");
    let agent = harness.runtime.slot().agent().unwrap();
    assert_eq!(
        agent.snapshot(child).unwrap().origin,
        Some(AgentSessionOrigin::Subagent)
    );

    harness.key(KeyCode::Esc);
    harness.pump_until_screen_shows("Main conversation");
    assert_eq!(harness.app.bridge.session_id(), Some(parent));
}

#[test]
fn a_subagents_approval_is_shown_in_the_parent_and_answered_on_the_subagent() {
    for mode in [TranscriptMode::Terminal, TranscriptMode::Owned] {
        child_approval(mode);
    }
}

fn child_approval(mode: TranscriptMode) {
    let mut harness = Harness::start_with(
        vec![
            spawn_and_wait("Run the command."),
            tool_calls_events(&[(
                "call-child",
                "exec_command",
                &command_arguments("echo child-output"),
            )]),
            assistant_events("The command printed child-output."),
            assistant_events("The subagent ran it."),
        ],
        true,
        mode,
    );
    harness.type_and_submit("Delegate the command.");
    // The subagent runs in Manual mode, as its parent: its call waits.
    // The shell's name is the platform's: Posix, or PowerShell on Windows.
    let approval = harness.pump_until_screen_shows("Subagent counter: Run");
    // The summary names the temporary directory, so where it wraps varies.
    let unwrapped = approval.split_whitespace().collect::<Vec<_>>().join(" ");
    assert!(unwrapped.contains("echo child-output"), "{approval}");
    assert!(approval.contains("Yes, proceed"), "{approval}");
    let parent = harness.app.bridge.session_id().unwrap();
    let child = harness.asking_subagent().unwrap();
    let agent = harness.runtime.slot().agent().unwrap();
    let asked = agent
        .events_since(child, None)
        .unwrap()
        .events
        .into_iter()
        .find(|event| matches!(event.kind, AgentEventKind::ApprovalRequested { .. }))
        .unwrap();
    harness.key(KeyCode::Char('y'));
    harness.pump_until_screen_shows("The subagent ran it.");
    harness.pump_until_turn_ends();
    let requests = harness.server.requests();
    assert_eq!(requests.len(), 4);
    let answered = String::from_utf8(requests[2].body.clone()).unwrap();
    assert!(answered.contains("child-output"), "{answered}");

    // The initial journal read may arrive after the live resolution.
    let stale = AppEvent::AgentBackground(AgentBackgroundEvent::Child {
        root: parent,
        session_id: child,
        event: Box::new(asked),
    });
    harness.app.handle_event(&mut harness.tui, stale);
    harness.app.draw(&mut harness.tui).unwrap();
    assert!(!harness.screen().contains("Yes, proceed"));
}

#[test]
fn a_running_subagent_shown_follows_its_turn_and_takes_its_approval() {
    let mut harness = Harness::start_timed(
        vec![
            (spawn_and_wait("Run the command."), Duration::ZERO),
            (
                tool_calls_events(&[(
                    "call-child",
                    "exec_command",
                    &command_arguments("echo child-output"),
                )]),
                Duration::from_secs(2),
            ),
            (
                assistant_events("The command printed child-output."),
                Duration::ZERO,
            ),
            (assistant_events("The subagent ran it."), Duration::ZERO),
        ],
        true,
        TranscriptMode::Terminal,
    );
    harness.type_and_submit("Delegate the command.");
    harness.pump_until("the subagent to start", |harness| {
        harness
            .app
            .bridge
            .session_id()
            .is_some_and(|parent| !harness.subagents_of(parent).is_empty())
    });
    harness.type_and_submit("/subagents");
    harness.pump_until_screen_shows("running");
    harness.key(KeyCode::Enter);
    harness.pump_until_screen_shows("Subagent counter");
    // Its call arrives once shown: the approval is asked here.
    harness.pump_until_screen_shows("Yes, proceed");
    assert!(!harness.screen().contains("Subagent counter: Run"));
    harness.key(KeyCode::Char('y'));
    harness.pump_until_screen_shows("The command printed child-output.");
    harness.pump_until_turn_ends();
    harness.key(KeyCode::Esc);
    harness.pump_until_screen_shows("The subagent ran it.");
}

/// A subagent shown once its call already waits on the user (the parent's
/// list was slower than the subagent) shows that approval too.
#[test]
fn a_subagent_shown_after_it_asked_shows_the_approval_it_waits_on() {
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
    harness.pump_until("the subagent to ask", |harness| {
        harness.asking_subagent().is_some()
    });
    let child = harness.asking_subagent().unwrap();

    let event = AppEvent::Workspace(WorkspaceEvent::OpenChild(child));
    assert!(harness.app.handle_event(&mut harness.tui, event));
    harness.pump_until("the subagent's approval", |harness| {
        let screen = harness.screen();
        screen.contains("Subagent counter ·")
            && screen.contains("Yes, proceed")
            && !screen.contains("Subagent counter: Run")
    });
    harness.key(KeyCode::Char('y'));
    harness.pump_until_screen_shows("The command printed child-output.");
    harness.pump_until_turn_ends();
    harness.key(KeyCode::Esc);
    harness.pump_until_screen_shows("The subagent ran it.");
}

impl Harness {
    /// The session's subagent whose turn has asked for an approval.
    fn asking_subagent(&self) -> Option<AgentSessionId> {
        let parent = self.app.bridge.session_id()?;
        let agent = self.runtime.slot().agent().unwrap();
        self.subagents_of(parent).into_iter().find(|child| {
            agent
                .events_since(*child, None)
                .unwrap()
                .events
                .iter()
                .any(|event| matches!(event.kind, AgentEventKind::ApprovalRequested { .. }))
        })
    }
}
