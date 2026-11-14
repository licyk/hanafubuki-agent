//! Background terminals wake their session, end to end over real commands
//! on the fake model server (POSIX shells, and PowerShell on Windows): a
//! command still running when its turn ends is waited on, and its end
//! wakes the session once, whose woken turn reads how it ended and its last
//! output; a session that keeps working reads the end at a later step of
//! its turn, and is not woken; a poll that sees the end tells the model,
//! which is not told again. Each command waits on a file the test (or
//! another command) creates, never on time.

mod close;

use std::path::Path;

use hanafubuki_agent_kernel::{AgentEvent, AgentEventSink, AgentTurnAccepted, AgentWakePolicy};
use serde_json::Value;

use super::*;
use crate::{AgentConfig, AgentTurnRequest, AgentWakeState};

/// A command that waits until `file` exists, then prints `waited` and
/// exits with code 3.
fn waits_for(file: &Path) -> String {
    let file = file.display();
    #[cfg(not(windows))]
    return format!("while [ ! -e '{file}' ]; do sleep 0.1; done; echo waited; exit 3");
    #[cfg(windows)]
    return format!(
        "while (-not (Test-Path -LiteralPath '{file}')) {{ Start-Sleep -Milliseconds 100 }}; Write-Output waited; exit 3"
    );
}

/// A command that creates `file`.
fn creates(file: &Path) -> String {
    let file = file.display();
    #[cfg(not(windows))]
    return format!(": > '{file}'");
    #[cfg(windows)]
    return format!("New-Item -ItemType File -Path '{file}' | Out-Null");
}

/// `exec_command`'s arguments for `cmd`, yielding after `yield_time_ms`.
fn exec_arguments(cmd: &str, yield_time_ms: u64) -> String {
    json!({
        "cmd": cmd,
        "shell": null,
        "login": null,
        "tty": null,
        "yield_time_ms": yield_time_ms,
        "max_output_bytes": 32_768,
        "workdir": null
    })
    .to_string()
}

struct Discard;

impl AgentEventSink for Discard {
    fn emit(&self, _event: AgentEvent) {}
}

/// An Agent on `server` telling the returned recorder whom to wake, and a
/// session of it whose commands run without approval or sandbox.
async fn waking(
    name: &str,
    server: &FakeServer,
) -> (
    AgentApplicationService,
    TestPaths,
    AgentSessionId,
    RecordedBackgroundEvents,
) {
    let paths = test_paths(name);
    let config = AgentConfig::new(&paths.config_dir);
    configure_test_provider(&config, &server.base_url, None);
    config.set_plugin_disabled("titles", true).unwrap();
    let woken = RecordedBackgroundEvents::default();
    let service = reopen_service_waking(config, test_host(&paths), &paths, Arc::new(woken.clone()));
    service.start_plugins().await;
    let session_id = service
        .create_session(AgentSessionMode::Default, None)
        .await
        .unwrap()
        .id;
    service
        .set_session_permission_mode(
            session_id,
            hanafubuki_agent_approval::AgentPermissionMode::FullAccess,
        )
        .unwrap();
    service
        .set_session_sandbox_mode(session_id, AgentSandboxMode::FullAccess)
        .unwrap();
    (service, paths, session_id, woken)
}

async fn person(
    service: &AgentApplicationService,
    session_id: AgentSessionId,
    text: &str,
) -> AgentTurnAccepted {
    service
        .submit_turn_with_sink(
            session_id,
            AgentTurnRequest {
                text: text.to_string(),
                images: Vec::new(),
                skills: Vec::new(),
                host: Default::default(),
            },
            Arc::new(Discard),
        )
        .await
        .unwrap()
}

async fn wait_until(what: &str, mut done: impl FnMut() -> bool) {
    let deadline = Instant::now() + TEST_WAIT;
    while !done() {
        assert!(Instant::now() < deadline, "timed out waiting until {what}");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

async fn ended(service: &AgentApplicationService, turn: AgentTurnAccepted) {
    wait_until("the turn ends", || {
        service
            .events_since(turn.session_id, None)
            .unwrap()
            .events
            .iter()
            .any(|event| event.turn_id == Some(turn.turn_id) && event.kind.is_terminal())
    })
    .await;
}

/// Waits until the dispatcher looked at every signal sent so far: it takes
/// the sessions signalled together in the order of their ids, so a session
/// created and woken now is looked at after the earlier ones.
async fn settled(service: &AgentApplicationService, woken: &RecordedBackgroundEvents) {
    let probe = service
        .create_session(AgentSessionMode::Default, None)
        .await
        .unwrap()
        .id;
    set_wake_policy(service, probe, AgentWakePolicy::On)
        .await
        .unwrap();
    queue_wake_event(
        service,
        probe,
        crate::AgentWakeSource::Subagent {
            session_id: probe,
            name: "probe".to_string(),
            event: crate::AgentSubagentWake::Message {
                text: "probe".to_string(),
            },
        },
    )
    .await
    .unwrap();
    wait_until("the dispatcher reaches the probe", || {
        woken.woken(probe) == 1
    })
    .await;
}

fn body(server: &FakeServer, index: usize) -> String {
    String::from_utf8(server.requests()[index].body.clone()).unwrap()
}

/// The result of call `call_id`, as request `index` carries it.
fn call_output(server: &FakeServer, index: usize, call_id: &str) -> Value {
    let request: Value = serde_json::from_str(&body(server, index)).unwrap();
    let output = request["input"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["type"] == "function_call_output" && item["call_id"] == call_id)
        .unwrap_or_else(|| panic!("the output of {call_id}: {request}"))["output"]
        .as_str()
        .unwrap()
        .to_string();
    serde_json::from_str(&output).unwrap()
}

/// The `session_id` the result of call `call_id` gave, in request `index`.
fn started_session(server: &FakeServer, index: usize, call_id: &str) -> u64 {
    let output = call_output(server, index, call_id);
    output["sessionId"]
        .as_u64()
        .unwrap_or_else(|| panic!("a running session: {output}"))
}

fn finish(service: AgentApplicationService, paths: TestPaths) {
    let diagnostics = service.shutdown(Instant::now() + TEST_SHUTDOWN_DEADLINE);
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    let _ = std::fs::remove_dir_all(paths.app_data_dir);
}

/// The round trip: the model starts a command and ends its turn while it
/// runs; the session waits on it, told so; its end wakes the session once,
/// and the woken turn reads how it ended, its command and its last output,
/// and answers.
#[tokio::test(flavor = "multi_thread")]
async fn a_command_that_ends_after_its_turn_wakes_the_session_once() {
    let go = std::env::temp_dir().join(format!("terminal-wake-go-{}", std::process::id()));
    let _ = std::fs::remove_file(&go);
    let wait = exec_arguments(&waits_for(&go), 500);
    let server = FakeServer::start(vec![
        (
            tool_calls_events(&[("call-wait", "exec_command", wait.as_str())]),
            Duration::ZERO,
        ),
        (
            assistant_events("It runs in the background."),
            Duration::ZERO,
        ),
        (assistant_events("It exited with code 3."), Duration::ZERO),
    ])
    .await;
    let (service, paths, session_id, woken) = waking("terminal-wake-round-trip", &server).await;

    let first = person(&service, session_id, "Run it in the background.").await;
    ended(&service, first).await;
    let guided = body(&server, 0);
    assert!(
        guided.contains("## Background terminals")
            && guided.contains("times out (after 10 minutes)"),
        "{guided}"
    );
    let terminal = started_session(&server, 1, "call-wait");
    let status = service.background_status(session_id).unwrap();
    assert_eq!(
        (
            status.work.terminals,
            status.work.agents,
            status.pending_wake_events,
            status.wake
        ),
        (1, 0, 0, AgentWakeState::On)
    );
    settled(&service, &woken).await;
    assert_eq!(woken.woken(session_id), 0, "nothing ended yet");
    wait_until("the terminal start changes the tree", || {
        woken.changed(session_id) > 0
    })
    .await;
    let changed = woken.changed(session_id);
    let tasks = service.background_tasks(session_id).await.unwrap();
    assert_eq!(tasks.terminals.len(), 1);
    assert_eq!(tasks.terminals[0].owner, session_id);
    assert_eq!(tasks.terminals[0].terminal.execution_id.value(), terminal);

    std::fs::write(&go, b"").unwrap();
    wait_until("the end wakes the session", || woken.woken(session_id) == 1).await;
    wait_until("the terminal end changes the tree", || {
        woken.changed(session_id) > changed
    })
    .await;
    let tasks = service.background_tasks(session_id).await.unwrap();
    assert_eq!(tasks.terminals[0].terminal.exit_code, Some(3));
    assert!(tasks.terminals[0]
        .terminal
        .recent_output
        .iter()
        .any(|line| line.contains("waited")));
    let status = service.background_status(session_id).unwrap();
    assert_eq!((status.work.terminals, status.pending_wake_events), (0, 1));
    let accepted = service
        .submit_wake_turn_with_sink(session_id, Arc::new(Discard))
        .await
        .unwrap()
        .expect("the end is queued");
    ended(&service, accepted.turn).await;

    let woken_request = body(&server, 2);
    assert!(
        woken_request.contains(&format!(
            "Terminal session {terminal} exited with code 3 after"
        )) && woken_request.contains("terminal-wake-go-")
            && woken_request.contains("waited\\n</output>"),
        "{woken_request}"
    );
    assert_eq!(server.requests().len(), 3);
    settled(&service, &woken).await;
    assert_eq!(woken.woken(session_id), 1, "woken once");
    assert_eq!(
        service
            .background_status(session_id)
            .unwrap()
            .pending_wake_events,
        0
    );
    let _ = std::fs::remove_file(&go);
    finish(service, paths);
}

/// A session that keeps working reads its terminal's end at a later step
/// of the same turn: the turn goes on until it has read it, and nothing
/// wakes the session afterwards.
#[tokio::test(flavor = "multi_thread")]
async fn a_session_that_keeps_working_reads_the_end_at_a_later_step() {
    let go = std::env::temp_dir().join(format!("terminal-step-go-{}", std::process::id()));
    let _ = std::fs::remove_file(&go);
    let wait = exec_arguments(&waits_for(&go), 500);
    let release = exec_arguments(&creates(&go), 30_000);
    let server = FakeServer::start(vec![
        (
            tool_calls_events(&[("call-wait", "exec_command", wait.as_str())]),
            Duration::ZERO,
        ),
        (
            tool_calls_events(&[("call-go", "exec_command", release.as_str())]),
            Duration::ZERO,
        ),
        // Held until the first command's end is applied, which may come
        // after this step read the queue: the turn then takes one more.
        (
            [HOLD.as_bytes(), &assistant_events("Both done.")].concat(),
            Duration::ZERO,
        ),
        (assistant_events("Both done."), Duration::ZERO),
    ])
    .await;
    let (service, paths, session_id, woken) = waking("terminal-wake-step", &server).await;

    let turn = person(&service, session_id, "Wait, then release it.").await;
    wait_until("the third step is asked for", || {
        server.requests().len() >= 3
    })
    .await;
    let terminal = started_session(&server, 1, "call-wait");
    wait_until("the first command's end is applied", || {
        service
            .background_status(session_id)
            .unwrap()
            .work
            .terminals
            == 0
    })
    .await;
    server.hold.notify_one();
    ended(&service, turn).await;

    let told = (0..server.requests().len())
        .filter(|index| {
            body(&server, *index)
                .contains(&format!("Terminal session {terminal} exited with code 3"))
        })
        .count();
    assert!(told >= 1, "a later step reads the end");
    let first_told = (0..server.requests().len())
        .find(|index| {
            body(&server, *index).contains(&format!("Terminal session {terminal} exited"))
        })
        .unwrap();
    assert!(first_told >= 2, "after the command that released it");
    settled(&service, &woken).await;
    assert_eq!(woken.woken(session_id), 0, "a running turn reads it");
    assert_eq!(
        service
            .background_status(session_id)
            .unwrap()
            .pending_wake_events,
        0
    );
    let _ = std::fs::remove_file(&go);
    finish(service, paths);
}

/// A poll that is waiting when its terminal ends returns the end, which is
/// how the model learns of it: no update tells it again, and the session is
/// not woken.
#[tokio::test(flavor = "multi_thread")]
async fn a_poll_that_sees_the_end_is_not_told_it_again() {
    let go = std::env::temp_dir().join(format!("terminal-poll-go-{}", std::process::id()));
    let _ = std::fs::remove_file(&go);
    // The fresh Agent's first execution is session 1.
    let poll = json!({
        "session_id": 1,
        "chars": "",
        "yield_time_ms": 30_000,
        "max_output_bytes": 32_768
    })
    .to_string();
    let wait = exec_arguments(&waits_for(&go), 500);
    let server = FakeServer::start(vec![
        (
            tool_calls_events(&[("call-wait", "exec_command", wait.as_str())]),
            Duration::ZERO,
        ),
        (
            tool_calls_events(&[("call-poll", "write_stdin", poll.as_str())]),
            Duration::ZERO,
        ),
        (assistant_events("It exited with code 3."), Duration::ZERO),
    ])
    .await;
    let (service, paths, session_id, woken) = waking("terminal-wake-poll", &server).await;

    let turn = person(&service, session_id, "Run it and wait for it.").await;
    wait_until("the poll waits", || {
        service
            .events_since(session_id, None)
            .unwrap()
            .events
            .iter()
            .any(|event| {
                matches!(&event.kind, AgentEventKind::ToolStarted { call_id, .. } if call_id == "call-poll")
            })
    })
    .await;
    assert_eq!(started_session(&server, 1, "call-wait"), 1);
    std::fs::write(&go, b"").unwrap();
    ended(&service, turn).await;

    let requests = server.requests();
    assert_eq!(requests.len(), 3);
    let polled = call_output(&server, 2, "call-poll");
    assert_eq!(polled["exitCode"], 3, "{polled}");
    let after_poll = body(&server, 2);
    assert!(!after_poll.contains("Terminal session 1"), "{after_poll}");
    settled(&service, &woken).await;
    assert_eq!(woken.woken(session_id), 0);
    let status = service.background_status(session_id).unwrap();
    assert_eq!((status.work.terminals, status.pending_wake_events), (0, 0));
    let _ = std::fs::remove_file(&go);
    finish(service, paths);
}
