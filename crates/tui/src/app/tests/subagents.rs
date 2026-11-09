//! `/subagents` and a subagent's approval end to end over the standalone
//! Agent: the parent's scripted model spawns a subagent and waits for it;
//! the status feed lists it and the interface shows it beside its parent;
//! in Manual mode the approval it waits on is shown in the parent,
//! labelled with its title, and answered on it.

use std::time::Duration;

use hanafubuki_agent::test_support::{assistant_events, tool_calls_events};
use hanafubuki_agent::{AgentSessionId, AgentSessionOrigin};
use ratatui::crossterm::event::KeyCode;

use super::{command_arguments, Harness};
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
        harness.transcript().contains("• counter  idle")
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
    // The subagent runs in Manual mode, as its parent: its call waits.
    // The shell's name is the platform's: Posix, or PowerShell on Windows.
    let approval = harness.pump_until_screen_shows("Subagent counter: Run");
    assert!(approval.contains("echo child-output"), "{approval}");
    assert!(approval.contains("Yes, proceed"), "{approval}");
    harness.key(KeyCode::Char('y'));
    harness.pump_until_screen_shows("The subagent ran it.");
    harness.pump_until_turn_ends();
    let requests = harness.server.requests();
    assert_eq!(requests.len(), 4);
    let answered = String::from_utf8(requests[2].body.clone()).unwrap();
    assert!(answered.contains("child-output"), "{answered}");
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
