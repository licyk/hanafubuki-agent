use std::time::Duration;

use hanafubuki_agent::test_support::{assistant_events, queue_wake_event, tool_calls_events, HOLD};
use hanafubuki_agent::{AgentSessionId, AgentSubagentWake, AgentWakeSource, AgentWakeState};
use ratatui::crossterm::event::{KeyCode, KeyModifiers};

use super::{subagents::spawn_and_wait, Harness};
use crate::transcript_mode::TranscriptMode;

#[test]
fn a_forked_agents_task_skips_inherited_messages_across_transcript_pages() {
    let assigned = "Inspect the child-specific API.";
    let spawn =
        serde_json::json!({"task_name":"forked_worker","message":assigned,"fork_turns":"all"})
            .to_string();
    let mut responses = (0..33)
        .map(|index| assistant_events(&format!("Inherited answer {index}.")))
        .collect::<Vec<_>>();
    responses.extend([
        tool_calls_events(&[
            ("call-spawn", "spawn_agent", &spawn),
            ("call-wait", "wait_agent", r#"{"timeout_ms":30000}"#),
        ]),
        assistant_events("The API is consistent."),
        assistant_events("The inherited worker finished."),
    ]);
    let mut harness = Harness::owned(responses);
    for index in 0..33 {
        harness.answered(
            &format!("Inherited request {index}"),
            &format!("Inherited answer {index}."),
        );
    }
    harness.answered(
        "Delegate with inherited history",
        "The inherited worker finished.",
    );
    harness.type_and_submit("/tasks");
    let screen = harness.pump_until_screen_shows(assigned);
    assert!(!screen.contains("Inherited request 0"), "{screen}");
}

#[test]
fn task_manager_closes_a_terminal_and_wakes_its_owner() {
    for mode in [TranscriptMode::Terminal, TranscriptMode::Owned] {
        let command = if cfg!(windows) {
            "Start-Sleep -Seconds 60"
        } else {
            "sleep 60"
        };
        let arguments = serde_json::json!({"cmd":command,"yield_time_ms":500}).to_string();
        let mut harness = Harness::start_with(
            vec![
                assistant_events("Ready."),
                tool_calls_events(&[("call-background", "exec_command", &arguments)]),
                assistant_events("The terminal is running."),
                assistant_events("The terminal was stopped."),
            ],
            true,
            mode,
        );
        let root = harness.answered("Start", "Ready.");
        let agent = harness.runtime.slot().agent().unwrap();
        agent
            .set_session_permission_mode(root, hanafubuki_agent::AgentPermissionMode::FullAccess)
            .unwrap();
        agent
            .set_session_sandbox_mode(root, hanafubuki_agent::AgentSandboxMode::FullAccess)
            .unwrap();
        harness.answered("Start the background command", "The terminal is running.");
        harness.type_and_submit("/tasks");
        harness.pump_until_screen_shows("wake: on");
        harness.key(KeyCode::Tab);
        harness.pump_until_screen_shows(command);
        harness.key(KeyCode::Char('x'));
        harness.pump_until_screen_shows("Close terminal #");
        harness.key(KeyCode::Char('y'));
        harness.pump_until("the terminal is disposed", |_| {
            agent.list_background_terminals(root).unwrap().is_empty()
        });
        harness.key(KeyCode::Esc);
        harness.pump_until("the terminal's wake answer", |h| {
            h.transcript().contains("The terminal was stopped.")
        });
    }
}

#[test]
fn task_manager_closes_an_agent_and_its_parent_receives_the_update() {
    for mode in [TranscriptMode::Terminal, TranscriptMode::Owned] {
        let mut harness = Harness::start_with(
            vec![
                spawn_and_wait("Count the files."),
                assistant_events("Twelve files."),
                assistant_events("There are twelve."),
                assistant_events("The worker was closed."),
            ],
            true,
            mode,
        );
        let root = harness.answered("Delegate the count.", "There are twelve.");
        harness.key_with(KeyCode::Char('x'), KeyModifiers::CONTROL);
        harness.key(KeyCode::Char('t'));
        harness.pump_until_screen_shows("counter");
        harness.key(KeyCode::Char('x'));
        harness.pump_until_screen_shows("Close counter and its 0 sub-agents?");
        harness.key(KeyCode::Enter);
        harness.app.draw(&mut harness.tui).unwrap();
        assert!(!harness.screen().contains("Confirm"));
        harness.key(KeyCode::Char('x'));
        harness.key(KeyCode::Char('y'));
        harness.pump_until_screen_shows("closed");
        harness.key(KeyCode::Esc);
        harness.pump_until("the parent's wake answer", |h| {
            h.transcript().contains("The worker was closed.")
        });
        assert_eq!(harness.app.bridge.session_id(), Some(root));
        assert!(harness.transcript().contains("Background updates"));
    }
}

#[test]
fn task_manager_resumes_a_paused_wake_in_both_transcript_modes() {
    for mode in [TranscriptMode::Terminal, TranscriptMode::Owned] {
        let mut harness = Harness::start_timed(
            vec![
                (assistant_events("Ready."), Duration::ZERO),
                (
                    [
                        HOLD.as_bytes(),
                        &assistant_events("This answer was interrupted."),
                    ]
                    .concat(),
                    Duration::ZERO,
                ),
                (
                    assistant_events("The queued update was read."),
                    Duration::ZERO,
                ),
            ],
            true,
            mode,
        );
        let root = harness.answered("Start", "Ready.");
        harness.type_and_submit("Keep working");
        harness.pump_until("the held model call is shown as running", |h| {
            h.server.requests().len() >= 2 && h.app.chat_widget.is_task_running()
        });
        harness.key(KeyCode::Esc);
        harness.pump_until_turn_ends();
        let agent = harness.runtime.slot().agent().unwrap();
        assert!(matches!(
            agent.background_status(root).unwrap().wake,
            AgentWakeState::Paused { .. }
        ));
        harness.server.hold.notify_one();
        let worker = agent.clone();
        agent
            .runtime()
            .block_on(async move {
                queue_wake_event(
                    &worker,
                    root,
                    AgentWakeSource::Subagent {
                        session_id: AgentSessionId::new(999),
                        name: "reported task".into(),
                        event: AgentSubagentWake::Message {
                            text: "Ready to resume.".into(),
                        },
                    },
                )
                .await
            })
            .unwrap()
            .unwrap();
        harness.type_and_submit("/tasks");
        harness.pump_until_screen_shows("wake: paused");
        harness.key(KeyCode::Char('r'));
        harness.pump_until_screen_shows("wake: on");
        harness.key(KeyCode::Esc);
        harness.pump_until("the resumed answer", |h| {
            h.transcript().contains("The queued update was read.")
        });
        assert_eq!(
            agent.background_status(root).unwrap().pending_wake_events,
            0
        );
    }
}
